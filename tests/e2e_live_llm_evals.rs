#[path = "live_llm_evals/mod.rs"]
mod live_llm_evals;

use live_llm_evals::llm_evals_enabled;
use live_llm_evals::runner::{model_paths_from_env, run_model_eval};

#[test]
#[ignore]
fn le001_model_barometer_outputs_reports() {
    if !llm_evals_enabled() {
        return;
    }

    let models = model_paths_from_env();
    assert!(
        !models.is_empty(),
        "AGENT_TRACE_EVAL_MODELS must contain at least one ';'-separated model path"
    );

    for model in models {
        let report = run_model_eval(&model);
        let json = serde_json::to_string_pretty(&report).expect("serialize report json");
        let md = report.to_markdown();
        assert!(!json.is_empty());
        assert!(md.contains("LLM Eval Report"));
    }
}
