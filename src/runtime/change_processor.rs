use crate::config::MergedConfig;
use crate::git_store::{CommitInfo, GitStore};
use crate::manifest::Manifest;
use crate::permissions::{check_permission, Overrides, PermissionResult, Violation};
use crate::session;
use crate::trace::pipeline::apply_trace_hooks;
use crate::types::{Action, Actor, DocType, FileChange, LogEntry};
use anyhow::Result;
use chrono::Utc;
use git2::Oid;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub use crate::runtime::session::AgentState;

// ── UI Event channel ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum UiEvent {
    NewCommit(LogEntry),
    Violation(String),
    StatusMessage(String),
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
    /// Last HEAD OID seen by the poll loop (dedup with external commits).
    last_seen_oid: Oid,
}

impl ChangeProcessor {
    pub fn new(
        git: GitStore,
        manifest: Arc<Mutex<Manifest>>,
        _config: MergedConfig,
        agent_state: AgentState,
        ui_tx: Option<tokio::sync::mpsc::Sender<UiEvent>>,
    ) -> Self {
        let session_id =
            session::session_id_for_store(&git.workdir).unwrap_or_else(session::new_session_id);
        let last_seen_oid = git.head_oid().unwrap_or_else(|_| Oid::zero());
        Self {
            git,
            manifest,
            agent_state,
            ui_tx,
            session_id,
            last_seen_oid,
        }
    }

    fn refresh_session_id(&mut self) {
        if let Some(sid) = session::session_id_for_store(&self.git.workdir) {
            self.session_id = sid;
        }
    }

    fn reload_manifest_from_disk(&self) -> Result<()> {
        let loaded = Manifest::load(&self.git.workdir)?;
        *self.manifest.lock().unwrap() = loaded;
        Ok(())
    }

    fn poll_external_commits(&mut self) -> Result<()> {
        let current_head = self.git.head_oid()?;
        if current_head == self.last_seen_oid {
            return Ok(());
        }

        let new_commits = self.git.commits_since(self.last_seen_oid)?;
        if !new_commits.is_empty() {
            // Violation-only commits do not touch manifest.toml; reloading would
            // clobber in-memory registrations with a stale on-disk manifest.
            let has_non_violation = new_commits
                .iter()
                .any(|e| !matches!(e.action, Action::Violation));
            if has_non_violation {
                if let Err(e) = self.reload_manifest_from_disk() {
                    tracing::warn!("Failed to reload manifest after external commit: {}", e);
                }
            }
            if let Some(tx) = &self.ui_tx {
                for entry in new_commits {
                    let _ = tx.try_send(UiEvent::NewCommit(entry));
                }
            }
        }

        self.last_seen_oid = current_head;
        Ok(())
    }

    pub fn run_poll_cycle(&mut self) -> Result<()> {
        self.refresh_session_id();

        let store_root = self.git.workdir.clone();
        let changes = self.git.detect_changes()?;
        let mut own_commit_oid: Option<Oid> = None;

        if changes.is_empty() {
            return self.poll_external_commits();
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
            let doc_type = manifest
                .find_by_path(&path)
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
                            actor,
                            path.display(),
                            reason
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
                    manifest
                        .find_by_path(&v.doc_path)
                        .map(|d| d.doc_type.clone())
                        .unwrap_or(DocType::Scratch),
                )],
                actor: Actor::System,
                summary: format!(
                    "violation: {} attempted {} on {} — {}",
                    v.actor,
                    v.attempted_action,
                    v.doc_path.display(),
                    v.reason
                ),
                agent_name: v.agent_name.clone(),
                session_id: None,
            };
            if let Err(e) = self.git.commit(&info) {
                tracing::error!(
                    "Failed to commit violation record for {:?}: {:#}",
                    v.doc_path,
                    e
                );
            }
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
                    own_commit_oid = Some(oid);
                    // Persist the manifest now that git is consistent.
                    let _ = manifest.save(&store_root);
                    let entry = LogEntry {
                        commit_id: crate::types::CommitId(oid.to_string()),
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

                    if let Err(e) = apply_trace_hooks(
                        &store_root,
                        &self.git,
                        &manifest,
                        &actor,
                        Some(&self.session_id),
                        &allowed,
                        "poll",
                    ) {
                        tracing::warn!("post-write trace hooks failed: {}", e);
                    }
                }
                Err(e) => {
                    // Roll back in-memory registrations so manifest stays consistent.
                    for path in &newly_registered {
                        let _ = manifest.untrack(path);
                    }
                    tracing::warn!(
                        "Commit failed, rolled back {} registration(s): {}",
                        newly_registered.len(),
                        e
                    );
                    drop(manifest);
                    return self.poll_external_commits();
                }
            }
        }

        drop(manifest);

        // Sync to HEAD after poll-cycle commits (batch + trace hooks) so HEAD poll
        // does not re-emit commits from this cycle.
        if own_commit_oid.is_some() {
            if let Ok(head) = self.git.head_oid() {
                self.last_seen_oid = head;
            }
        }
        self.poll_external_commits()
    }
}

