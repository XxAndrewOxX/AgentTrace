use crate::types::{Action, Actor, DocType};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ── Permission Result ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum PermissionResult {
    Allowed,
    Denied { reason: String },
    RequiresConfirmation { prompt: String },
}

// ── Core Permission Check ─────────────────────────────────────────────────────

/// Encoding of the permission table from PRD 4.2.2.
///
/// Permissions are currently checked by (actor, doc_type) only.
/// Action-level granularity (create vs modify vs delete) is not yet implemented.
pub fn check_permission(
    doc_type: &DocType,
    actor: &Actor,
    overrides: &Overrides,
    path: Option<&Path>,
) -> PermissionResult {
    // If there's an active override for this path + actor, allow everything.
    if let Some(p) = path {
        if overrides.is_overridden(p, actor) {
            return PermissionResult::Allowed;
        }
    }

    match (doc_type, actor) {
        // Plan: user + agent can do anything.
        (DocType::Plan, Actor::User | Actor::Agent { .. }) => PermissionResult::Allowed,
        (DocType::Plan, Actor::System) => PermissionResult::Denied {
            reason: "System cannot modify plan documents".into(),
        },

        // Context: system only. Agent denied; user requires confirmation.
        (DocType::Context, Actor::System) => PermissionResult::Allowed,
        (DocType::Context, Actor::Agent { .. }) => PermissionResult::Denied {
            reason: "Agents cannot modify system-owned context documents".into(),
        },
        (DocType::Context, Actor::User) => PermissionResult::RequiresConfirmation {
            prompt: "Context is system-synthesized. Use `agent-trace context update \"...\"` to update. Overwrite directly? [y/N]".into(),
        },

        // Log: system only. Agent denied; user requires confirmation.
        (DocType::Log, Actor::System) => PermissionResult::Allowed,
        (DocType::Log, Actor::Agent { .. }) => PermissionResult::Denied {
            reason: "Agents cannot modify system-owned log documents".into(),
        },
        (DocType::Log, Actor::User) => PermissionResult::RequiresConfirmation {
            prompt: "Log documents are system-managed. Modify anyway? [y/N]".into(),
        },

        // Reference: user only. Agent denied; system denied.
        (DocType::Reference, Actor::User) => PermissionResult::Allowed,
        (DocType::Reference, Actor::Agent { .. }) => PermissionResult::Denied {
            reason: "Agents cannot modify reference documents".into(),
        },
        (DocType::Reference, Actor::System) => PermissionResult::Denied {
            reason: "System cannot modify reference documents".into(),
        },

        // Scratch: anyone can write.
        (DocType::Scratch, _) => PermissionResult::Allowed,
    }
}

// ── Override Entry ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OverrideEntry {
    pub doc_id: String,
    pub path: PathBuf,
    pub allow_actor: String, // serialized Actor string
    pub granted_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub granted_by: String,
}

impl OverrideEntry {
    pub fn is_active(&self) -> bool {
        Utc::now() < self.expires_at
    }

    pub fn matches_actor(&self, actor: &Actor) -> bool {
        // Match by actor kind.
        match actor {
            Actor::User => self.allow_actor == "user",
            Actor::Agent { name } => {
                self.allow_actor == "agent" || self.allow_actor == format!("agent:{name}")
            }
            Actor::System => self.allow_actor == "system",
        }
    }
}

// ── Overrides ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Overrides {
    #[serde(default, rename = "override")]
    pub entries: Vec<OverrideEntry>,
}

impl Overrides {
    pub fn load(store_root: &Path) -> Result<Self> {
        let path = overrides_path(store_root);
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("Reading overrides: {}", path.display()))?;
        toml::from_str(&contents).with_context(|| format!("Parsing overrides: {}", path.display()))
    }

    pub fn save(&self, store_root: &Path) -> Result<()> {
        let path = overrides_path(store_root);
        std::fs::create_dir_all(path.parent().unwrap())?;
        let contents = toml::to_string_pretty(self)?;
        crate::util::atomic_write(&path, &contents)?;
        Ok(())
    }

    pub fn add(&mut self, entry: OverrideEntry) -> Result<()> {
        self.entries.push(entry);
        Ok(())
    }

    pub fn is_overridden(&self, path: &Path, actor: &Actor) -> bool {
        self.entries
            .iter()
            .any(|e| e.path == path && e.is_active() && e.matches_actor(actor))
    }

    pub fn prune_expired(&mut self) {
        self.entries.retain(|e| e.is_active());
    }
}

fn overrides_path(store_root: &Path) -> PathBuf {
    store_root.join(".agent-trace").join("overrides.toml")
}

