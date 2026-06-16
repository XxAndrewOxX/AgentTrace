use crate::observability::CliOutput;
use crate::running_summary;
use crate::session_recap;
use anyhow::Result;
use clap::Subcommand;
use std::path::Path;

#[derive(Subcommand, Debug)]
pub enum ResumeCmd {
    /// Print the four-section resume briefing (same as MCP get_resume_context).
    Show,
    /// Force rebuild of running_summary.md from the event log.
    Refresh,
    /// Print recent summary events from the JSONL log.
    Events {
        /// Maximum number of events to show.
        #[arg(long, default_value = "20")]
        limit: usize,
    },
}

pub fn run(store_root: &Path, cmd: ResumeCmd, output: &dyn CliOutput) -> Result<()> {
    match cmd {
        ResumeCmd::Show => {
            if let Err(e) = session_recap::ensure_prior_session_recap(store_root) {
                tracing::warn!("prior session recap failed: {e}");
            }
            if let Err(e) = running_summary::refresh_if_stale(store_root) {
                tracing::warn!("running summary refresh before resume show failed: {e}");
            }
            let briefing = crate::briefing::assemble_resume_briefing(
                store_root,
                &crate::types::Actor::User,
                &crate::briefing::BriefingOptions::default(),
            )?;
            output.raw_stdout(&briefing)?;
        }
        ResumeCmd::Refresh => {
            if let Err(e) = session_recap::ensure_prior_session_recap(store_root) {
                tracing::warn!("prior session recap failed: {e}");
            }
            running_summary::refresh_from_path(store_root)?;
            output.line("running_summary.md refreshed.")?;
        }
        ResumeCmd::Events { limit } => {
            let events = running_summary::load_recent_events(store_root, limit)?;
            if events.is_empty() {
                output.line("No summary events recorded.")?;
            } else {
                output.line(&format!("Last {} event(s):", events.len()))?;
                for e in events {
                    output.line(&format!(
                        "  [{}] {} {} — {} (source={}, detected_by={})",
                        e.timestamp, e.action, e.path, e.summary, e.source, e.detected_by
                    ))?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::briefing::{assemble_resume_briefing, BriefingOptions};
    use crate::config::StoreInfo;
    use crate::git_store::GitStore;
    use crate::manifest::Manifest;
    use crate::observability::NoopOutput;
    use crate::running_summary::{append_event, write_running_summary, SummaryEvent};
    use crate::types::Actor;
    use chrono::Utc;
    use tempfile::TempDir;

    #[test]
    fn resume_show_prints_summary() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        let git = GitStore::init(root).unwrap();
        let info = StoreInfo::new("test".into());
        let mut manifest = Manifest::create_empty(info, root).unwrap();
        write_running_summary(
            root,
            "# Running Summary\n\ntest body\n",
            &git,
            &mut manifest,
            "template",
        )
        .unwrap();

        run(root, ResumeCmd::Show, &NoopOutput).unwrap();
    }

    #[test]
    fn resume_events_lists_jsonl() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        append_event(
            root,
            SummaryEvent {
                timestamp: Utc::now().to_rfc3339(),
                session_id: None,
                agent_name: None,
                actor: "agent:test".into(),
                action: "modify".into(),
                path: "plan.md".into(),
                doc_type: "plan".into(),
                summary: "test".into(),
                source: "cli_write".into(),
                detected_by: "cli".into(),
                lines_added: 1,
                lines_removed: 0,
                change_kind: "modify".into(),
            },
        )
        .unwrap();
        run(root, ResumeCmd::Events { limit: 10 }, &NoopOutput).unwrap();
    }

    #[test]
    fn assemble_resume_briefing_includes_sections() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        let git = GitStore::init(root).unwrap();
        let info = StoreInfo::new("test".into());
        let mut manifest = Manifest::create_empty(info, root).unwrap();
        std::fs::write(
            root.join("plan.md"),
            "# Plan\n\n## Goal\n\nShip it.\n\n- [ ] Phase 1\n",
        )
        .unwrap();
        manifest
            .register(
                &std::path::PathBuf::from("plan.md"),
                crate::types::DocType::Plan,
                "",
            )
            .unwrap();
        write_running_summary(
            root,
            "# Running Summary\n\n## Resume Here\n\nDo the thing\n",
            &git,
            &mut manifest,
            "template",
        )
        .unwrap();
        let text =
            assemble_resume_briefing(root, &Actor::User, &BriefingOptions::default()).unwrap();
        assert!(text.contains("## 1. Overall Objective"));
        assert!(text.contains("Ship it"));
        assert!(text.contains("INSTRUCTIONS"));
        assert!(!text.contains("--- Context (context.md) ---"));
    }
}
