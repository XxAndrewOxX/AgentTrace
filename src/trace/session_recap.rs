use crate::llm::trace_insights::TraceInsightsFacade;
use crate::running_summary::{load_all_events, SummaryEvent};
use crate::session::AgentSession;
use anyhow::Result;
use std::path::{Path, PathBuf};

const SESSION_RECAPS_DIR: &str = ".agent-trace/session_recaps";

pub fn recap_dir(store_root: &Path) -> PathBuf {
    store_root.join(SESSION_RECAPS_DIR)
}

pub fn recap_path(store_root: &Path, session_id: &str) -> PathBuf {
    recap_dir(store_root).join(format!("{session_id}.md"))
}

pub fn load_events_for_session(store_root: &Path, session_id: &str) -> Result<Vec<SummaryEvent>> {
    Ok(load_all_events(store_root)?
        .into_iter()
        .filter(|e| e.session_id.as_deref() == Some(session_id))
        .collect())
}

pub fn synthesize_template_session_recap(
    prior: &AgentSession,
    events: &[SummaryEvent],
) -> String {
    let mut out = String::from("# Prior Session Recap\n\n");
    out.push_str(&format!(
        "*Agent: {} / {} ({})*\n",
        prior.name, prior.session_id, prior.transport
    ));
    out.push_str(&format!("*Started: {}*\n", prior.started_at));
    out.push_str("*Ended: stale lock — new session started*\n\n");

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

pub fn generate_session_recap(store_root: &Path, prior: &AgentSession) -> Result<String> {
    let events = load_events_for_session(store_root, &prior.session_id)?;
    let event_strings: Vec<String> = events
        .iter()
        .map(|e| {
            format!(
                "[{}] {} {} — {}",
                e.timestamp, e.action, e.path, e.summary
            )
        })
        .collect();

    if let Ok(api) = TraceInsightsFacade::from_store_root(store_root) {
        if !api.is_degraded() {
            match api.summarize_session(&prior.session_id, &event_strings) {
                Ok(summary) => {
                    let mut out = String::from("# Prior Session Recap\n\n");
                    out.push_str(&format!(
                        "*Agent: {} / {} ({})*\n\n",
                        prior.name, prior.session_id, prior.transport
                    ));
                    out.push_str(&summary);
                    out.push_str("\n\n## Events\n\n");
                    if events.is_empty() {
                        out.push_str("*(no recorded events)*\n");
                    } else {
                        for e in &events {
                            out.push_str(&format!(
                                "- {} {} — {}\n",
                                e.action, e.path, e.summary
                            ));
                        }
                    }
                    return Ok(out);
                }
                Err(e) => {
                    tracing::warn!("session recap synthesis failed: {e}");
                }
            }
        }
    }

    Ok(synthesize_template_session_recap(prior, &events))
}

pub fn persist_session_recap(
    store_root: &Path,
    session_id: &str,
    content: &str,
) -> Result<()> {
    let path = recap_path(store_root, session_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::util::atomic_write(&path, content)?;
    Ok(())
}

/// Generate and persist a recap for a stale session if one does not already exist.
pub fn maybe_recap_prior_session(store_root: &Path, prior: &AgentSession) -> Result<()> {
    let path = recap_path(store_root, &prior.session_id);
    if path.exists() {
        return Ok(());
    }
    let content = generate_session_recap(store_root, prior)?;
    persist_session_recap(store_root, &prior.session_id, &content)?;
    tracing::info!(
        session_id = %prior.session_id,
        agent = %prior.name,
        "persisted session recap for stale session"
    );
    Ok(())
}

/// Load the most recent prior-session recap (excludes the current active session).
pub fn load_prior_session_recap(store_root: &Path) -> Option<String> {
    let current_id = crate::session::load_session(store_root)
        .filter(|s| !s.is_stale())
        .map(|s| s.session_id);

    let dir = recap_dir(store_root);
    if !dir.exists() {
        return None;
    }

    let mut entries: Vec<(PathBuf, std::time::SystemTime)> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "md")
                .unwrap_or(false)
        })
        .filter_map(|e| {
            let stem = e.path().file_stem()?.to_string_lossy().to_string();
            if current_id.as_deref() == Some(stem.as_str()) {
                return None;
            }
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((e.path(), modified))
        })
        .collect();

    entries.sort_by_key(|(_, m)| *m);
    let (path, _) = entries.last()?;
    std::fs::read_to_string(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::running_summary::append_event;
    use crate::session::{self, LOCK_FILE};
    use tempfile::TempDir;

    fn stale_lock(root: &Path, name: &str, session_id: &str) {
        std::fs::create_dir_all(root.join(".agent-trace/locks")).unwrap();
        std::fs::write(
            root.join(LOCK_FILE),
            format!(
                "[agent]\nname=\"{name}\"\nsession_id=\"{session_id}\"\ntransport=\"cli\"\n\
                 started_at=\"2020-01-01T00:00:00Z\"\nlast_heartbeat=\"2020-01-01T00:00:00Z\"\n"
            ),
        )
        .unwrap();
    }

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
            lines_added: 1,
            lines_removed: 0,
        }
    }

    #[test]
    fn template_recap_lists_session_events() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        let prior = AgentSession {
            name: "bot".into(),
            session_id: "20260605-120000".into(),
            transport: "mcp".into(),
            started_at: "2026-06-05T12:00:00Z".into(),
            last_heartbeat: "2020-01-01T00:00:00Z".into(),
        };
        append_event(root, sample_event("20260605-120000", "plan.md", "phase 1 done")).unwrap();
        append_event(root, sample_event("20260605-120000", "notes.md", "follow-up")).unwrap();
        append_event(root, sample_event("other-session", "plan.md", "ignored")).unwrap();

        let events = load_events_for_session(root, "20260605-120000").unwrap();
        assert_eq!(events.len(), 2);
        let recap = synthesize_template_session_recap(&prior, &events);
        assert!(recap.contains("# Prior Session Recap"));
        assert!(recap.contains("phase 1 done"));
        assert!(recap.contains("follow-up"));
        assert!(!recap.contains("ignored"));
    }

    #[test]
    fn maybe_recap_persists_once() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        let prior = AgentSession {
            name: "bot".into(),
            session_id: "20260605-120000".into(),
            transport: "cli".into(),
            started_at: "2026-06-05T12:00:00Z".into(),
            last_heartbeat: "2020-01-01T00:00:00Z".into(),
        };
        append_event(root, sample_event("20260605-120000", "plan.md", "work")).unwrap();

        maybe_recap_prior_session(root, &prior).unwrap();
        let path = recap_path(root, "20260605-120000");
        assert!(path.exists());
        let first = std::fs::read_to_string(&path).unwrap();

        maybe_recap_prior_session(root, &prior).unwrap();
        let second = std::fs::read_to_string(&path).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn start_session_recaps_stale_lock() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        stale_lock(root, "bot", "old-session");
        append_event(root, sample_event("old-session", "plan.md", "prior work")).unwrap();

        let new_sess = session::start_session(root, "bot", "cli").unwrap();
        assert_ne!(new_sess.session_id, "old-session");
        assert!(recap_path(root, "old-session").exists());
        let recap = std::fs::read_to_string(recap_path(root, "old-session")).unwrap();
        assert!(recap.contains("prior work"));
    }

    #[test]
    fn load_prior_recap_excludes_current_session() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(recap_dir(root)).unwrap();
        persist_session_recap(root, "old-session", "# Prior Session Recap\n\nold work\n").unwrap();
        let current = session::start_session(root, "bot", "cli").unwrap();
        persist_session_recap(
            root,
            &current.session_id,
            "# Prior Session Recap\n\ncurrent session recap\n",
        )
        .unwrap();

        let recap = load_prior_session_recap(root).unwrap();
        assert!(recap.contains("old work"));
        assert!(!recap.contains("current session recap"));
    }
}
