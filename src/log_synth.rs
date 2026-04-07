use crate::git_store::{CommitInfo, GitStore};
use crate::types::{Action, Actor, DocType, DiffStats};
use anyhow::Result;
use chrono::Utc;
use std::path::{Path, PathBuf};

/// Generate a human-readable summary for an agent change (no LLM version).
pub fn summarize_change_no_llm(
    path: &Path,
    _doc_type: &DocType,
    stats: &DiffStats,
    agent_name: &str,
) -> String {
    format!(
        "Agent {} modified {}: +{} lines, -{} lines.",
        agent_name,
        path.display(),
        stats.lines_added,
        stats.lines_removed,
    )
}

/// Append a log entry to the session log file, creating it if needed.
pub fn append_agent_log(
    store_root: &Path,
    git: &GitStore,
    agent_name: &str,
    session_id: &str,
    entries: &[LogSynthEntry],
) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }

    let logs_dir = store_root.join("logs");
    std::fs::create_dir_all(&logs_dir)?;

    let log_path = logs_dir.join(format!("{}-{}.md", agent_name, session_id));
    let rel_log_path = log_path.strip_prefix(store_root).unwrap_or(&log_path).to_path_buf();

    let mut content = if log_path.exists() {
        std::fs::read_to_string(&log_path)?
    } else {
        format!(
            "# Agent Log: {} (session {})\n\n",
            agent_name, session_id
        )
    };

    for entry in entries {
        content.push_str(&format!(
            "## {} — {}\n\n{}\n\n",
            entry.timestamp.format("%Y-%m-%d %H:%M:%S UTC"),
            entry.path.display(),
            entry.summary,
        ));
    }

    std::fs::write(&log_path, &content)?;

    // Commit with system authorship.
    let info = CommitInfo {
        action: Action::Modify,
        files: vec![(rel_log_path, Action::Modify, DocType::Log)],
        actor: Actor::System,
        summary: format!("update agent log for {}", agent_name),
        agent_name: Some(agent_name.to_string()),
        session_id: Some(session_id.to_string()),
    };
    git.commit(&info)?;

    Ok(())
}

pub struct LogSynthEntry {
    pub timestamp: chrono::DateTime<Utc>,
    pub path: PathBuf,
    pub summary: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DiffStats;
    use tempfile::TempDir;
    use crate::git_store::GitStore;

    #[test]
    fn test_summarize_change_no_llm() {
        let stats = DiffStats { lines_added: 15, lines_removed: 3 };
        let summary = summarize_change_no_llm(
            &PathBuf::from("impl-plan.md"),
            &DocType::Plan,
            &stats,
            "claude-code",
        );
        assert!(summary.contains("claude-code"));
        assert!(summary.contains("+15"));
        assert!(summary.contains("-3"));
        assert!(summary.contains("impl-plan.md"));
    }

    #[test]
    fn test_append_agent_log_creates_file() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".docmgr")).unwrap();
        let git = GitStore::init(root).unwrap();

        let entries = vec![LogSynthEntry {
            timestamp: Utc::now(),
            path: PathBuf::from("prd.md"),
            summary: "Agent modified prd.md: +5 lines, -1 lines.".into(),
        }];

        append_agent_log(root, &git, "claude-code", "ses001", &entries).unwrap();

        let log_file = root.join("logs").join("claude-code-ses001.md");
        assert!(log_file.exists());
        let content = std::fs::read_to_string(&log_file).unwrap();
        assert!(content.contains("Agent Log: claude-code"));
        assert!(content.contains("prd.md"));
    }

    #[test]
    fn test_append_agent_log_appends_to_existing() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".docmgr")).unwrap();
        let git = GitStore::init(root).unwrap();

        let entries1 = vec![LogSynthEntry {
            timestamp: Utc::now(),
            path: PathBuf::from("a.md"),
            summary: "first".into(),
        }];
        append_agent_log(root, &git, "aider", "ses1", &entries1).unwrap();

        let entries2 = vec![LogSynthEntry {
            timestamp: Utc::now(),
            path: PathBuf::from("b.md"),
            summary: "second".into(),
        }];
        append_agent_log(root, &git, "aider", "ses1", &entries2).unwrap();

        let content = std::fs::read_to_string(root.join("logs").join("aider-ses1.md")).unwrap();
        assert!(content.contains("first"));
        assert!(content.contains("second"));
    }
}
