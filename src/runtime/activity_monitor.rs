use crate::config::MergedConfig;
use crate::git_store::GitStore;
use crate::manifest::Manifest;
use crate::runtime::{AgentState, ChangeProcessor, PollLock, UiEvent};
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
    #[allow(dead_code)]
    processor: Option<Arc<Mutex<ChangeProcessor>>>,
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
        let poll_lock = match PollLock::try_acquire(store_root)? {
            Some(lock) => lock,
            None => {
                tracing::info!(
                    "Poll leader already active in another process; \
                     running without a poll thread (HEAD-only updates)."
                );
                return Ok(Self {
                    poll_leader: false,
                    processor: None,
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
            processor: Some(processor),
            _poll_lock: Some(poll_lock),
        })
    }

    /// Whether this monitor owns the cross-process poll loop.
    pub fn is_poll_leader(&self) -> bool {
        self.poll_leader
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
            ActivityMonitor::try_start(root, config, manifest, AgentState::new(None), None).unwrap();
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
            ActivityMonitor::try_start(root, config, manifest, AgentState::new(None), None).unwrap();
        assert!(
            !second.is_poll_leader(),
            "second monitor must not run a duplicate poll loop"
        );
    }
}
