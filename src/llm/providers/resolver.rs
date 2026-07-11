use super::http::HttpBackend;
use super::ollama;
use crate::config::{
    CredentialsStore, MergedConfig, SynthesisConfig, SynthesisMode, SynthesisProvider,
};
use crate::llm::synthesis_engine::{DegradedBackend, SynthesisEngine};
use std::sync::{Arc, LazyLock, Mutex};

static DEGRADED_WARNED: LazyLock<Mutex<bool>> = LazyLock::new(|| Mutex::new(false));

#[derive(Debug, Clone)]
pub struct ResolvedBackendInfo {
    pub label: String,
    pub provider: SynthesisProvider,
    pub model: String,
    pub degraded: bool,
}

pub enum ResolvedBackend {
    Http(HttpBackend),
    Degraded(DegradedBackend),
}

impl ResolvedBackend {
    pub fn info(&self) -> ResolvedBackendInfo {
        match self {
            Self::Http(b) => ResolvedBackendInfo {
                label: b.label().to_string(),
                provider: b.provider,
                model: b.model.clone(),
                degraded: false,
            },
            Self::Degraded(_) => ResolvedBackendInfo {
                label: "degraded".into(),
                provider: SynthesisProvider::Ollama,
                model: "mechanical".into(),
                degraded: true,
            },
        }
    }

    pub fn into_engine(self) -> Arc<dyn SynthesisEngine> {
        match self {
            Self::Http(b) => Arc::new(b),
            Self::Degraded(b) => Arc::new(b),
        }
    }
}

pub fn resolve(merged: &MergedConfig, creds: &CredentialsStore) -> ResolvedBackend {
    // Always re-enter resolution. Health probes themselves are TTL-cached
    // inside HttpBackend (short negative TTL, longer positive TTL) so this is
    // cheap without pinning the process in degraded mode for 30s after recovery.
    let syn = &merged.synthesis;
    match syn.mode {
        SynthesisMode::Remote => try_remote(syn, creds)
            .unwrap_or_else(|| warn_and_degraded("remote provider unavailable")),
        SynthesisMode::Ollama => {
            try_ollama(syn).unwrap_or_else(|| warn_and_degraded("ollama unavailable"))
        }
        // Legacy: treat Embedded mode same as Auto (Ollama-first)
        SynthesisMode::Embedded | SynthesisMode::Auto => try_remote(syn, creds)
            .or_else(|| try_ollama(syn))
            .unwrap_or_else(|| warn_and_degraded("all synthesis backends unavailable")),
    }
}

fn try_remote(syn: &SynthesisConfig, creds: &CredentialsStore) -> Option<ResolvedBackend> {
    if matches!(
        syn.provider,
        SynthesisProvider::Ollama | SynthesisProvider::Embedded
    ) {
        return None;
    }
    let needs_key = SynthesisConfig::provider_needs_credentials(syn.provider);
    let api_key = creds.api_key_for(syn.provider);
    if needs_key && api_key.is_none() {
        return None;
    }
    let backend = HttpBackend::from_config(
        syn.provider,
        syn,
        api_key,
        format!("{}/{}", syn.provider.slug(), syn.model),
    );
    backend.health_check().ok()?;
    Some(ResolvedBackend::Http(backend))
}

fn try_ollama(syn: &SynthesisConfig) -> Option<ResolvedBackend> {
    if !ollama::is_reachable(syn) {
        return None;
    }
    let backend = ollama::make_backend(syn);
    backend.health_check().ok()?;
    Some(ResolvedBackend::Http(backend))
}

fn warn_and_degraded(reason: &str) -> ResolvedBackend {
    let mut warned = DEGRADED_WARNED.lock().unwrap();
    if !*warned {
        tracing::warn!(
            "Synthesis running in degraded mode ({reason}) — configure `agent-trace model setup`"
        );
        *warned = true;
    }
    ResolvedBackend::Degraded(DegradedBackend)
}

/// Clear HTTP health caches (e.g. after `model ensure` / config changes).
pub fn invalidate_resolve_caches() {
    HttpBackend::invalidate_health_cache();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GlobalConfig, PollingConfig, StoreConfig, StoreInfo};

    #[test]
    fn auto_mode_falls_back_to_degraded_without_backends() {
        invalidate_resolve_caches();
        let merged = MergedConfig::merge(
            GlobalConfig {
                synthesis: SynthesisConfig::for_unit_tests_degraded(),
                ..Default::default()
            },
            StoreConfig {
                store: StoreInfo::new("t".into()),
                llm: None,
                synthesis: None,
                polling: PollingConfig::default(),
            },
        );
        let creds = CredentialsStore::default();
        let resolved = resolve(&merged, &creds);
        assert!(resolved.info().degraded);
        let again = resolve(&merged, &creds);
        assert!(again.info().degraded);
    }

    #[test]
    fn auto_prefers_remote_when_creds_present() {
        use crate::config::{SynthesisConfig, SynthesisMode, SynthesisProvider};
        invalidate_resolve_caches();
        let mut creds = CredentialsStore::default();
        creds.set_key(SynthesisProvider::Openai, "sk-fake-key".into());
        let merged = MergedConfig::merge(
            GlobalConfig {
                synthesis: SynthesisConfig {
                    mode: SynthesisMode::Auto,
                    provider: SynthesisProvider::Openai,
                    model: "gpt-4o-mini".into(),
                    ..Default::default()
                },
                ..Default::default()
            },
            StoreConfig {
                store: StoreInfo::new("t".into()),
                llm: None,
                synthesis: None,
                polling: PollingConfig::default(),
            },
        );
        let resolved = resolve(&merged, &creds);
        let _ = resolved.info();
    }
}
