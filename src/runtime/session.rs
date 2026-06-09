use crate::types::Actor;
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const LOCK_FILE: &str = ".agent-trace/locks/agent-lock.toml";
const STALE_TIMEOUT_MINUTES: i64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentSession {
    pub name: String,
    pub session_id: String,
    pub transport: String,
    pub started_at: String,
    pub last_heartbeat: String,
}

pub fn new_session_id() -> String {
    let now = Utc::now();
    format!(
        "{}-{:03}",
        now.format("%Y%m%d-%H%M%S"),
        now.timestamp_subsec_millis()
    )
}

impl AgentSession {
    fn now_rfc3339() -> String {
        Utc::now().to_rfc3339()
    }

    fn parse_ts(ts: &str) -> Option<DateTime<Utc>> {
        chrono::DateTime::parse_from_rfc3339(ts)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    }

    pub fn is_stale(&self) -> bool {
        let Some(last) = Self::parse_ts(&self.last_heartbeat) else {
            return true;
        };
        Utc::now().signed_duration_since(last) > Duration::minutes(STALE_TIMEOUT_MINUTES)
    }

    pub fn refresh_heartbeat(&mut self) {
        self.last_heartbeat = Self::now_rfc3339();
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct LockFile {
    agent: AgentSession,
}

fn lock_path(store_root: &Path) -> std::path::PathBuf {
    store_root.join(LOCK_FILE)
}

fn write_lock(store_root: &Path, session: &AgentSession) -> Result<()> {
    let path = lock_path(store_root);
    std::fs::create_dir_all(path.parent().expect("lock parent"))?;
    let payload = toml::to_string(&LockFile {
        agent: session.clone(),
    })?;
    std::fs::write(path, payload)?;
    Ok(())
}

pub fn load_session(store_root: &Path) -> Option<AgentSession> {
    let path = lock_path(store_root);
    let content = std::fs::read_to_string(path).ok()?;
    let parsed: LockFile = toml::from_str(&content).ok()?;
    Some(parsed.agent)
}

pub fn start_session(store_root: &Path, name: &str, transport: &str) -> Result<AgentSession> {
    if let Some(existing) = load_session(store_root) {
        if existing.is_stale() {
            crate::session_recap::maybe_recap_prior_session(store_root, &existing)?;
            remove_session(store_root)?;
        }
    }

    let session = AgentSession {
        name: name.to_string(),
        session_id: new_session_id(),
        transport: transport.to_string(),
        started_at: AgentSession::now_rfc3339(),
        last_heartbeat: AgentSession::now_rfc3339(),
    };
    write_lock(store_root, &session)?;
    Ok(session)
}

pub fn touch_session(store_root: &Path, expected_name: &str) -> Result<()> {
    if let Some(mut session) = load_session(store_root) {
        if session.name == expected_name {
            session.refresh_heartbeat();
            write_lock(store_root, &session)?;
        }
    }
    Ok(())
}

pub fn remove_session(store_root: &Path) -> Result<()> {
    let path = lock_path(store_root);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// Returns the active session ID from the lock file, if non-stale.
pub fn session_id_for_store(store_root: &Path) -> Option<String> {
    load_session(store_root)
        .filter(|s| !s.is_stale())
        .map(|s| s.session_id)
}

pub fn session_id_for_actor(store_root: &Path, actor: &Actor) -> Option<String> {
    let Actor::Agent { name } = actor else {
        return None;
    };
    let session = load_session(store_root)?;
    if session.name != *name || session.is_stale() {
        return None;
    }
    Some(session.session_id)
}

/// Resolves the current actor for a command/session.
///
/// Priority:
/// 1) active lock file from `agent-trace connect`
/// 2) explicit CLI flag (`--agent` / `--actor`)
/// 3) fallback to user
pub struct AgentState {
    pub cli_agent: Option<String>,
}

impl AgentState {
    pub fn new(cli_agent: Option<String>) -> Self {
        Self { cli_agent }
    }

    pub fn current_actor(&self, store_root: &Path) -> Actor {
        if let Some(session) = load_session(store_root) {
            if session.is_stale() {
                let _ = crate::session_recap::maybe_recap_prior_session(store_root, &session);
                let _ = remove_session(store_root);
            } else {
                return Actor::Agent { name: session.name };
            }
        }

        if let Some(name) = &self.cli_agent {
            return Actor::Agent { name: name.clone() };
        }

        Actor::User
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn stale_session_is_ignored() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace/locks")).unwrap();
        std::fs::write(
            root.join(LOCK_FILE),
            "[agent]\nname=\"bot\"\nsession_id=\"s1\"\ntransport=\"cli\"\nstarted_at=\"2020-01-01T00:00:00Z\"\nlast_heartbeat=\"2020-01-01T00:00:00Z\"\n",
        )
        .unwrap();

        let state = AgentState::new(None);
        assert_eq!(state.current_actor(root), Actor::User);
    }

    #[test]
    fn start_session_writes_metadata() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let s = start_session(root, "worker", "mcp").unwrap();
        assert_eq!(s.name, "worker");
        assert_eq!(s.transport, "mcp");
        let loaded = load_session(root).unwrap();
        assert_eq!(loaded.name, "worker");
        assert!(!loaded.session_id.is_empty());
    }
}
