pub mod backend;
pub mod prompts;
pub(crate) mod providers;
pub mod synthesis_engine;
pub mod trace_insights;

// ── Public API re-exports ─────────────────────────────────────────────────────
pub use providers::resolver::ResolvedBackendInfo;
pub use trace_insights::{Llm, LlmError, TraceDocument, TraceInsightsError, TraceInsightsFacade};
