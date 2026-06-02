use std::path::{Path, PathBuf};
use std::time::Instant;

use agent_trace::config::LlmConfig;
use agent_trace::llm::trace_insights::TraceInsightsFacade;

use super::fixtures::{default_fixtures, EvalFixture};
use super::report::{EvalCaseResult, ModelEvalReport};

pub fn model_paths_from_env() -> Vec<PathBuf> {
    std::env::var("AGENT_TRACE_EVAL_MODELS")
        .ok()
        .map(|v| {
            v.split(';')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub fn run_model_eval(model_path: &Path) -> ModelEvalReport {
    let cfg = LlmConfig {
        model_path: Some(model_path.to_path_buf()),
        max_tokens: 4096,
        temperature: 0.2,
    };

    let api = match TraceInsightsFacade::from_llm_config(&cfg) {
        Ok(Some(api)) => api,
        Ok(None) => {
            return ModelEvalReport {
                model_path: model_path.display().to_string(),
                cases: vec![EvalCaseResult {
                    fixture_id: "init".into(),
                    success: false,
                    error: Some("llm.model_path missing".into()),
                    summary_latency_ms: 0,
                    context_latency_ms: 0,
                    recap_latency_ms: 0,
                }],
            };
        }
        Err(e) => {
            return ModelEvalReport {
                model_path: model_path.display().to_string(),
                cases: vec![EvalCaseResult {
                    fixture_id: "init".into(),
                    success: false,
                    error: Some(e.to_string()),
                    summary_latency_ms: 0,
                    context_latency_ms: 0,
                    recap_latency_ms: 0,
                }],
            };
        }
    };

    let cases = default_fixtures()
        .into_iter()
        .map(|fx| run_case(&api, fx))
        .collect();

    ModelEvalReport {
        model_path: model_path.display().to_string(),
        cases,
    }
}

fn run_case(api: &TraceInsightsFacade, fx: EvalFixture) -> EvalCaseResult {
    let t0 = Instant::now();
    let summary_res = api.summarize_change(
        &PathBuf::from(fx.summary_path),
        &fx.summary_doc_type,
        fx.summary_diff,
    );
    let summary_ms = t0.elapsed().as_millis();

    let t1 = Instant::now();
    let context_res = api.synthesize_context(&fx.context_docs, &fx.context_updates);
    let context_ms = t1.elapsed().as_millis();

    let t2 = Instant::now();
    let recap_res = api.summarize_session(fx.session_id, &fx.session_events);
    let recap_ms = t2.elapsed().as_millis();

    let mut errors = Vec::new();
    if let Err(e) = summary_res {
        errors.push(format!("summary: {e}"));
    }
    if let Err(e) = context_res {
        errors.push(format!("context: {e}"));
    }
    if let Err(e) = recap_res {
        errors.push(format!("recap: {e}"));
    }

    EvalCaseResult {
        fixture_id: fx.id.to_string(),
        success: errors.is_empty(),
        error: (!errors.is_empty()).then(|| errors.join(" | ")),
        summary_latency_ms: summary_ms,
        context_latency_ms: context_ms,
        recap_latency_ms: recap_ms,
    }
}
