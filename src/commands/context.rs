use crate::context::{load_pending_updates, synthesize_no_llm, write_context};
use crate::llm::trace_insights::{TraceDocument, TraceInsightsFacade};
use crate::observability::CliOutput;
use crate::store::Store;
use anyhow::Result;
use chrono::Utc;
use clap::Subcommand;
use std::path::Path;

#[derive(Subcommand, Debug)]
pub enum ContextCmd {
    /// Force a full re-synthesis of context.md.
    Refresh,
    /// Inject a specific update statement for the next synthesis.
    Update {
        /// The update statement to inject.
        statement: String,
    },
    /// Print the current context.md to stdout.
    Show,
    /// List pending (unincorporated) context updates.
    Updates,
}

pub fn run(store_root: &Path, cmd: ContextCmd, output: &dyn CliOutput) -> Result<()> {
    match cmd {
        ContextCmd::Show => {
            let context_file = store_root.join("context.md");
            if !context_file.exists() {
                output
                    .line("No context.md found. Run `agent-trace context refresh` to generate.")?;
                return Ok(());
            }
            let content = std::fs::read_to_string(&context_file)?;
            output.raw_stdout(&content)?;
        }
        ContextCmd::Update { statement } => {
            let updates_file = store_root
                .join(".agent-trace")
                .join("context_updates.jsonl");
            let entry = serde_json::json!({
                "timestamp": Utc::now().to_rfc3339(),
                "update": statement,
                "incorporated": false,
            });
            let mut content = if updates_file.exists() {
                std::fs::read_to_string(&updates_file)?
            } else {
                String::new()
            };
            content.push_str(&entry.to_string());
            content.push('\n');
            std::fs::write(&updates_file, content)?;
            output.line(&format!("Context update queued: {statement}"))?;
        }
        ContextCmd::Updates => {
            let pending = load_pending_updates(store_root)?;
            if pending.is_empty() {
                output.line("No pending context updates.")?;
            } else {
                output.line(&format!("{} pending update(s):", pending.len()))?;
                for u in &pending {
                    output.line(&format!("  [{}] {}", u.timestamp, u.update))?;
                }
            }
        }
        ContextCmd::Refresh => {
            let store = Store::open(store_root)?;
            let content = if let Some(api) = TraceInsightsFacade::from_store_root(store_root).ok() {
                let docs = store
                    .manifest
                    .documents()
                    .iter()
                    .filter(|d| {
                        matches!(
                            d.doc_type,
                            crate::types::DocType::Plan
                                | crate::types::DocType::Reference
                                | crate::types::DocType::Scratch
                        )
                    })
                    .map(|d| {
                        let content =
                            std::fs::read_to_string(store_root.join(&d.path)).unwrap_or_default();
                        let snippet: String = content.chars().take(2000).collect();
                        TraceDocument {
                            path: d.path.display().to_string(),
                            doc_type: d.doc_type.clone(),
                            content_snippet: snippet,
                        }
                    })
                    .collect::<Vec<_>>();
                let updates = load_pending_updates(store_root)?
                    .into_iter()
                    .map(|u| u.update)
                    .collect::<Vec<_>>();
                match api.synthesize_context(&docs, &updates) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("LLM synthesize_context failed, using template: {e}");
                        synthesize_no_llm(store_root, &store.manifest)?
                    }
                }
            } else {
                synthesize_no_llm(store_root, &store.manifest)?
            };
            write_context(store_root, &content)?;
            let plans = store.manifest.list(Some(&crate::types::DocType::Plan));
            let refs = store.manifest.list(Some(&crate::types::DocType::Reference));
            output.line(&format!(
                "context.md refreshed ({} plans, {} reference docs).",
                plans.len(),
                refs.len()
            ))?;
        }
    }
    Ok(())
}
