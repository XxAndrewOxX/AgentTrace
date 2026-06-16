use crate::llm::{Llm, ResolvedBackendInfo};
use anyhow::Result;
use std::path::Path;

/// Test-only escape hatch — never documented for end users.
/// In-process unit tests (`cfg(test)`) also allow degraded mode.
pub fn allow_degraded_mode() -> bool {
    if cfg!(test) {
        return true;
    }
    std::env::var("AGENT_TRACE_ALLOW_DEGRADED").as_deref() == Ok("1")
}

/// Fail fast when no reachable synthesis backend is configured.
/// Delegates to `Llm::require_backend`.
pub fn require_synthesis_backend(store_root: Option<&Path>) -> Result<ResolvedBackendInfo> {
    Llm::require_backend(store_root)
}

/// Used by trace pipeline paths that must not silently emit degraded artifacts.
pub fn ensure_synthesis_available(info: &ResolvedBackendInfo) -> Result<()> {
    if info.degraded && !allow_degraded_mode() {
        anyhow::bail!("Synthesis backend unavailable. Run: agent-trace model ensure");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SynthesisProvider;

    #[test]
    fn allow_degraded_in_unit_test_builds() {
        assert!(allow_degraded_mode());
    }

    #[test]
    fn ensure_synthesis_available_ok_when_not_degraded() {
        let info = ResolvedBackendInfo {
            label: "ollama/qwen2.5".into(),
            provider: SynthesisProvider::Ollama,
            model: "qwen2.5".into(),
            degraded: false,
        };
        assert!(ensure_synthesis_available(&info).is_ok());
    }

    #[test]
    fn ensure_synthesis_available_ok_when_degraded_but_allowed() {
        let info = ResolvedBackendInfo {
            label: "degraded".into(),
            provider: SynthesisProvider::Ollama,
            model: "mechanical".into(),
            degraded: true,
        };
        assert!(allow_degraded_mode());
        assert!(ensure_synthesis_available(&info).is_ok());
    }
}
