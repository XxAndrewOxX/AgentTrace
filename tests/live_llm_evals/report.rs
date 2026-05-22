use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalCaseResult {
    pub fixture_id: String,
    pub success: bool,
    pub error: Option<String>,
    pub summary_latency_ms: u128,
    pub context_latency_ms: u128,
    pub recap_latency_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEvalReport {
    pub model_path: String,
    pub cases: Vec<EvalCaseResult>,
}

impl ModelEvalReport {
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("# LLM Eval Report: `{}`\n\n", self.model_path));
        out.push_str("| Fixture | Success | Summary ms | Context ms | Recap ms | Error |\n");
        out.push_str("|---|---:|---:|---:|---:|---|\n");
        for c in &self.cases {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} |\n",
                c.fixture_id,
                if c.success { "yes" } else { "no" },
                c.summary_latency_ms,
                c.context_latency_ms,
                c.recap_latency_ms,
                c.error.as_deref().unwrap_or("")
            ));
        }
        out
    }
}
