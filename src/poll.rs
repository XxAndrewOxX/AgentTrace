use crate::config::MergedConfig;
use crate::context::{synthesize_no_llm, write_context};
use crate::agent_trace_md;
use crate::git_store::{CommitInfo, GitStore};
use crate::log_synth::{append_agent_log, summarize_change_no_llm, LogSynthEntry};
use crate::manifest::Manifest;
use crate::permissions::{check_permission, Overrides, PermissionResult, Violation};
use crate::types::{Action, Actor, DocType, FileChange, LogEntry};
use anyhow::Result;
use chrono::Utc;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// ── UI Event channel ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum UiEvent {
    NewCommit(LogEntry),
    Violation(String),
    StatusMessage(String),
}

// ── Agent State ───────────────────────────────────────────────────────────────

pub struct AgentState {
    pub cli_agent: Option<String>,
}

impl AgentState {
    pub fn new(cli_agent: Option<String>) -> Self {
        Self { cli_agent }
    }

    pub fn current_actor(&self, store_root: &Path) -> Actor {
        // 1. Check agent-lock file.
        let lock_path = store_root.join(".agent-trace").join("locks").join("agent-lock.toml");
        if lock_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&lock_path) {
                if let Ok(value) = toml::from_str::<toml::Value>(&content) {
                    let agent_section = value.get("agent");
                    let pid = agent_section.and_then(|a| a.get("pid")).and_then(|v| v.as_integer());
                    let name = agent_section.and_then(|a| a.get("name")).and_then(|v| v.as_str()).map(String::from);

                    if let (Some(pid), Some(name)) = (pid, name) {
                        if is_pid_alive(pid as u32) {
                            return Actor::Agent { name };
                        } else {
                            // Stale lock — remove it.
                            let _ = std::fs::remove_file(&lock_path);
                        }
                    }
                }
            }
        }

        // 2. Check CLI --agent flag.
        if let Some(name) = &self.cli_agent {
            return Actor::Agent { name: name.clone() };
        }

        Actor::User
    }
}

fn is_pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // kill(pid, 0) returns 0 if process exists.
        let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
        result == 0
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

// ── Change Processor ──────────────────────────────────────────────────────────

pub struct ChangeProcessor {
    pub git: GitStore,
    pub manifest: Arc<Mutex<Manifest>>,
    pub agent_state: AgentState,
    pub ui_tx: Option<tokio::sync::mpsc::Sender<UiEvent>>,
    /// Session ID used for agent log file naming.
    session_id: String,
}

impl ChangeProcessor {
    pub fn new(
        git: GitStore,
        manifest: Arc<Mutex<Manifest>>,
        _config: MergedConfig,
        agent_state: AgentState,
        ui_tx: Option<tokio::sync::mpsc::Sender<UiEvent>>,
    ) -> Self {
        let session_id = format!("{}", Utc::now().format("%Y%m%d-%H%M%S"));
        Self { git, manifest, agent_state, ui_tx, session_id }
    }

