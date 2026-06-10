use crate::llm::trace_insights::TraceDocument;

/// Unified synthesis interface for trace artifacts (running summary, context, logs, recaps).
pub trait SynthesisEngine: Send + Sync {
    fn summarize_change(&self, path: &str, doc_type: &str, diff: &str) -> Result<String, String>;
    fn synthesize_context(
        &self,
        documents: &[TraceDocument],
        updates: &[String],
    ) -> Result<String, String>;
    fn update_running_summary(
        &self,
        previous: &str,
        events: &str,
        plan: &str,
    ) -> Result<String, String>;
    fn summarize_session(&self, session_id: &str, events: &[String]) -> Result<String, String>;
    fn backend_label(&self) -> &str;
}

/// Mechanical fallback when no remote, Ollama, or embedded backend is available.
pub struct DegradedBackend;

impl DegradedBackend {
    fn line_stats_summary(path: &str, diff: &str) -> String {
        let added = diff.lines().filter(|l| l.starts_with('+')).count();
        let removed = diff.lines().filter(|l| l.starts_with('-')).count();
        format!("{path}: +{added} lines, -{removed} lines.")
    }
}

impl SynthesisEngine for DegradedBackend {
    fn summarize_change(&self, path: &str, _doc_type: &str, diff: &str) -> Result<String, String> {
        Ok(Self::line_stats_summary(path, diff))
    }

    fn synthesize_context(
        &self,
        documents: &[TraceDocument],
        _updates: &[String],
    ) -> Result<String, String> {
        let mut out =
            String::from("# Project Context\n\n*(Synthesis degraded — configure `agent-trace model setup`)*\n\n## Documents\n\n");
        for doc in documents {
            out.push_str(&format!("- `{}` [{}]\n", doc.path, doc.doc_type));
        }
        Ok(out)
    }

    fn update_running_summary(
        &self,
        _previous: &str,
        events: &str,
        _plan: &str,
    ) -> Result<String, String> {
        let mut out = String::from(
            "# Running Summary\n\n*(Synthesis degraded — configure `agent-trace model setup`)*\n\n## Recent Activity\n\n",
        );
        for line in events.lines().take(15) {
            out.push_str(&format!("- {line}\n"));
        }
        Ok(out)
    }

    fn summarize_session(&self, session_id: &str, events: &[String]) -> Result<String, String> {
        Ok(format!(
            "Session {session_id}: {} event(s) recorded (degraded summary).",
            events.len()
        ))
    }

    fn backend_label(&self) -> &str {
        "degraded"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DocType;

    #[test]
    fn degraded_summarize_change_counts_lines() {
        let b = DegradedBackend;
        let s = b.summarize_change("a.md", "plan", "+a\n+b\n-c").unwrap();
        assert!(s.contains("+2"));
        assert!(s.contains("-1"));
    }

    #[test]
    fn degraded_context_lists_documents() {
        let b = DegradedBackend;
        let docs = vec![TraceDocument {
            path: "plan.md".into(),
            doc_type: DocType::Plan,
            content_snippet: "goals".into(),
        }];
        let out = b.synthesize_context(&docs, &[]).unwrap();
        assert!(out.contains("plan.md"));
    }
}
