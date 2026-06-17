use crate::llm::trace_insights::TraceDocument;
use crate::types::DocType;

const MAX_SNIPPET_CHARS: usize = 2000;

#[derive(Debug, Clone)]
pub struct DocSummary {
    pub path: String,
    pub doc_type: DocType,
    pub content_snippet: String,
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}

pub fn summarize_change_prompt(path: &str, doc_type: &str, diff: &str) -> String {
    format!(
        "Summarize this diff of a {doc_type} document '{path}' in one sentence (max 20 words).\n\n{}",
        truncate(diff, MAX_SNIPPET_CHARS)
    )
}

pub fn synthesize_context_prompt(documents: &[DocSummary], updates: &[String]) -> String {
    let mut docs_str = String::new();
    for doc in documents {
        docs_str.push_str(&format!(
            "[{}] {}: {}\n",
            doc.doc_type, doc.path, doc.content_snippet
        ));
    }
    format!(
        "Given these project documents and user updates, produce a concise context document \
         covering: goals, status, key decisions, architecture, open questions.\n\
         Start with '# Project Context'.\n\n\
         Documents:\n{}\n\nUser updates:\n{}",
        truncate(&docs_str, MAX_SNIPPET_CHARS),
        truncate(&updates.join("\n"), MAX_SNIPPET_CHARS / 2)
    )
}

pub fn summarize_session_prompt(session_id: &str, events: &[String]) -> String {
    format!(
        "Summarize this agent session in 2-3 sentences for a reconnecting agent.\n\
         Session: {session_id}\n\nEvents:\n{}",
        truncate(&events.join("\n"), MAX_SNIPPET_CHARS)
    )
}

pub fn summarize_event_history_prompt(events: &str) -> String {
    format!(
        "Summarize this agent work history in one paragraph (4-6 sentences).\n\
         Focus on themes and progress, not per-file enumeration.\n\n{}",
        truncate(events, MAX_SNIPPET_CHARS)
    )
}

pub fn update_running_summary_prompt(previous: &str, events: &str, plan: &str) -> String {
    format!(
        "Update the running project summary given the previous version and new events.\n\
         Keep sections: Current Status, Recent Activity, Resume Here, Key Documents, Open Items.\n\
         Under 800 words.\n\n\
         Previous summary:\n{}\n\nNew events:\n{}\n\nPlan excerpt:\n{}",
        truncate(previous, 3000),
        truncate(events, 2000),
        truncate(plan, 2000)
    )
}

pub fn trace_to_doc_summaries(documents: &[TraceDocument]) -> Vec<DocSummary> {
    documents
        .iter()
        .map(|d| DocSummary {
            path: d.path.clone(),
            doc_type: d.doc_type.clone(),
            content_snippet: d.content_snippet.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_are_non_empty() {
        assert!(!summarize_change_prompt("a.md", "plan", "+1").is_empty());
        assert!(!update_running_summary_prompt("prev", "ev", "plan").is_empty());
        assert!(!summarize_session_prompt("s1", &["event".into()]).is_empty());
    }
}
