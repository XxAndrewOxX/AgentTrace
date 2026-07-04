use crate::config::MergedConfig;
use crate::git_store::GitStore;
use crate::manifest::Manifest;
use crate::runtime::{AgentState, ChangeProcessor, PollLock, UiEvent};
use crate::types::Action;
use anyhow::Result;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::Sender;

/// Background filesystem activity monitor.
///
/// Poll-loop ownership is elected across processes with a [`PollLock`]. Only the
/// process that acquires the lock runs the poll thread (the *poll leader*);
/// other processes still construct an `ActivityMonitor` but with
/// `poll_leader == false` and no thread, relying on HEAD-only updates so that
/// commits / activity events are never duplicated.
pub struct ActivityMonitor {
    poll_leader: bool,
    _processor: Option<Arc<Mutex<ChangeProcessor>>>,
    /// Held for the lifetime of the monitor to keep poll leadership.
    _poll_lock: Option<PollLock>,
}

impl ActivityMonitor {
    /// Start the poll loop if this process can become the poll leader.
    ///
    /// Always returns a monitor: when the poll lock is already held by another
    /// process the returned monitor has no poll thread (`is_poll_leader()` is
    /// `false`).
    pub fn try_start(
        store_root: &Path,
        config: MergedConfig,
        manifest: Arc<Mutex<Manifest>>,
        agent_state: AgentState,
        ui_tx: Option<Sender<UiEvent>>,
    ) -> Result<Self> {
        if !config.polling.enabled {
            tracing::info!("Polling disabled in store config; activity monitor not started.");
            return Ok(Self {
                poll_leader: false,
                _processor: None,
                _poll_lock: None,
            });
        }

        let poll_lock = match PollLock::try_acquire(store_root)? {
            Some(lock) => lock,
            None => {
                tracing::info!(
                    "Poll leader already active in another process; \
                     running HEAD observer for TUI updates."
                );
                if let Some(tx) = ui_tx {
                    Self::start_head_watcher(store_root, config.polling.interval_ms, manifest, tx)?;
                }
                return Ok(Self {
                    poll_leader: false,
                    _processor: None,
                    _poll_lock: None,
                });
            }
        };

        let git = GitStore::open(store_root)?;
        let processor = ChangeProcessor::new(git, manifest, config.clone(), agent_state, ui_tx);
        let processor = Arc::new(Mutex::new(processor));
        let poll_interval_ms = config.polling.interval_ms;
        let processor_clone = processor.clone();

        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(poll_interval_ms));
            if let Ok(mut p) = processor_clone.lock() {
                if let Err(e) = p.run_poll_cycle() {
                    tracing::warn!("Poll cycle error: {e}");
                }
            }
        });

        Ok(Self {
            poll_leader: true,
            _processor: Some(processor),
            _poll_lock: Some(poll_lock),
        })
    }

    /// Whether this monitor owns the cross-process poll loop.
    pub fn is_poll_leader(&self) -> bool {
        self.poll_leader
    }

    /// Lightweight HEAD watcher for observer/read-only TUI instances.
    pub fn start_head_watcher(
        store_root: &Path,
        interval_ms: u64,
        manifest: Arc<Mutex<Manifest>>,
        ui_tx: Sender<UiEvent>,
    ) -> Result<()> {
        let store_root = store_root.to_path_buf();
        let git = GitStore::open(&store_root)?;
        let mut last_seen_oid = git.head_oid()?;

        std::thread::spawn(move || {
            let git = match GitStore::open(&store_root) {
                Ok(g) => g,
                Err(e) => {
                    tracing::warn!("HEAD watcher failed to open git store: {e}");
                    return;
                }
            };
            loop {
                std::thread::sleep(Duration::from_millis(interval_ms));
                let current_head = match git.head_oid() {
                    Ok(oid) => oid,
                    Err(e) => {
                        tracing::warn!("HEAD watcher oid error: {e}");
                        continue;
                    }
                };
                if current_head == last_seen_oid {
                    continue;
                }
                match git.commits_since(last_seen_oid) {
                    Ok(commits) => {
                        let has_non_violation = commits
                            .iter()
                            .any(|e| !matches!(e.action, Action::Violation));
                        if has_non_violation {
                            if let Ok(m) = Manifest::load(&store_root) {
                                if let Ok(mut guard) = manifest.lock() {
                                    *guard = m;
                                }
                            }
                        }
                        for entry in commits {
                            let _ = ui_tx.try_send(UiEvent::NewCommit(entry));
                        }
                    }
                    Err(e) => tracing::warn!("HEAD watcher commits_since error: {e}"),
                }
                last_seen_oid = current_head;
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GlobalConfig, PollingConfig, StoreConfig, StoreInfo};
    use tempfile::TempDir;

    #[test]
    fn activity_monitor_becomes_poll_leader_when_lock_free() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace/locks")).unwrap();
        let git = GitStore::init(root).unwrap();
        let _ = git;
        let info = StoreInfo::new("test".into());
        let manifest = Manifest::create_empty(info.clone(), root).unwrap();
        let store_cfg = StoreConfig {
            store: info,
            llm: None,
            synthesis: None,
            polling: PollingConfig {
                interval_ms: 100,
                ..PollingConfig::default()
            },
        };
        store_cfg.save(root).unwrap();
        let global = GlobalConfig::default();
        let config = MergedConfig::merge(global, store_cfg);
        let manifest = Arc::new(Mutex::new(manifest));
        let monitor =
            ActivityMonitor::try_start(root, config, manifest, AgentState::new(None), None)
                .unwrap();
        assert!(monitor.is_poll_leader());
    }

    #[test]
    fn second_monitor_is_not_poll_leader() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace/locks")).unwrap();
        let git = GitStore::init(root).unwrap();
        let _ = git;
        let info = StoreInfo::new("test".into());
        let manifest = Manifest::create_empty(info.clone(), root).unwrap();
        let store_cfg = StoreConfig {
            store: info,
            llm: None,
            synthesis: None,
            polling: PollingConfig {
                interval_ms: 100,
                ..PollingConfig::default()
            },
        };
        store_cfg.save(root).unwrap();
        let global = GlobalConfig::default();
        let config = MergedConfig::merge(global, store_cfg);
        let manifest = Arc::new(Mutex::new(manifest));

        let first = ActivityMonitor::try_start(
            root,
            config.clone(),
            manifest.clone(),
            AgentState::new(None),
            None,
        )
        .unwrap();
        assert!(first.is_poll_leader(), "first monitor should lead the poll");

        let second =
            ActivityMonitor::try_start(root, config, manifest, AgentState::new(None), None)
                .unwrap();
        assert!(
            !second.is_poll_leader(),
            "second monitor must not run a duplicate poll loop"
        );
    }
}
