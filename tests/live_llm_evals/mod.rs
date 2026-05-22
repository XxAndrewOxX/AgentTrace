pub mod fixtures;
pub mod report;
pub mod runner;

pub fn llm_evals_enabled() -> bool {
    std::env::var("AGENT_TRACE_LIVE_LLM_EVALS").as_deref() == Ok("1")
}
