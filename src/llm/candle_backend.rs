use crate::config::MergedConfig;

#[cfg(feature = "llm")]
use super::backend::TraceInsightsBackend;
#[cfg(feature = "llm")]
use super::candle::CandleLlmMut;
#[cfg(feature = "llm")]
use super::trace_insights::TraceDocument;
#[cfg(feature = "llm")]
use super::DocSummary;
#[cfg(feature = "llm")]
use std::sync::Mutex;

/// Candle GGUF backend for trace insights (requires `--features llm`).
#[cfg(feature = "llm")]
pub struct CandleTraceBackend {
    inner: Mutex<CandleLlmMut>,
}

#[cfg(feature = "llm")]
impl CandleTraceBackend {
    pub fn from_merged_config(cfg: &MergedConfig) -> Result<Option<Self>, String> {
        let Some(model_path) = cfg.llm.model_path.clone() else {
            return Ok(None);
        };
        if !model_path.exists() {
            return Err(format!(
                "LLM model path not found: {}",
                model_path.display()
            ));
        }
        let mut_model = CandleLlmMut::from_path(&model_path)
            .map_err(|e| format!("failed to load Candle model: {e}"))?;
        Ok(Some(Self {
            inner: Mutex::new(mut_model),
        }))
    }
}

#[cfg(feature = "llm")]
impl TraceInsightsBackend for CandleTraceBackend {
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

    fn summarize_session(&self, session_id: &str, events: &[String]) -> Result<String, String> {
        let mut inner = self.inner.lock().map_err(|e| e.to_string())?;
        inner
            .summarize_session(session_id, events)
            .map_err(|e| e.to_string())
    }

    fn update_running_summary(
        &self,
        previous_summary: &str,
        new_events: &str,
        plan_snippet: &str,
    ) -> Result<String, String> {
        let mut inner = self.inner.lock().map_err(|e| e.to_string())?;
        inner
            .update_running_summary(previous_summary, new_events, plan_snippet)
            .map_err(|e| e.to_string())
    }
}

#[cfg(not(feature = "llm"))]
impl CandleTraceBackend {
    pub fn from_merged_config(cfg: &MergedConfig) -> Result<Option<Self>, String> {
        if cfg.llm.model_path.is_some() {
            Err("LLM feature not compiled in — rebuild with `--features llm`".into())
        } else {
            Ok(None)
        }
    }
}

/// Placeholder type when `llm` feature is disabled.
#[cfg(not(feature = "llm"))]
pub struct CandleTraceBackend;