// ── Instance Locking ─────────────────────────────────────────────────────────

pub struct InstanceLock {
    path: PathBuf,
}

impl InstanceLock {
    pub fn acquire(store_root: &Path) -> Result<Self> {
        let path = store_root
            .join(".agent-trace")
            .join("locks")
            .join("instance.lock");
        std::fs::create_dir_all(path.parent().unwrap())?;

        // Check for stale lock.
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(pid) = content.trim().parse::<u32>() {
                    if is_pid_alive(pid) {
                        anyhow::bail!(
                            "Another agent-trace instance is running (PID {pid}). \
                             Opening in read-only mode."
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
    use crate::config::{GlobalConfig, MergedConfig, PollingConfig, StoreConfig, StoreInfo};
    use crate::session::AgentState;
    use tempfile::TempDir;

    fn setup(tmp: &TempDir) -> (GitStore, Arc<Mutex<Manifest>>, MergedConfig) {
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace").join("locks")).unwrap();
        let git = GitStore::init(root).unwrap();
        let info = StoreInfo::new("test".into());
        let manifest = Manifest::create_empty(info.clone(), root).unwrap();
        let global = GlobalConfig::default();
        let store_cfg = StoreConfig {
            store: info,
            llm: None,
            synthesis: None,
            polling: PollingConfig::default(),
        };
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
        assert_eq!(
            state.current_actor(tmp.path()),
            Actor::Agent {
                name: "aider".into()
            }
        );
    }

    #[test]
    fn test_agent_state_connect_lock_file() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace/locks")).unwrap();
        // Simulate `agent-trace connect my-agent` (no PID field)
        std::fs::write(
            root.join(".agent-trace/locks/agent-lock.toml"),
            "[agent]\nname=\"my-agent\"\nsession_id=\"s1\"\ntransport=\"cli\"\nstarted_at=\"2026-01-01T00:00:00Z\"\nlast_heartbeat=\"2099-01-01T00:00:00Z\"\n",
        ).unwrap();
        let state = AgentState::new(None);
        assert_eq!(
            state.current_actor(root),
            Actor::Agent {
                name: "my-agent".into()
            }
        );
    }

    #[test]
    fn test_agent_state_cli_flag_takes_priority_over_no_lock() {
        let tmp = TempDir::new().unwrap();
        // No lock file, but CLI flag set — should be Agent
        let state = AgentState::new(Some("cli-agent".into()));
        assert_eq!(
            state.current_actor(tmp.path()),
            Actor::Agent {
                name: "cli-agent".into()
            }
        );
    }

    #[test]
    fn test_instance_lock_created_and_removed() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace").join("locks")).unwrap();
        let lock_path = root
            .join(".agent-trace")
            .join("locks")
            .join("instance.lock");

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
                files: vec![(
                    PathBuf::from("context.md"),
                    Action::Create,
                    DocType::Context,
                )],
                actor: Actor::System,
                summary: "create context".into(),
                agent_name: None,
                session_id: None,
            };
            git.commit(&info).unwrap();
        }
        {
            let mut m = manifest.lock().unwrap();
            m.register(&PathBuf::from("context.md"), DocType::Context, "")
                .unwrap();
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

    #[test]
    fn test_head_poll_detects_external_commit() {
        let tmp = TempDir::new().unwrap();
        let (git, manifest, config) = setup(&tmp);
        let (tx, mut rx) = tokio::sync::mpsc::channel(10);

        let agent = AgentState::new(None);
        let mut proc = ChangeProcessor::new(git, manifest.clone(), config, agent, Some(tx));

        // Simulate MCP write: file + manifest update + commit outside poll loop.
        std::fs::write(tmp.path().join("mcp.md"), "# MCP write").unwrap();
        {
            let mut m = Manifest::load(tmp.path()).unwrap();
            m.register(&PathBuf::from("mcp.md"), DocType::Plan, "claude")
                .unwrap();
            m.save(tmp.path()).unwrap();
        }
        let store = GitStore::open(tmp.path()).unwrap();
        let info = CommitInfo {
            action: Action::Create,
            files: vec![(PathBuf::from("mcp.md"), Action::Create, DocType::Plan)],
            actor: Actor::Agent {
                name: "claude".into(),
            },
            summary: "mcp write: mcp.md".into(),
            agent_name: Some("claude".into()),
            session_id: Some("sess-mcp".into()),
        };
        store.commit(&info).unwrap();

        // In-memory manifest is stale (no mcp.md).
        assert!(
            !manifest
                .lock()
                .unwrap()
                .is_tracked(&PathBuf::from("mcp.md")),
            "pre-poll manifest should not track mcp.md"
        );

        proc.run_poll_cycle().unwrap();

        let event = rx
            .try_recv()
            .expect("expected NewCommit for external commit");
        match event {
            UiEvent::NewCommit(entry) => {
                assert_eq!(entry.agent_name.as_deref(), Some("claude"));
                assert!(entry.summary.contains("mcp"));
            }
            other => panic!("expected NewCommit, got {other:?}"),
        }

        assert!(
            manifest
                .lock()
                .unwrap()
                .is_tracked(&PathBuf::from("mcp.md")),
            "manifest should reload from disk after external commit"
        );
    }

    #[test]
    fn test_poll_own_commit_not_duplicated_via_head_poll() {
        let tmp = TempDir::new().unwrap();
        let (git, manifest, config) = setup(&tmp);
        let (tx, mut rx) = tokio::sync::mpsc::channel(10);

        std::fs::write(tmp.path().join("notes.md"), "# notes").unwrap();

        let agent = AgentState::new(None);
        let mut proc = ChangeProcessor::new(git, manifest.clone(), config, agent, Some(tx));
        proc.run_poll_cycle().unwrap();

        let first = rx.try_recv().expect("poll commit should emit once");
        assert!(matches!(first, UiEvent::NewCommit(_)));
        assert!(
            rx.try_recv().is_err(),
            "HEAD poll should not duplicate poll-loop commit"
        );
    }

    #[test]
    fn test_session_id_reread_from_lock() {
        let tmp = TempDir::new().unwrap();
        let (git, manifest, config) = setup(&tmp);

        std::fs::write(
            tmp.path().join(".agent-trace/locks/agent-lock.toml"),
            "[agent]\nname=\"my-agent\"\nsession_id=\"lock-sid-42\"\ntransport=\"mcp\"\nstarted_at=\"2099-01-01T00:00:00Z\"\nlast_heartbeat=\"2099-01-01T00:00:00Z\"\n",
        )
        .unwrap();

        std::fs::write(tmp.path().join("scratch.md"), "# scratch").unwrap();

        let agent = AgentState::new(None);
        let mut proc = ChangeProcessor::new(git, manifest.clone(), config, agent, None);
        proc.run_poll_cycle().unwrap();

        let store = GitStore::open(tmp.path()).unwrap();
        let all = store.log(10).unwrap();
        let scratch_entry = all
            .iter()
            .find(|e| {
                e.files
                    .iter()
                    .any(|(p, _, _)| p == &PathBuf::from("scratch.md"))
            })
            .expect("poll commit for scratch.md expected");
        let commit_id = scratch_entry.commit_id.0.clone();
        let repo_path = tmp.path().join(".agent-trace/repo");
        let repo = git2::Repository::open(&repo_path).unwrap();
        let commit = repo
            .find_commit(git2::Oid::from_str(&commit_id).expect("valid commit oid"))
            .unwrap();
        let msg = commit.message().unwrap_or("");
        assert!(
            msg.contains("lock-sid-42"),
            "commit should use session_id from lock file, got:\n{msg}"
        );
    }

    #[test]
    fn test_head_poll_no_changes_still_runs() {
        let tmp = TempDir::new().unwrap();
        let (git, manifest, config) = setup(&tmp);
        let agent = AgentState::new(None);
        let mut proc = ChangeProcessor::new(git, manifest, config, agent, None);
        proc.run_poll_cycle().unwrap(); // no file changes, should not error
    }
}
