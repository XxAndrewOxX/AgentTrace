use super::embedded::EmbeddedBackend;
use super::http::HttpBackend;
use super::ollama;
use crate::config::{
    CredentialsStore, MergedConfig, SynthesisConfig, SynthesisMode, SynthesisProvider,
};
use crate::llm::synthesis_engine::{DegradedBackend, SynthesisEngine};
use std::sync::{LazyLock, Mutex};

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
    Embedded(EmbeddedBackend),
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
            Self::Embedded(b) => ResolvedBackendInfo {
                label: b.backend_label().to_string(),
                provider: SynthesisProvider::Embedded,
                model: b
                    .model_path()
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "embedded".into()),
                degraded: false,
            },
            Self::Degraded(_) => ResolvedBackendInfo {
                label: "degraded".into(),
                provider: SynthesisProvider::Embedded,
                model: "mechanical".into(),
                degraded: true,
            },
        }
    }

    pub fn into_engine(self) -> Box<dyn SynthesisEngine> {
        match self {
            Self::Http(b) => Box::new(b),
            #[cfg(feature = "llm")]
            Self::Embedded(b) => Box::new(b),
            #[cfg(not(feature = "llm"))]
            Self::Embedded(_) => Box::new(DegradedBackend),
            Self::Degraded(b) => Box::new(b),
        }
    }
}

pub fn resolve(merged: &MergedConfig, creds: &CredentialsStore) -> ResolvedBackend {
    let syn = &merged.synthesis;
    match syn.mode {
        SynthesisMode::Remote => try_remote(syn, creds)
            .unwrap_or_else(|| warn_and_degraded("remote provider unavailable")),
        SynthesisMode::Ollama => {
            try_ollama(syn).unwrap_or_else(|| warn_and_degraded("ollama unavailable"))
        }
        SynthesisMode::Embedded => {
            try_embedded(merged).unwrap_or_else(|| warn_and_degraded("embedded model unavailable"))
        }
        SynthesisMode::Auto => try_remote(syn, creds)
            .or_else(|| try_ollama(syn))
            .or_else(|| try_embedded(merged))
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

fn try_embedded(merged: &MergedConfig) -> Option<ResolvedBackend> {
    EmbeddedBackend::try_from_config(&merged.synthesis, &merged.llm).map(ResolvedBackend::Embedded)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GlobalConfig, PollingConfig, StoreConfig, StoreInfo};

    #[test]
    fn auto_mode_falls_back_to_degraded_without_backends() {
        let merged = MergedConfig::merge(
            GlobalConfig::default(),
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
    }
}
