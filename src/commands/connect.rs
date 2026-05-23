use anyhow::Result;
use crate::session::{self, AgentSession};
use std::path::Path;

pub fn run_connect(root: &Path, name: &str, output: &dyn CliOutput) -> Result<()> {
    if let Some(existing) = session::load_session(root) {
        if !existing.is_stale() {
            anyhow::bail!(
                "Already connected as '{}'. Run 'agent-trace disconnect' first.",
                existing.name
            );
        }
    }

    let started = session::start_session(root, name, "cli")?;
    print_connect_message(&started, output)?;
    Ok(())
}

pub fn run_disconnect(root: &Path, output: &dyn CliOutput) -> Result<()> {
    if let Some(existing) = session::load_session(root) {
        session::remove_session(root)?;
        output.line(&format!("Disconnected '{}'. Actor reverts to User.", existing.name))?;
    } else {
        output.line("Not connected (no agent session active).")?;
    }
    Ok(())
}

fn print_connect_message(s: &AgentSession, output: &dyn CliOutput) -> Result<()> {
    output.line(&format!(
        "Connected as '{}'. Session {} (transport: {}).",
        s.name, s.session_id, s.transport
    ))?;
    output.line("Agent writes will be permission-checked and trace-linked.")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{AgentState, LOCK_FILE};
    use crate::types::Actor;
    use tempfile::TempDir;

    fn setup(tmp: &TempDir) -> std::path::PathBuf {
        let root = tmp.path().to_path_buf();
        std::fs::create_dir_all(root.join(".agent-trace/locks")).unwrap();
        root
    }

    #[test]
    fn test_connect_writes_lock_file() {
        let tmp = TempDir::new().unwrap();
        let root = setup(&tmp);
        run_connect(&root, "claude").unwrap();
        let lock = root.join(LOCK_FILE);
        assert!(lock.exists(), "lock file should exist after connect");
        let content = std::fs::read_to_string(&lock).unwrap();
        assert!(content.contains("claude"), "lock file should contain agent name");
        assert!(!content.contains("pid"), "new lock file should not contain pid");
    }

    #[test]
    fn test_connect_error_if_already_connected() {
        let tmp = TempDir::new().unwrap();
        let root = setup(&tmp);
        run_connect(&root, "claude").unwrap();
        let result = run_connect(&root, "another");
        assert!(result.is_err(), "double connect should fail");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("claude"), "error should name the existing agent");
    }

    #[test]
    fn test_disconnect_removes_lock_file() {
        let tmp = TempDir::new().unwrap();
        let root = setup(&tmp);
        run_connect(&root, "claude").unwrap();
        run_disconnect(&root).unwrap();
        assert!(!root.join(LOCK_FILE).exists(), "lock file should be gone after disconnect");
    }

    #[test]
    fn test_disconnect_noop_if_not_connected() {
        let tmp = TempDir::new().unwrap();
        let root = setup(&tmp);
        // Should not panic or error
        run_disconnect(&root).unwrap();
    }

    #[test]
    fn test_agent_state_reads_connect_lock() {
        let tmp = TempDir::new().unwrap();
        let root = setup(&tmp);
        run_connect(&root, "my-agent").unwrap();
        let state = AgentState::new(None);
        assert_eq!(
            state.current_actor(&root),
            Actor::Agent { name: "my-agent".into() }
        );
    }

    #[test]
    fn test_agent_state_reverts_to_user_after_disconnect() {
        let tmp = TempDir::new().unwrap();
        let root = setup(&tmp);
        run_connect(&root, "my-agent").unwrap();
        run_disconnect(&root).unwrap();
        let state = AgentState::new(None);
        assert_eq!(state.current_actor(&root), Actor::User);
    }
}
