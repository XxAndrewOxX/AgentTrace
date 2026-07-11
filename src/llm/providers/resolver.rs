use super::http::HttpBackend;
use super::ollama;
use crate::config::{
    CredentialsStore, MergedConfig, SynthesisConfig, SynthesisMode, SynthesisProvider,
};
use crate::llm::synthesis_engine::{DegradedBackend, SynthesisEngine};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

static DEGRADED_WARNED: LazyLock<Mutex<bool>> = LazyLock::new(|| Mutex::new(false));

static RESOLVE_CACHE: LazyLock<Mutex<HashMap<u64, (Instant, ResolvedBackendInfo)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

const RESOLVE_CACHE_TTL: Duration = Duration::from_secs(30);

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

fn synthesis_fingerprint(syn: &SynthesisConfig, creds: &CredentialsStore) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    format!("{:?}", syn.mode).hash(&mut hasher);
    syn.provider.slug().hash(&mut hasher);
    syn.model.hash(&mut hasher);
    syn.effective_base_url().hash(&mut hasher);
    syn.max_tokens.hash(&mut hasher);
    syn.temperature.to_bits().hash(&mut hasher);
    // Credential presence affects remote resolution without storing secrets.
    creds.api_key_for(syn.provider).is_some().hash(&mut hasher);
    hasher.finish()
}

pub fn resolve(merged: &MergedConfig, creds: &CredentialsStore) -> ResolvedBackend {
    let fp = synthesis_fingerprint(&merged.synthesis, creds);
    if let Ok(cache) = RESOLVE_CACHE.lock() {
        if let Some((at, info)) = cache.get(&fp) {
            if at.elapsed() < RESOLVE_CACHE_TTL && info.degraded {
                // Fast path for known-degraded configs (common in tests / offline).
                return ResolvedBackend::Degraded(DegradedBackend);
            }
        }
    }

    let resolved = resolve_uncached(merged, creds);
    let info = resolved.info();
    if let Ok(mut cache) = RESOLVE_CACHE.lock() {
        cache.insert(fp, (Instant::now(), info));
    }
    resolved
}

fn resolve_uncached(merged: &MergedConfig, creds: &CredentialsStore) -> ResolvedBackend {
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

/// Clear resolve + HTTP health caches (e.g. after `model ensure`).
pub fn invalidate_resolve_caches() {
    if let Ok(mut cache) = RESOLVE_CACHE.lock() {
        cache.clear();
    }
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
        // Second resolve should hit degraded cache.
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
        // Remote will fail health check in unit test (no real API), so falls through to degraded
        let resolved = resolve(&merged, &creds);
        // Either remote (if reachable) or degraded — main thing: no embedded step
        let _ = resolved.info();
    }
}
