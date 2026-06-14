use crate::config::{
    CredentialsStore, GlobalConfig, MergedConfig, PollingConfig, StoreConfig, StoreInfo,
};
use crate::llm::providers::{resolve, ResolvedBackendInfo};
use anyhow::{bail, Context, Result};
use std::path::Path;

/// Test-only escape hatch — never documented for end users.
pub fn allow_degraded_mode() -> bool {
    std::env::var("AGENT_TRACE_ALLOW_DEGRADED").as_deref() == Ok("1")
}

fn load_merged_for_gate(store_root: Option<&Path>) -> Result<MergedConfig> {
    match store_root {
        Some(root) if root.join(".agent-trace").exists() => {
            MergedConfig::load(root).context("failed to load store config")
        }
        _ => {
            let global = GlobalConfig::load()?;
            let store = StoreConfig {
                store: StoreInfo::new("gate-check".into()),
                llm: None,
                synthesis: None,
                polling: PollingConfig::default(),
            };
            Ok(MergedConfig::merge(global, store))
        }
    }
}

/// Fail fast when no reachable synthesis backend is configured.
pub fn require_synthesis_backend(store_root: Option<&Path>) -> Result<ResolvedBackendInfo> {
    let merged = load_merged_for_gate(store_root)?;
    let creds = CredentialsStore::load().unwrap_or_default();
    let resolved = resolve(&merged, &creds);
    let info = resolved.info();
    if info.degraded && !allow_degraded_mode() {
        bail!(
            "Synthesis backend unavailable. Run: agent-trace model setup && agent-trace model serve-check"
        );
    }
    Ok(info)
}

/// Used by trace pipeline paths that must not silently emit degraded artifacts.
pub fn ensure_synthesis_available(info: &ResolvedBackendInfo) -> Result<()> {
    if info.degraded && !allow_degraded_mode() {
        bail!(
            "Synthesis backend unavailable. Run: agent-trace model setup && agent-trace model serve-check"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_degraded_respects_env() {
        std::env::set_var("AGENT_TRACE_ALLOW_DEGRADED", "1");
        assert!(allow_degraded_mode());
        std::env::remove_var("AGENT_TRACE_ALLOW_DEGRADED");
        assert!(!allow_degraded_mode());
    }
}
