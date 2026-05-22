use crate::config::{LlmConfig, StoreConfig};
use crate::types::DocType;
use std::path::Path;
use thiserror::Error;

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
}

#[derive(Debug, Clone)]
pub enum TraceInsightsResponse {
    ChangeSummary(String),
    ContextDocument(String),
    SessionSummary(String),
}

#[derive(Debug, Error)]
pub enum TraceInsightsError {
    #[error("timeout while running trace_insights request")]
    Timeout,
    #[error("model unavailable: {0}")]
    ModelUnavailable(String),
    #[error("invalid output: {0}")]
    InvalidOutput(String),
    #[error("backend failure: {0}")]
    BackendFailure(String),
}

pub struct TraceInsightsFacade {
    backend: super::llama_cpp::LlamaCppBackend,
}

impl TraceInsightsFacade {
    pub fn from_llm_config(cfg: &LlmConfig) -> Result<Option<Self>, TraceInsightsError> {
        if cfg.model_path.is_none() {
            return Ok(None);
        }
        let backend = super::llama_cpp::LlamaCppBackend::from_config(cfg)
            .map_err(TraceInsightsError::ModelUnavailable)?;
        Ok(Some(Self { backend }))
    }

    pub fn from_store_root(store_root: &Path) -> Result<Option<Self>, TraceInsightsError> {
        let store_cfg = match StoreConfig::load(store_root) {
            Ok(cfg) => cfg,
            Err(_) => return Ok(None),
        };
        let llm_cfg = store_cfg.llm.unwrap_or_default();
        match Self::from_llm_config(&llm_cfg) {
            Ok(v) => Ok(v),
            Err(TraceInsightsError::ModelUnavailable(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn execute(
        &self,
        request: TraceInsightsRequest,
    ) -> Result<TraceInsightsResponse, TraceInsightsError> {
        match request {
            TraceInsightsRequest::SummarizeChange {
                path,
                doc_type,
                diff,
            } => {
                let text = self
                    .backend
                    .summarize_change(&path, &doc_type.to_string(), &diff)
                    .map_err(TraceInsightsError::BackendFailure)?;
                validate_non_empty(&text)?;
                Ok(TraceInsightsResponse::ChangeSummary(text))
            }
            TraceInsightsRequest::SynthesizeContext { documents, updates } => {
                let text = self
                    .backend
                    .synthesize_context(&documents, &updates)
                    .map_err(TraceInsightsError::BackendFailure)?;
                validate_non_empty(&text)?;
                Ok(TraceInsightsResponse::ContextDocument(text))
            }
            TraceInsightsRequest::SummarizeSession { session_id, events } => {
                let text = self
                    .backend
                    .summarize_session(&session_id, &events)
                    .map_err(TraceInsightsError::BackendFailure)?;
                validate_non_empty(&text)?;
                Ok(TraceInsightsResponse::SessionSummary(text))
            }
        }
    }

    pub fn summarize_change(
        &self,
        path: &Path,
        doc_type: &DocType,
        diff: &str,
    ) -> Result<String, TraceInsightsError> {
        let request = TraceInsightsRequest::SummarizeChange {
            path: path.display().to_string(),
            doc_type: doc_type.clone(),
            diff: diff.to_string(),
        };
        match self.execute(request)? {
            TraceInsightsResponse::ChangeSummary(v) => Ok(v),
            _ => Err(TraceInsightsError::InvalidOutput(
                "expected ChangeSummary response".into(),
            )),
        }
    }

    pub fn synthesize_context(
        &self,
        documents: &[TraceDocument],
        updates: &[String],
    ) -> Result<String, TraceInsightsError> {
        let request = TraceInsightsRequest::SynthesizeContext {
            documents: documents.to_vec(),
            updates: updates.to_vec(),
        };
        match self.execute(request)? {
            TraceInsightsResponse::ContextDocument(v) => Ok(v),
            _ => Err(TraceInsightsError::InvalidOutput(
                "expected ContextDocument response".into(),
            )),
        }
    }

    pub fn summarize_session(
        &self,
        session_id: &str,
        events: &[String],
    ) -> Result<String, TraceInsightsError> {
        let request = TraceInsightsRequest::SummarizeSession {
            session_id: session_id.to_string(),
            events: events.to_vec(),
        };
        match self.execute(request)? {
            TraceInsightsResponse::SessionSummary(v) => Ok(v),
            _ => Err(TraceInsightsError::InvalidOutput(
                "expected SessionSummary response".into(),
            )),
        }
    }
}

fn validate_non_empty(text: &str) -> Result<(), TraceInsightsError> {
    if text.trim().is_empty() {
        return Err(TraceInsightsError::InvalidOutput(
            "backend returned empty output".into(),
        ));
    }
    Ok(())
}
