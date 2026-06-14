use crate::config::MergedConfig;
use crate::git_store::GitStore;
use crate::manifest::Manifest;
use crate::runtime::{AgentState, ChangeProcessor, InstanceLock, UiEvent};
use anyhow::Result;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::Sender;

/// Background filesystem activity monitor (poll loop + instance lock).
pub struct ActivityMonitor {
    _instance_lock: InstanceLock,
    #[allow(dead_code)]
    processor: Arc<Mutex<ChangeProcessor>>,
}

impl ActivityMonitor {
    /// Start the poll loop when the instance lock is available.
    pub fn try_start(
        store_root: &Path,
        config: MergedConfig,
        manifest: Arc<Mutex<Manifest>>,
        agent_state: AgentState,
        ui_tx: Option<Sender<UiEvent>>,
        runtime: &Runtime,
    ) -> Result<Option<Self>> {
        let instance_lock = match InstanceLock::acquire(store_root) {
            Ok(lock) => lock,
            Err(e) => {
                tracing::warn!("Activity monitor disabled: {e}");
                return Ok(None);
            }
        };

        let git = GitStore::open(store_root)?;
        let processor = ChangeProcessor::new(
            git,
            manifest,
            config.clone(),
            agent_state,
            ui_tx,
        );
        let processor = Arc::new(Mutex::new(processor));
        let poll_interval_ms = config.polling.interval_ms;
        let processor_clone = processor.clone();

        runtime.spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(poll_interval_ms)).await;
                if let Ok(mut p) = processor_clone.lock() {
                    if let Err(e) = p.run_poll_cycle() {
                        tracing::warn!("Poll cycle error: {e}");
                    }
                }
            }
        });

        Ok(Some(Self {
            _instance_lock: instance_lock,
            processor,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GlobalConfig, PollingConfig, StoreConfig, StoreInfo};
    use tempfile::TempDir;

    #[test]
    fn activity_monitor_starts_when_lock_available() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace/locks")).unwrap();
        let git = GitStore::init(root).unwrap();
        let _ = git;
        let info = StoreInfo::new("test".into());
        let manifest = Manifest::create_empty(info.clone(), root).unwrap();
        let global = GlobalConfig::default();
        let store_cfg = StoreConfig {
            store: info,
            llm: None,
            synthesis: None,
            polling: PollingConfig {
                interval_ms: 100,
                ..PollingConfig::default()
            },
        };
        let config = MergedConfig::merge(global, store_cfg);
        let manifest = Arc::new(Mutex::new(manifest));
        let runtime = Runtime::new().unwrap();
        let monitor = ActivityMonitor::try_start(
            root,
            config,
            manifest,
            AgentState::new(None),
            None,
            &runtime,
        )
        .unwrap();
        assert!(monitor.is_some());
    }
}
