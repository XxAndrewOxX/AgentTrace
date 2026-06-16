pub mod backend;
pub mod prompts;
pub(crate) mod providers;
pub mod synthesis_engine;
pub mod trace_insights;

// ── Public API re-exports ─────────────────────────────────────────────────────
pub use providers::resolver::ResolvedBackendInfo;
pub use trace_insights::{Llm, LlmError, TraceDocument, TraceInsightsError, TraceInsightsFacade};

use crate::types::DocType;

// ── Types (used by HTTP backend / future TUI NL) ─────────────────────────────

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
