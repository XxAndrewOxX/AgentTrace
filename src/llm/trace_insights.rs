use super::backend::TraceInsightsBackend;
use super::providers::ollama::EnsureReport;
use super::providers::{resolve, ResolvedBackendInfo};
use super::synthesis_engine::SynthesisEngine;
use crate::config::{CredentialsStore, MergedConfig};
use crate::runtime::allow_degraded_mode;
use crate::types::DocType;
use std::path::Path;
use thiserror::Error;

#[cfg(test)]
use super::backend::NoTraceBackend;

#[derive(Debug, Clone)]
pub struct TraceDocument {
    pub path: String,
    pub doc_type: DocType,
    pub content_snippet: String,
}

#[derive(Debug, Clone)]
pub enum TraceInsightsRequest {
    SummarizeChange {
        path: String,
        doc_type: DocType,
        diff: String,
    },
    SynthesizeContext {
        documents: Vec<TraceDocument>,
        updates: Vec<String>,
    },
    SummarizeSession {
        session_id: String,
        events: Vec<String>,
    },
    UpdateRunningSummary {
        previous_summary: String,
        new_events: String,
        plan_snippet: String,
    },
}

#[derive(Debug, Clone)]
pub enum TraceInsightsResponse {
    ChangeSummary(String),
    ContextDocument(String),
    SessionSummary(String),
    RunningSummary(String),
}

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("timeout while running llm request")]
    Timeout,
    #[error("model unavailable: {0}")]
    ModelUnavailable(String),
    #[error("invalid output: {0}")]
    InvalidOutput(String),
    #[error("backend failure: {0}")]
    BackendFailure(String),
}

/// Backward-compat alias for `LlmError` (deprecated — use `LlmError`).
pub type TraceInsightsError = LlmError;

struct EngineAdapter {
    inner: Box<dyn SynthesisEngine>,
}

impl TraceInsightsBackend for EngineAdapter {
    fn summarize_change(&self, path: &str, doc_type: &str, diff: &str) -> Result<String, String> {
        self.inner.summarize_change(path, doc_type, diff)
    }

    fn synthesize_context(
        &self,
        documents: &[TraceDocument],
        updates: &[String],
    ) -> Result<String, String> {
        self.inner.synthesize_context(documents, updates)
    }

    fn summarize_session(&self, session_id: &str, events: &[String]) -> Result<String, String> {
        self.inner.summarize_session(session_id, events)
    }

    fn update_running_summary(
        &self,
        previous_summary: &str,
        new_events: &str,
        plan_snippet: &str,
    ) -> Result<String, String> {
        self.inner
            .update_running_summary(previous_summary, new_events, plan_snippet)
    }
}

/// Unified LLM facade — the single public entry point for all inference.
///
/// Construct with `Llm::from_store_root` or `Llm::from_merged_config`.
/// All synthesis operations are routed through this type; callers never
/// import from `providers::` directly.
pub struct Llm {
    backend: Box<dyn TraceInsightsBackend>,
    pub backend_label: String,
}

/// Backward-compat alias for `Llm` (deprecated — use `Llm`).
pub type TraceInsightsFacade = Llm;

impl Llm {
    pub fn from_merged_config(merged: &MergedConfig) -> Result<Self, LlmError> {
        let creds = CredentialsStore::load().unwrap_or_default();
        let resolved = resolve(merged, &creds);
        let info = resolved.info();
        if info.degraded && !allow_degraded_mode() {
            return Err(LlmError::ModelUnavailable(
                "Synthesis backend unavailable. Run: agent-trace model ensure".into(),
            ));
        }
        let label = info.label.clone();
        Ok(Self {
            backend: Box::new(EngineAdapter {
                inner: resolved.into_engine(),
            }),
            backend_label: label,
        })
    }

    pub fn from_store_root(store_root: &Path) -> Result<Self, LlmError> {
        let merged = MergedConfig::load(store_root).map_err(|e| {
            LlmError::ModelUnavailable(format!("config load failed: {e}"))
        })?;
        Self::from_merged_config(&merged)
    }

    #[cfg(test)]
    pub fn with_no_backend() -> Self {
        Self {
            backend: Box::new(NoTraceBackend),
            backend_label: "none".into(),
        }
    }

    pub fn is_degraded(&self) -> bool {
        self.backend_label == "degraded"
    }

    /// Return backend info for status/TUI display (from store root path).
    pub fn backend_info(store_root: &Path) -> ResolvedBackendInfo {
        let merged = MergedConfig::load(store_root).unwrap_or_default();
        let creds = CredentialsStore::load().unwrap_or_default();
        resolve(&merged, &creds).info()
    }

    /// Return backend info for status/TUI display (from already-loaded config).
    pub fn backend_info_from_config(merged: &MergedConfig) -> ResolvedBackendInfo {
        let creds = CredentialsStore::load().unwrap_or_default();
        resolve(merged, &creds).info()
    }

    /// Ensure Ollama daemon is running and the configured model is pulled.
    /// Returns `Ok(EnsureReport)` on success, or an error with an actionable message.
    pub fn ensure_ready(merged: &MergedConfig) -> anyhow::Result<EnsureReport> {
        super::providers::ollama::ensure_ready(&merged.synthesis)
    }

