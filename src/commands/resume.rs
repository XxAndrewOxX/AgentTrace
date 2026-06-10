use crate::observability::CliOutput;
use crate::running_summary;
use crate::session;
use crate::session_recap;
use anyhow::Result;
use clap::Subcommand;
use std::path::Path;

#[derive(Subcommand, Debug)]
pub enum ResumeCmd {
    /// Print running_summary.md and session info.
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
            if let Some(sess) = session::load_session(store_root) {
                let stale = if sess.is_stale() { " (stale)" } else { "" };
                output.line(&format!(
                    "Session: {} / {}{} (transport: {})",
                    sess.name, sess.session_id, stale, sess.transport
                ))?;
            } else {
                output.line("No active agent session.")?;
            }
            let summary_path = store_root.join("running_summary.md");
            if summary_path.exists() {
                let content = std::fs::read_to_string(&summary_path)?;
                output.raw_stdout(&content)?;
            } else {
                output.line("No running_summary.md yet. Write a document to generate one.")?;
            }
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
                        "  [{}] {} {} — {} ({})",
                        e.timestamp, e.action, e.path, e.summary, e.source
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
    use crate::config::StoreInfo;
    use crate::git_store::GitStore;
    use crate::manifest::Manifest;
    use crate::observability::NoopOutput;
    use crate::running_summary::{
        append_event, assemble_resume_context, write_running_summary, SummaryEvent,
    };
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
                lines_added: 1,
                lines_removed: 0,
            },
        )
        .unwrap();
        run(root, ResumeCmd::Events { limit: 10 }, &NoopOutput).unwrap();
    }

    #[test]
    fn assemble_resume_context_includes_summary() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        let git = GitStore::init(root).unwrap();
        let info = StoreInfo::new("test".into());
        let mut manifest = Manifest::create_empty(info, root).unwrap();
        write_running_summary(
            root,
            "# Running Summary\n\n## Resume Here\n\nDo the thing\n",
            &git,
            &mut manifest,
        )
        .unwrap();
        let text = assemble_resume_context(root, &Actor::User, false, 5).unwrap();
        assert!(text.contains("Running Summary"));
        assert!(text.contains("Resume Here"));
    }
}
