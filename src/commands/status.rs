use crate::config::MergedConfig;
use crate::llm::providers::resolve;
use crate::config::CredentialsStore;
use crate::observability::CliOutput;
use crate::store::Store;
use crate::types::FileChange;
use anyhow::Result;
use std::path::Path;

pub fn run(store_root: &Path, output: &dyn CliOutput) -> Result<()> {
    let store = Store::open(store_root)?;

    if let Ok(merged) = MergedConfig::load(store_root) {
        let creds = CredentialsStore::load().unwrap_or_default();
        let info = resolve(&merged, &creds).info();
        let line = if info.degraded {
            "Synthesis: degraded (no backend)".to_string()
        } else {
            format!("Synthesis: {} (ok)", info.label)
        };
        output.line(&line)?;
    }

    let changes = store.git.detect_changes()?;

    if changes.is_empty() {
        output.line(&format!(
            "Store is clean. {} document(s) tracked.",
            store.manifest.len()
        ))?;
        return Ok(());
    }

    output.line("Changes detected:")?;
    for change in &changes {
        match change {
            FileChange::New(p) => {
                let indicator = if store.manifest.is_tracked(p) {
                    "+"
                } else {
                    "?"
                };
                output.line(&format!("  [{}] {}", indicator, p.display()))?;
            }
            FileChange::Modified(p) => output.line(&format!("  [~] {}", p.display()))?,
            FileChange::Deleted(p) => output.line(&format!("  [x] {}", p.display()))?,
            FileChange::Renamed { from, to } => {
                output.line(&format!("  [>] {} -> {}", from.display(), to.display()))?
            }
        }
    }

    let untracked = changes
        .iter()
        .filter(|c| {
            if let FileChange::New(p) = c {
                !store.manifest.is_tracked(p)
            } else {
                false
            }
        })
        .count();
    if untracked > 0 {
        output.line(&format!(
            "\n{untracked} untracked file(s). Use `agent-trace add <type> <file>` to register."
        ))?;
    }

    Ok(())
}
