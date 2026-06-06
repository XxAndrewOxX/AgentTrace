use super::trace_insights::TraceDocument;

/// Backend trait for trace insight operations (summarize, synthesize, session recap).
pub trait TraceInsightsBackend: Send + Sync {
    fn summarize_change(&self, path: &str, doc_type: &str, diff: &str) -> Result<String, String>;

    fn synthesize_context(
        &self,
        documents: &[TraceDocument],
        updates: &[String],
    ) -> Result<String, String>;

    fn summarize_session(&self, session_id: &str, events: &[String]) -> Result<String, String>;

    fn update_running_summary(
        &self,
        previous_summary: &str,
        new_events: &str,
        plan_snippet: &str,
    ) -> Result<String, String>;
}

/// Stub backend that always fails — triggers template fallback in callers.
pub struct NoTraceBackend;

impl TraceInsightsBackend for NoTraceBackend {
    fn summarize_change(
        &self,
        _path: &str,
        _doc_type: &str,
        _diff: &str,
    ) -> Result<String, String> {
        Err("no trace insights backend available".into())
    }

    fn synthesize_context(
        &self,
        _documents: &[TraceDocument],
        _updates: &[String],
    ) -> Result<String, String> {
        Err("no trace insights backend available".into())
    }

    fn summarize_session(&self, _session_id: &str, _events: &[String]) -> Result<String, String> {
        Err("no trace insights backend available".into())
    }

    fn update_running_summary(
        &self,
        _previous_summary: &str,
        _new_events: &str,
        _plan_snippet: &str,
    ) -> Result<String, String> {
        Err("no trace insights backend available".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_trace_backend_returns_err() {
        let b = NoTraceBackend;
        assert!(b.summarize_change("a.md", "plan", "+1").is_err());
        assert!(b.update_running_summary("", "", "").is_err());
    }
}