    pub fn run_poll_cycle(&mut self) -> Result<()> {
        let store_root = self.git.workdir.clone();
        let changes = self.git.detect_changes()?;

        if changes.is_empty() {
            return Ok(());
        }

        let actor = self.agent_state.current_actor(&store_root);
        let overrides = Overrides::load(&store_root).unwrap_or_default();

        let mut allowed: Vec<(PathBuf, Action, DocType)> = Vec::new();
        let mut violations: Vec<Violation> = Vec::new();
        // Track newly-registered paths so we can roll back if the git commit fails.
        let mut newly_registered: Vec<PathBuf> = Vec::new();

        let mut manifest = self.manifest.lock().unwrap();

        for change in &changes {
            let path = change.path().clone();
            let action = change.action();

            // Determine doc type from manifest, default to Scratch for new files.
            let doc_type = manifest.find_by_path(&path)
                .map(|d| d.doc_type.clone())
                .unwrap_or(DocType::Scratch);

            let perm = check_permission(&doc_type, &actor, &overrides, Some(&path));

            match perm {
                PermissionResult::Allowed => {
                    // Register new files.
                    if matches!(change, FileChange::New(_)) && !manifest.is_tracked(&path) {
                        let agent_name = actor.agent_name().unwrap_or("");
                        let _ = manifest.register(&path, DocType::Scratch, agent_name);
                        newly_registered.push(path.clone());
                    }
                    // Update renamed paths.
                    if let FileChange::Renamed { from, to } = change {
                        let _ = manifest.update_path(from, to);
                    }
                    allowed.push((path, action, doc_type));
                }
                PermissionResult::Denied { reason } => {
                    // Revert the file.
                    if let Err(e) = self.git.revert_file(&path) {
                        tracing::warn!("Failed to revert {}: {}", path.display(), e);
                    }

                    // Save rejected content.
                    let full = store_root.join(&path);
                    if let Ok(content) = std::fs::read_to_string(&full) {
                        let _ = self.git.save_rejected(&path, &content);
                    }

                    violations.push(Violation {
                        timestamp: Utc::now(),
                        doc_path: path.clone(),
                        actor: actor.clone(),
                        agent_name: actor.agent_name().map(String::from),
                        attempted_action: action,
                        reason: reason.clone(),
                    });

                    // Send UI warning.
                    if let Some(tx) = &self.ui_tx {
                        let msg = format!(
                            "Permission denied: {} tried to modify {} ({})",
                            actor, path.display(), reason
                        );
                        let _ = tx.try_send(UiEvent::Violation(msg));
                    }
                }
                PermissionResult::RequiresConfirmation { .. } => {
                    // In headless/poll mode, require confirmation → treat as allowed for user.
                    allowed.push((path, action, doc_type));
                }
            }
        }

        // Commit violations.
        for v in &violations {
            let info = CommitInfo {
                action: Action::Violation,
                files: vec![(
                    v.doc_path.clone(),
                    v.attempted_action.clone(),
                    manifest.find_by_path(&v.doc_path)
                        .map(|d| d.doc_type.clone())
                        .unwrap_or(DocType::Scratch),
                )],
                actor: Actor::System,
                summary: format!(
                    "violation: {} attempted {} on {} — {}",
                    v.actor, v.attempted_action, v.doc_path.display(), v.reason
                ),
                agent_name: v.agent_name.clone(),
                session_id: None,
            };
            let _ = self.git.commit(&info);
        }

        // Batch commit allowed changes.
        if !allowed.is_empty() {
            let info = CommitInfo {
                action: allowed[0].1.clone(),
                files: allowed.clone(),
                actor: actor.clone(),
                summary: format!("{} {} file(s)", allowed[0].1, allowed.len()),
                agent_name: actor.agent_name().map(String::from),
                session_id: Some(self.session_id.clone()),
            };
            match self.git.commit(&info) {
                Ok(oid) => {
                    // Persist the manifest now that git is consistent.
                    let _ = manifest.save(&store_root);
                    let entry = LogEntry {
                        commit_id: oid.to_string(),
                        timestamp: Utc::now(),
                        action: info.action,
                        actor: actor.clone(),
                        agent_name: actor.agent_name().map(String::from),
                        files: info.files.clone(),
                        summary: info.summary,
                    };
                    if let Some(tx) = &self.ui_tx {
                        let _ = tx.try_send(UiEvent::NewCommit(entry));
                    }
                }
                Err(e) => {
                    // Roll back in-memory registrations so manifest stays consistent.
                    for path in &newly_registered {
                        let _ = manifest.untrack(path);
                    }
                    tracing::warn!(
                        "Commit failed, rolled back {} registration(s): {}",
                        newly_registered.len(), e
                    );
                    return Ok(());
                }
            }

            // If agent made changes, synthesize log entries.
            if actor.is_agent() {
                let agent_name = actor.agent_name().unwrap_or("unknown");
                let log_entries: Vec<LogSynthEntry> = allowed
                    .iter()
                    .map(|(path, _, doc_type)| {
                        let stats = self.git
                            .diff_stats(path, None, None)
                            .unwrap_or_default();
                        let summary = summarize_change_no_llm(path, doc_type, &stats, agent_name);
                        LogSynthEntry { timestamp: Utc::now(), path: path.clone(), summary }
                    })
                    .collect();
                if let Err(e) = append_agent_log(&store_root, &self.git, agent_name, &self.session_id, &log_entries) {
                    tracing::warn!("append_agent_log failed: {}", e);
                }
            }

            // Regenerate AGENT-TRACE.md atomically (tmp → rename so readers never see a partial file).
            let agent_trace_content = agent_trace_md::generate(&store_root, &manifest);
            let agent_trace_tmp = store_root.join(".agent-trace").join("AGENT-TRACE.md.tmp");
            if let Err(e) = std::fs::write(&agent_trace_tmp, &agent_trace_content)
                .and_then(|_| std::fs::rename(&agent_trace_tmp, store_root.join("AGENT-TRACE.md")))
            {
                tracing::warn!("AGENT-TRACE.md write failed: {}", e);
            }

            // If a plan or reference changed, re-synthesize context.md.
            let context_trigger = allowed.iter().any(|(_, _, dt)| {
                matches!(dt, DocType::Plan | DocType::Reference)
            });
            if context_trigger {
                match synthesize_no_llm(&store_root, &manifest) {
                    Ok(content) => {
                        if let Err(e) = write_context(&store_root, &content) {
                            tracing::warn!("write_context failed: {}", e);
                        }
                    }
                    Err(e) => tracing::warn!("synthesize_no_llm failed: {}", e),
                }
            }
        }

        Ok(())
    }
}

