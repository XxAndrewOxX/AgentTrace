use crate::config::{embedded_model_path, LlmConfig, MergedConfig, SynthesisConfig};
#[cfg(feature = "llm")]
use crate::llm::synthesis_engine::SynthesisEngine;
#[cfg(feature = "llm")]
use crate::llm::trace_insights::TraceDocument;
#[cfg(feature = "llm")]
use crate::llm::DocSummary;
use std::path::PathBuf;

/// Embedded GGUF inference via Candle (requires `llm` feature).
pub struct EmbeddedBackend {
    #[cfg(feature = "llm")]
    inner: std::sync::Mutex<crate::llm::candle::CandleLlmMut>,
    label: String,
    model_path: PathBuf,
}

impl EmbeddedBackend {
    pub fn try_from_config(cfg: &SynthesisConfig, llm: &LlmConfig) -> Option<Self> {
        let path = resolve_model_path(cfg, llm)?;
        Self::from_path(path)
    }

    pub fn from_path(path: PathBuf) -> Option<Self> {
        if !path.exists() {
            return None;
        }
        #[cfg(feature = "llm")]
        {
            match crate::llm::candle::CandleLlmMut::from_path(&path) {
                Ok(inner) => Some(Self {
                    inner: std::sync::Mutex::new(inner),
                    label: format!("embedded/{}", path.file_name()?.to_string_lossy()),
                    model_path: path,
                }),
                Err(e) => {
                    tracing::warn!("embedded model load failed: {e}");
                    None
                }
            }
        }
        #[cfg(not(feature = "llm"))]
        {
            let _ = path;
            None
        }
    }

    pub fn model_path(&self) -> &PathBuf {
        &self.model_path
    }

    pub fn backend_label(&self) -> &str {
        &self.label
    }
}

pub fn resolve_model_path(cfg: &SynthesisConfig, llm: &LlmConfig) -> Option<PathBuf> {
    if let Some(path) = &llm.model_path {
        if path.exists() {
            return Some(path.clone());
        }
    }
    let embedded = embedded_model_path(&cfg.fallback.embedded_model);
    if embedded.exists() {
        return Some(embedded);
    }
    None
}

#[cfg(feature = "llm")]
impl SynthesisEngine for EmbeddedBackend {
    fn summarize_change(&self, path: &str, doc_type: &str, diff: &str) -> Result<String, String> {
        let mut inner = self.inner.lock().map_err(|e| e.to_string())?;
        inner
            .summarize_change(path, doc_type, diff)
            .map_err(|e| e.to_string())
    }

    fn synthesize_context(
        &self,
        documents: &[TraceDocument],
        updates: &[String],
    ) -> Result<String, String> {
        let docs: Vec<DocSummary> = documents
            .iter()
            .map(|d| DocSummary {
                path: d.path.clone(),
                doc_type: d.doc_type.clone(),
                content_snippet: d.content_snippet.clone(),
            })
            .collect();
        let mut inner = self.inner.lock().map_err(|e| e.to_string())?;
        inner
            .synthesize_context(&docs, updates)
            .map_err(|e| e.to_string())
    }

    fn update_running_summary(
        &self,
        previous: &str,
        events: &str,
        plan: &str,
    ) -> Result<String, String> {
        let mut inner = self.inner.lock().map_err(|e| e.to_string())?;
        inner
            .update_running_summary(previous, events, plan)
            .map_err(|e| e.to_string())
    }

    fn summarize_session(&self, session_id: &str, events: &[String]) -> Result<String, String> {
        let mut inner = self.inner.lock().map_err(|e| e.to_string())?;
        inner
            .summarize_session(session_id, events)
            .map_err(|e| e.to_string())
    }

    fn backend_label(&self) -> &str {
        &self.label
    }
}

pub fn from_merged_config(merged: &MergedConfig) -> Option<EmbeddedBackend> {
    EmbeddedBackend::try_from_config(&merged.synthesis, &merged.llm)
}
