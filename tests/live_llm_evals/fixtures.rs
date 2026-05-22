use agent_trace::llm::trace_insights::TraceDocument;
use agent_trace::types::DocType;

pub struct EvalFixture {
    pub id: &'static str,
    pub summary_path: &'static str,
    pub summary_doc_type: DocType,
    pub summary_diff: &'static str,
    pub context_docs: Vec<TraceDocument>,
    pub context_updates: Vec<String>,
    pub session_id: &'static str,
    pub session_events: Vec<String>,
}

pub fn default_fixtures() -> Vec<EvalFixture> {
    vec![EvalFixture {
        id: "fx001_plan_iteration",
        summary_path: "plan.md",
        summary_doc_type: DocType::Plan,
        summary_diff: "+ Added rollout checklist\n- Removed placeholder timeline\n",
        context_docs: vec![
            TraceDocument {
                path: "plan.md".into(),
                doc_type: DocType::Plan,
                content_snippet: "# Plan\n\nRollout in 3 phases.\n".into(),
            },
            TraceDocument {
                path: "reference/api.md".into(),
                doc_type: DocType::Reference,
                content_snippet: "# API\n\nPOST /v1/jobs\n".into(),
            },
        ],
        context_updates: vec!["We chose staged rollout".into()],
        session_id: "ses-fx001",
        session_events: vec![
            "agent wrote plan.md".into(),
            "system refreshed context.md".into(),
            "agent completed milestone 1".into(),
        ],
    }]
}