// ── Instance Locking ─────────────────────────────────────────────────────────

pub struct InstanceLock {
    path: PathBuf,
}

impl InstanceLock {
    pub fn acquire(store_root: &Path) -> Result<Self> {
        let path = store_root.join(".agent-trace").join("locks").join("instance.lock");
        std::fs::create_dir_all(path.parent().unwrap())?;

        // Check for stale lock.
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(pid) = content.trim().parse::<u32>() {
                    if is_pid_alive(pid) {
                        anyhow::bail!(
                            "Another agent-trace instance is running (PID {}). \
                             Opening in read-only mode.",
                            pid
                        );
                    }
                }
            }
        }

        let pid = std::process::id();
        std::fs::write(&path, pid.to_string())?;
        Ok(Self { path })
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{MergedConfig, GlobalConfig, StoreConfig, StoreInfo, PollingConfig};
    use tempfile::TempDir;

    fn setup(tmp: &TempDir) -> (GitStore, Arc<Mutex<Manifest>>, MergedConfig) {
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace").join("locks")).unwrap();
        let git = GitStore::init(root).unwrap();
        let info = StoreInfo::new("test".into());
        let manifest = Manifest::create_empty(info.clone(), root).unwrap();
        let global = GlobalConfig::default();
        let store_cfg = StoreConfig { store: info, llm: None, polling: PollingConfig::default() };
        let config = MergedConfig::merge(global, store_cfg);
        (git, Arc::new(Mutex::new(manifest)), config)
    }

    #[test]
    fn test_poll_no_changes() {
        let tmp = TempDir::new().unwrap();
        let (git, manifest, config) = setup(&tmp);
        let agent = AgentState::new(None);
        let mut proc = ChangeProcessor::new(git, manifest, config, agent, None);
        proc.run_poll_cycle().unwrap(); // should be a no-op
    }

    #[test]
    fn test_poll_new_file_registered() {
        let tmp = TempDir::new().unwrap();
        let (git, manifest, config) = setup(&tmp);

        // Create a new .md file.
        std::fs::write(tmp.path().join("notes.md"), "# notes").unwrap();

        let agent = AgentState::new(None);
        let mut proc = ChangeProcessor::new(git, manifest.clone(), config, agent, None);
        proc.run_poll_cycle().unwrap();

        let m = manifest.lock().unwrap();
        assert!(m.is_tracked(&PathBuf::from("notes.md")));
    }

    #[test]
    fn test_agent_state_no_lock_no_flag() {
        let tmp = TempDir::new().unwrap();
        let state = AgentState::new(None);
        assert_eq!(state.current_actor(tmp.path()), Actor::User);
    }

    #[test]
    fn test_agent_state_cli_flag() {
        let tmp = TempDir::new().unwrap();
        let state = AgentState::new(Some("aider".into()));
        assert_eq!(state.current_actor(tmp.path()), Actor::Agent { name: "aider".into() });
    }

    #[test]
    fn test_instance_lock_created_and_removed() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace").join("locks")).unwrap();
        let lock_path = root.join(".agent-trace").join("locks").join("instance.lock");

        {
            let _lock = InstanceLock::acquire(root).unwrap();
            assert!(lock_path.exists());
        }
        assert!(!lock_path.exists());
    }

    #[test]
    fn test_poll_agent_denied_context_reverted() {
        let tmp = TempDir::new().unwrap();
        let (git, manifest, config) = setup(&tmp);

        // Create and commit a context.md.
        std::fs::write(tmp.path().join("context.md"), "# Context").unwrap();
        {
            let info = CommitInfo {
                action: Action::Create,
                files: vec![(PathBuf::from("context.md"), Action::Create, DocType::Context)],
                actor: Actor::System,
                summary: "create context".into(),
                agent_name: None,
                session_id: None,
            };
            git.commit(&info).unwrap();
        }
        {
            let mut m = manifest.lock().unwrap();
            m.register(&PathBuf::from("context.md"), DocType::Context, "").unwrap();
        }

        // Agent modifies context.md.
        std::fs::write(tmp.path().join("context.md"), "# Agent tampered").unwrap();

        let agent = AgentState::new(Some("claude-code".into()));
        let mut proc = ChangeProcessor::new(git, manifest.clone(), config, agent, None);
        proc.run_poll_cycle().unwrap();

        // File should be reverted to original.
        let content = std::fs::read_to_string(tmp.path().join("context.md")).unwrap();
        assert_eq!(content, "# Context");
    }
}