// ── Violation ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Violation {
    pub timestamp: DateTime<Utc>,
    pub doc_path: PathBuf,
    pub actor: Actor,
    pub agent_name: Option<String>,
    pub attempted_action: Action,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use tempfile::TempDir;

    fn agent(name: &str) -> Actor {
        Actor::Agent { name: name.into() }
    }

    #[test]
    fn test_plan_permissions() {
        let o = Overrides::default();
        assert_eq!(
            check_permission(&DocType::Plan, &Actor::User, &o, None),
            PermissionResult::Allowed
        );
        assert_eq!(
            check_permission(&DocType::Plan, &agent("claude"), &o, None),
            PermissionResult::Allowed
        );
        assert!(matches!(
            check_permission(&DocType::Plan, &Actor::System, &o, None),
            PermissionResult::Denied { .. }
        ));
    }

    #[test]
    fn test_context_permissions() {
        let o = Overrides::default();
        assert_eq!(
            check_permission(&DocType::Context, &Actor::System, &o, None),
            PermissionResult::Allowed
        );
        assert!(matches!(
            check_permission(&DocType::Context, &agent("aider"), &o, None),
            PermissionResult::Denied { .. }
        ));
        assert!(matches!(
            check_permission(&DocType::Context, &Actor::User, &o, None),
            PermissionResult::RequiresConfirmation { .. }
        ));
    }

    #[test]
    fn test_log_permissions() {
        let o = Overrides::default();
        assert_eq!(
            check_permission(&DocType::Log, &Actor::System, &o, None),
            PermissionResult::Allowed
        );
        assert!(matches!(
            check_permission(&DocType::Log, &agent("x"), &o, None),
            PermissionResult::Denied { .. }
        ));
        assert!(matches!(
            check_permission(&DocType::Log, &Actor::User, &o, None),
            PermissionResult::RequiresConfirmation { .. }
        ));
    }

    #[test]
    fn test_reference_permissions() {
        let o = Overrides::default();
        assert_eq!(
            check_permission(&DocType::Reference, &Actor::User, &o, None),
            PermissionResult::Allowed
        );
        assert!(matches!(
            check_permission(&DocType::Reference, &agent("x"), &o, None),
            PermissionResult::Denied { .. }
        ));
        assert!(matches!(
            check_permission(&DocType::Reference, &Actor::System, &o, None),
            PermissionResult::Denied { .. }
        ));
    }

    #[test]
    fn test_scratch_permissions() {
        let o = Overrides::default();
        assert_eq!(
            check_permission(&DocType::Scratch, &Actor::User, &o, None),
            PermissionResult::Allowed
        );
        assert_eq!(
            check_permission(&DocType::Scratch, &agent("x"), &o, None),
            PermissionResult::Allowed
        );
        assert_eq!(
            check_permission(&DocType::Scratch, &Actor::System, &o, None),
            PermissionResult::Allowed
        );
    }

    #[test]
    fn test_active_override_allows() {
        let mut o = Overrides::default();
        let path = PathBuf::from("api.md");
        o.add(OverrideEntry {
            doc_id: "id1".into(),
            path: path.clone(),
            allow_actor: "agent".into(),
            granted_at: Utc::now(),
            expires_at: Utc::now() + Duration::minutes(10),
            granted_by: "user".into(),
        })
        .unwrap();
        assert_eq!(
            check_permission(&DocType::Reference, &agent("claude"), &o, Some(&path)),
            PermissionResult::Allowed
        );
    }

    #[test]
    fn test_expired_override_denied() {
        let mut o = Overrides::default();
        let path = PathBuf::from("api.md");
        o.add(OverrideEntry {
            doc_id: "id1".into(),
            path: path.clone(),
            allow_actor: "agent".into(),
            granted_at: Utc::now() - Duration::hours(2),
            expires_at: Utc::now() - Duration::hours(1),
            granted_by: "user".into(),
        })
        .unwrap();
        assert!(matches!(
            check_permission(&DocType::Reference, &agent("claude"), &o, Some(&path)),
            PermissionResult::Denied { .. }
        ));
    }

    #[test]
    fn test_override_path_specificity() {
        let mut o = Overrides::default();
        let path_a = PathBuf::from("a.md");
        let path_b = PathBuf::from("b.md");
        o.add(OverrideEntry {
            doc_id: "id1".into(),
            path: path_a.clone(),
            allow_actor: "agent".into(),
            granted_at: Utc::now(),
            expires_at: Utc::now() + Duration::minutes(10),
            granted_by: "user".into(),
        })
        .unwrap();
        // Override for a.md doesn't affect b.md
        assert!(matches!(
            check_permission(&DocType::Reference, &agent("x"), &o, Some(&path_b)),
            PermissionResult::Denied { .. }
        ));
    }

    #[test]
    fn test_prune_expired() {
        let mut o = Overrides::default();
        o.add(OverrideEntry {
            doc_id: "id1".into(),
            path: PathBuf::from("a.md"),
            allow_actor: "agent".into(),
            granted_at: Utc::now() - Duration::hours(2),
            expires_at: Utc::now() - Duration::hours(1),
            granted_by: "user".into(),
        })
        .unwrap();
        o.add(OverrideEntry {
            doc_id: "id2".into(),
            path: PathBuf::from("b.md"),
            allow_actor: "user".into(),
            granted_at: Utc::now(),
            expires_at: Utc::now() + Duration::minutes(10),
            granted_by: "user".into(),
        })
        .unwrap();
        o.prune_expired();
        assert_eq!(o.entries.len(), 1);
        assert_eq!(o.entries[0].doc_id, "id2");
    }

    #[test]
    fn test_overrides_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        let mut o = Overrides::default();
        o.add(OverrideEntry {
            doc_id: "id1".into(),
            path: PathBuf::from("api.md"),
            allow_actor: "agent".into(),
            granted_at: Utc::now(),
            expires_at: Utc::now() + Duration::minutes(10),
            granted_by: "user".into(),
        })
        .unwrap();
        o.save(root).unwrap();
        let loaded = Overrides::load(root).unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].doc_id, "id1");
    }
}
