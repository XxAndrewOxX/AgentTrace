use crate::llm::Llm;
use crate::running_summary::{load_all_events, SummaryEvent};
use crate::session::AgentSession;
use anyhow::Result;
use std::path::{Path, PathBuf};

const CHECKPOINTS_DIR: &str = ".agent-trace/session_checkpoints";

pub fn checkpoint_dir(store_root: &Path) -> PathBuf {
    store_root.join(CHECKPOINTS_DIR)
}

pub fn checkpoint_path(store_root: &Path, session_id: &str) -> PathBuf {
    checkpoint_dir(store_root).join(format!("{session_id}.md"))
}

pub fn load_events_for_session(store_root: &Path, session_id: &str) -> Result<Vec<SummaryEvent>> {
    Ok(load_all_events(store_root)?
        .into_iter()
        .filter(|e| e.session_id.as_deref() == Some(session_id))
        .collect())
}

pub fn synthesize_template_session_checkpoint(
    session: &AgentSession,
    events: &[SummaryEvent],
) -> String {
    let mut out = String::from("# Current Session Checkpoint\n\n");
    out.push_str(&format!(
        "*Agent: {} / {} ({})*\n",
        session.name, session.session_id, session.transport
    ));
    out.push_str(&format!("*Started: {}*\n\n", session.started_at));

    out.push_str("## Activity Summary\n\n");
    if events.is_empty() {
        out.push_str("*(no recorded events for this session)*\n");
    } else {
        let mut paths = std::collections::HashSet::new();
        for e in events {
            paths.insert(e.path.as_str());
        }
        out.push_str(&format!(
            "{} event(s) across {} file(s).\n\n",
            events.len(),
            paths.len()
        ));
        for e in events {
            let time = e
                .timestamp
                .split('T')
                .nth(1)
                .and_then(|t| t.split('Z').next())
                .unwrap_or(&e.timestamp);
            out.push_str(&format!(
                "- [{time}] {} {} — {}\n",
                e.action, e.path, e.summary
            ));
        }
    }

    out
}

pub fn generate_session_checkpoint(store_root: &Path, session: &AgentSession) -> Result<String> {
    let events = load_events_for_session(store_root, &session.session_id)?;
    let event_strings: Vec<String> = events
        .iter()
        .map(|e| format!("[{}] {} {} — {}", e.timestamp, e.action, e.path, e.summary))
        .collect();

    if let Ok(api) = Llm::from_store_root(store_root) {
        if !api.is_degraded() {
            match api.summarize_session(&session.session_id, &event_strings) {
                Ok(summary) => {
                    let mut out = String::from("# Current Session Checkpoint\n\n");
                    out.push_str(&format!(
                        "*Agent: {} / {} ({})*\n\n",
                        session.name, session.session_id, session.transport
                    ));
                    out.push_str(&summary);
                    out.push_str("\n\n## Events\n\n");
                    if events.is_empty() {
                        out.push_str("*(no recorded events)*\n");
                    } else {
                        for e in &events {
                            out.push_str(&format!("- {} {} — {}\n", e.action, e.path, e.summary));
                        }
                    }
                    return Ok(out);
                }
                Err(e) => {
                    tracing::warn!("session checkpoint synthesis failed: {e}");
                }
            }
        }
    }

    Ok(synthesize_template_session_checkpoint(session, &events))
}

pub fn persist_checkpoint(store_root: &Path, session_id: &str, content: &str) -> Result<()> {
    let path = checkpoint_path(store_root, session_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::util::atomic_write(&path, content)?;
    Ok(())
}

pub fn load_checkpoint(store_root: &Path, session_id: &str) -> Option<String> {
    let path = checkpoint_path(store_root, session_id);
    std::fs::read_to_string(path).ok()
}

pub fn maybe_write_session_checkpoint(store_root: &Path, session_id: &str) -> Result<()> {
    let session = crate::session::load_session(store_root)
        .filter(|s| !s.is_stale() && s.session_id == session_id)
        .ok_or_else(|| anyhow::anyhow!("no active session matching {session_id}"))?;
    let content = generate_session_checkpoint(store_root, &session)?;
    persist_checkpoint(store_root, session_id, &content)?;
    tracing::info!(
        session_id = %session_id,
        agent = %session.name,
        "persisted session checkpoint"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::running_summary::append_event;
    use crate::session;
    use tempfile::TempDir;

    fn sample_event(session_id: &str, path: &str, summary: &str) -> SummaryEvent {
        SummaryEvent {
            timestamp: "2026-06-05T20:58:09Z".into(),
            session_id: Some(session_id.into()),
            agent_name: Some("bot".into()),
            actor: "agent:bot".into(),
            action: "modify".into(),
            path: path.into(),
            doc_type: "plan".into(),
            summary: summary.into(),
            source: "mcp_write".into(),
            detected_by: "mcp".into(),
            lines_added: 1,
            lines_removed: 0,
            change_kind: "modify".into(),
        }
    }

    #[test]
    fn template_checkpoint_lists_session_events() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        let session = AgentSession {
            name: "bot".into(),
            session_id: "20260605-120000".into(),
            transport: "mcp".into(),
            started_at: "2026-06-05T12:00:00Z".into(),
            last_heartbeat: "2026-06-05T12:00:00Z".into(),
        };
        append_event(
            root,
            sample_event("20260605-120000", "plan.md", "phase 1 done"),
        )
        .unwrap();
        append_event(
            root,
            sample_event("20260605-120000", "notes.md", "follow-up"),
        )
        .unwrap();
        append_event(root, sample_event("other-session", "plan.md", "ignored")).unwrap();

        let events = load_events_for_session(root, "20260605-120000").unwrap();
        let checkpoint = synthesize_template_session_checkpoint(&session, &events);
        assert!(checkpoint.contains("# Current Session Checkpoint"));
        assert!(checkpoint.contains("phase 1 done"));
        assert!(checkpoint.contains("follow-up"));
        assert!(!checkpoint.contains("ignored"));
    }

    #[test]
    fn checkpoint_persist_and_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        session::start_session(root, "bot", "cli").unwrap();
        let sess = session::load_session(root).unwrap();
        append_event(root, sample_event(&sess.session_id, "plan.md", "work")).unwrap();

        let content = generate_session_checkpoint(root, &sess).unwrap();
        persist_checkpoint(root, &sess.session_id, &content).unwrap();
        assert!(checkpoint_path(root, &sess.session_id).exists());

        let loaded = load_checkpoint(root, &sess.session_id).unwrap();
        assert_eq!(loaded, content);
    }
}