    /// Gate check: return backend info or bail if degraded and not allowed.
    pub fn require_backend(store_root: Option<&Path>) -> anyhow::Result<ResolvedBackendInfo> {
        use crate::config::{GlobalConfig, PollingConfig, StoreConfig, StoreInfo};
        let merged = match store_root {
            Some(root) if root.join(".agent-trace").exists() => {
                MergedConfig::load(root).map_err(|e| anyhow::anyhow!("config load: {e}"))?
            }
            _ => {
                let global = GlobalConfig::load()?;
                MergedConfig::merge(
                    global,
                    StoreConfig {
                        store: StoreInfo::new("gate".into()),
                        llm: None,
                        synthesis: None,
                        polling: PollingConfig::default(),
                    },
                )
            }
        };
        let creds = CredentialsStore::load().unwrap_or_default();
        let info = resolve(&merged, &creds).info();
        if info.degraded && !allow_degraded_mode() {
            anyhow::bail!(
                "Synthesis backend unavailable. Run: agent-trace model ensure"
            );
        }
        Ok(info)
    }

    pub fn execute(
        &self,
        request: TraceInsightsRequest,
    ) -> Result<TraceInsightsResponse, LlmError> {
        match request {
            TraceInsightsRequest::SummarizeChange {
                path,
                doc_type,
                diff,
            } => {
                let text = self
                    .backend
                    .summarize_change(&path, &doc_type.to_string(), &diff)
                    .map_err(LlmError::BackendFailure)?;
                validate_non_empty(&text)?;
                Ok(TraceInsightsResponse::ChangeSummary(text))
            }
            TraceInsightsRequest::SynthesizeContext { documents, updates } => {
                let text = self
                    .backend
                    .synthesize_context(&documents, &updates)
                    .map_err(LlmError::BackendFailure)?;
                validate_non_empty(&text)?;
                Ok(TraceInsightsResponse::ContextDocument(text))
            }
            TraceInsightsRequest::SummarizeSession { session_id, events } => {
                let text = self
                    .backend
                    .summarize_session(&session_id, &events)
                    .map_err(LlmError::BackendFailure)?;
                validate_non_empty(&text)?;
                Ok(TraceInsightsResponse::SessionSummary(text))
            }
            TraceInsightsRequest::UpdateRunningSummary {
                previous_summary,
                new_events,
                plan_snippet,
            } => {
                let text = self
                    .backend
                    .update_running_summary(&previous_summary, &new_events, &plan_snippet)
                    .map_err(LlmError::BackendFailure)?;
                validate_non_empty(&text)?;
                Ok(TraceInsightsResponse::RunningSummary(text))
            }
        }
    }

    pub fn summarize_change(
        &self,
        path: &Path,
        doc_type: &DocType,
        diff: &str,
    ) -> Result<String, LlmError> {
        let request = TraceInsightsRequest::SummarizeChange {
            path: path.display().to_string(),
            doc_type: doc_type.clone(),
            diff: diff.to_string(),
        };
        match self.execute(request)? {
            TraceInsightsResponse::ChangeSummary(v) => Ok(v),
            _ => Err(LlmError::InvalidOutput(
                "expected ChangeSummary response".into(),
            )),
        }
    }

    pub fn synthesize_context(
        &self,
        documents: &[TraceDocument],
        updates: &[String],
    ) -> Result<String, LlmError> {
        let request = TraceInsightsRequest::SynthesizeContext {
            documents: documents.to_vec(),
            updates: updates.to_vec(),
        };
        match self.execute(request)? {
            TraceInsightsResponse::ContextDocument(v) => Ok(v),
            _ => Err(LlmError::InvalidOutput(
                "expected ContextDocument response".into(),
            )),
        }
    }

    pub fn summarize_session(
        &self,
        session_id: &str,
        events: &[String],
    ) -> Result<String, LlmError> {
        let request = TraceInsightsRequest::SummarizeSession {
            session_id: session_id.to_string(),
            events: events.to_vec(),
        };
        match self.execute(request)? {
            TraceInsightsResponse::SessionSummary(v) => Ok(v),
            _ => Err(LlmError::InvalidOutput(
                "expected SessionSummary response".into(),
            )),
        }
    }

    pub fn update_running_summary(
        &self,
        previous_summary: &str,
        new_events: &str,
        plan_snippet: &str,
    ) -> Result<String, LlmError> {
        let request = TraceInsightsRequest::UpdateRunningSummary {
            previous_summary: previous_summary.to_string(),
            new_events: new_events.to_string(),
            plan_snippet: plan_snippet.to_string(),
        };
        match self.execute(request)? {
            TraceInsightsResponse::RunningSummary(v) => Ok(v),
            _ => Err(LlmError::InvalidOutput(
                "expected RunningSummary response".into(),
            )),
        }
    }
}

fn validate_non_empty(text: &str) -> Result<(), LlmError> {
    if text.trim().is_empty() {
        return Err(LlmError::InvalidOutput(
            "backend returned empty output".into(),
        ));
    }
    Ok(())
}
