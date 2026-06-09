pub mod backend;
pub mod candle;
pub mod candle_backend;
pub mod llama_cpp;
pub mod prompts;
pub mod providers;
pub mod synthesis_engine;
pub mod trace_insights;

use crate::types::DocType;
use anyhow::Result;

// ── Types ─────────────────────────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Classification {
    pub doc_type: DocType,
    pub confidence: f32,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ParsedCommand {
    pub command: String,
    pub args: std::collections::HashMap<String, String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct DocSummary {
    pub path: String,
    pub doc_type: DocType,
    pub content_snippet: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum LlmRequest {
    Classify {
        id: u64,
        content: String,
    },
    SummarizeChange {
        id: u64,
        path: String,
        doc_type: String,
        diff: String,
    },
    ParseCommand {
        id: u64,
        input: String,
        manifest_summary: String,
    },
    SynthesizeContext {
        id: u64,
        documents: Vec<DocSummary>,
        updates: Vec<String>,
    },
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum LlmResponse {
    Classification {
        id: u64,
        result: Result<Classification, String>,
    },
    Summary {
        id: u64,
        result: Result<String, String>,
    },
    ParsedCommand {
        id: u64,
        result: Result<ParsedCommand, String>,
    },
    Context {
        id: u64,
        result: Result<String, String>,
    },
}

// ── Trait ─────────────────────────────────────────────────────────────────────

/// Immutable inference interface. Implement this for `NoLlm` and for wrappers
/// that don't need a mutable KV cache (e.g. stateless REST-backed engines).
/// `CandleLlm` uses a separate `CandleLlmMut` + `spawn_candle_task` instead.
pub trait LlmEngine: Send + Sync {
    fn classify(&self, content: &str) -> Result<Classification>;
    fn summarize_change(&self, path: &str, doc_type: &str, diff: &str) -> Result<String>;
    fn parse_command(&self, input: &str, manifest_summary: &str) -> Result<ParsedCommand>;
    fn synthesize_context(&self, documents: &[DocSummary], updates: &[String]) -> Result<String>;
    fn is_loaded(&self) -> bool;
}

// ── NoLlm Fallback ────────────────────────────────────────────────────────────

pub struct NoLlm;

impl LlmEngine for NoLlm {
    fn classify(&self, _content: &str) -> Result<Classification> {
        Ok(Classification {
            doc_type: DocType::Scratch,
            confidence: 0.0,
        })
    }

    fn summarize_change(&self, path: &str, _doc_type: &str, diff: &str) -> Result<String> {
        let added = diff.lines().filter(|l| l.starts_with('+')).count();
        let removed = diff.lines().filter(|l| l.starts_with('-')).count();
        Ok(format!("{path}: +{added} lines, -{removed} lines."))
    }

    fn parse_command(&self, _input: &str, _manifest_summary: &str) -> Result<ParsedCommand> {
        Ok(ParsedCommand {
            command: "unknown".to_string(),
            args: std::collections::HashMap::new(),
        })
    }

    fn synthesize_context(&self, documents: &[DocSummary], _updates: &[String]) -> Result<String> {
        let mut out =
            String::from("# Project Context\n\n*(Generated without LLM)*\n\n## Documents\n\n");
        for doc in documents {
            out.push_str(&format!("- `{}` [{}]\n", doc.path, doc.doc_type));
        }
        Ok(out)
    }

    fn is_loaded(&self) -> bool {
        false
    }
}

// ── Async Task (immutable engine: NoLlm or future REST backend) ───────────────

/// Spawn a background task that services `LlmRequest`s using an immutable engine.
/// For `CandleLlmMut` (which requires `&mut self` for KV-cache inference),
/// use `spawn_candle_task` instead.
pub fn spawn_llm_task(
    engine: std::sync::Arc<dyn LlmEngine>,
    mut request_rx: tokio::sync::mpsc::Receiver<LlmRequest>,
    response_tx: tokio::sync::mpsc::Sender<LlmResponse>,
) {
    tokio::spawn(async move {
        while let Some(request) = request_rx.recv().await {
            let engine = engine.clone();
            let tx = response_tx.clone();

            tokio::task::spawn_blocking(move || {
                let response = match request {
                    LlmRequest::Classify { id, content } => {
                        let result = engine.classify(&content).map_err(|e| e.to_string());
                        LlmResponse::Classification { id, result }
                    }
                    LlmRequest::SummarizeChange {
                        id,
                        path,
                        doc_type,
                        diff,
                    } => {
                        let result = engine
                            .summarize_change(&path, &doc_type, &diff)
                            .map_err(|e| e.to_string());
                        LlmResponse::Summary { id, result }
                    }
                    LlmRequest::ParseCommand {
                        id,
                        input,
                        manifest_summary,
                    } => {
                        let result = engine
                            .parse_command(&input, &manifest_summary)
                            .map_err(|e| e.to_string());
                        LlmResponse::ParsedCommand { id, result }
                    }
                    LlmRequest::SynthesizeContext {
                        id,
                        documents,
                        updates,
                    } => {
                        let result = engine
                            .synthesize_context(&documents, &updates)
                            .map_err(|e| e.to_string());
                        LlmResponse::Context { id, result }
                    }
                };
                let _ = tx.blocking_send(response);
            });
        }
    });
}

// ── Async Task (CandleLlmMut — requires mutable KV cache) ────────────────────

/// Spawn a background task for `CandleLlmMut`. The model is owned exclusively
/// by the task thread; all requests are serialized through a mutex so the KV
/// cache is never concurrently accessed.
#[cfg(feature = "llm")]
pub fn spawn_candle_task(
    model: candle::CandleLlmMut,
    mut request_rx: tokio::sync::mpsc::Receiver<LlmRequest>,
    response_tx: tokio::sync::mpsc::Sender<LlmResponse>,
) {
    use std::sync::{Arc, Mutex};
    let model = Arc::new(Mutex::new(model));

    tokio::spawn(async move {
        while let Some(request) = request_rx.recv().await {
            let model = model.clone();
            let tx = response_tx.clone();

            tokio::task::spawn_blocking(move || {
                let mut m = model.lock().unwrap();
                let response = match request {
                    LlmRequest::Classify { id, content } => {
                        let result = m.classify(&content).map_err(|e| e.to_string());
                        LlmResponse::Classification { id, result }
                    }
                    LlmRequest::SummarizeChange {
                        id,
                        path,
                        doc_type,
                        diff,
                    } => {
                        let result = m
                            .summarize_change(&path, &doc_type, &diff)
                            .map_err(|e| e.to_string());
                        LlmResponse::Summary { id, result }
                    }
                    LlmRequest::ParseCommand {
                        id,
                        input,
                        manifest_summary,
                    } => {
                        let result = m
                            .parse_command(&input, &manifest_summary)
                            .map_err(|e| e.to_string());
                        LlmResponse::ParsedCommand { id, result }
                    }
                    LlmRequest::SynthesizeContext {
                        id,
                        documents,
                        updates,
                    } => {
                        let result = m
                            .synthesize_context(&documents, &updates)
                            .map_err(|e| e.to_string());
                        LlmResponse::Context { id, result }
                    }
                };
                let _ = tx.blocking_send(response);
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_llm_classify() {
        let result = NoLlm.classify("some content").unwrap();
        assert_eq!(result.doc_type, DocType::Scratch);
    }

    #[test]
    fn test_no_llm_summarize() {
        let diff = "+new line\n-old line\n unchanged";
        let result = NoLlm.summarize_change("prd.md", "plan", diff).unwrap();
        assert!(result.contains("prd.md"));
        assert!(result.contains("+1"));
        assert!(result.contains("-1"));
    }

    #[test]
    fn test_no_llm_parse_command() {
        let result = NoLlm.parse_command("show me all plans", "").unwrap();
        assert_eq!(result.command, "unknown");
    }

    #[test]
    fn test_no_llm_synthesize_context() {
        let docs = vec![DocSummary {
            path: "prd.md".into(),
            doc_type: DocType::Plan,
            content_snippet: "Product requirements".into(),
        }];
        let result = NoLlm.synthesize_context(&docs, &[]).unwrap();
        assert!(result.contains("prd.md"));
    }

    #[test]
    fn test_no_llm_is_not_loaded() {
        assert!(!NoLlm.is_loaded());
    }

    #[tokio::test]
    async fn test_llm_task_classification() {
        let engine = std::sync::Arc::new(NoLlm);
        let (req_tx, req_rx) = tokio::sync::mpsc::channel(10);
        let (res_tx, mut res_rx) = tokio::sync::mpsc::channel(10);

        spawn_llm_task(engine, req_rx, res_tx);

        req_tx
            .send(LlmRequest::Classify {
                id: 42,
                content: "hello".into(),
            })
            .await
            .unwrap();

        let response = tokio::time::timeout(std::time::Duration::from_secs(2), res_rx.recv())
            .await
            .unwrap()
            .unwrap();

        match response {
            LlmResponse::Classification { id, result } => {
                assert_eq!(id, 42);
                assert_eq!(result.unwrap().doc_type, DocType::Scratch);
            }
            _ => panic!("Wrong response type"),
        }
    }

    #[tokio::test]
    async fn test_llm_task_all_variants() {
        let engine = std::sync::Arc::new(NoLlm);
        let (req_tx, req_rx) = tokio::sync::mpsc::channel(10);
        let (res_tx, mut res_rx) = tokio::sync::mpsc::channel(10);

        spawn_llm_task(engine, req_rx, res_tx);

        req_tx
            .send(LlmRequest::SummarizeChange {
                id: 1,
                path: "f.md".into(),
                doc_type: "plan".into(),
                diff: "+a\n-b".into(),
            })
            .await
            .unwrap();
        req_tx
            .send(LlmRequest::ParseCommand {
                id: 2,
                input: "list all plans".into(),
                manifest_summary: "".into(),
            })
            .await
            .unwrap();
        req_tx
            .send(LlmRequest::SynthesizeContext {
                id: 3,
                documents: vec![],
                updates: vec![],
            })
            .await
            .unwrap();

        let mut ids_seen = std::collections::HashSet::new();
        for _ in 0..3 {
            let r = tokio::time::timeout(std::time::Duration::from_secs(2), res_rx.recv())
                .await
                .unwrap()
                .unwrap();
            let id = match &r {
                LlmResponse::Summary { id, .. } => *id,
                LlmResponse::ParsedCommand { id, .. } => *id,
                LlmResponse::Context { id, .. } => *id,
                _ => panic!("unexpected variant"),
            };
            ids_seen.insert(id);
        }
        assert_eq!(ids_seen, [1, 2, 3].into());
    }
}
