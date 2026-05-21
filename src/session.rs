use crate::types::Actor;
use std::path::Path;

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
        let lock_path = store_root.join(".agent-trace").join("locks").join("agent-lock.toml");
        if lock_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&lock_path) {
                if let Ok(value) = toml::from_str::<toml::Value>(&content) {
                    if let Some(name) = value
                        .get("agent")
                        .and_then(|a| a.get("name"))
                        .and_then(|v| v.as_str())
                    {
                        return Actor::Agent {
                            name: name.to_string(),
                        };
                    }
                }
            }
        }

        if let Some(name) = &self.cli_agent {
            return Actor::Agent { name: name.clone() };
        }

        Actor::User
    }
}
