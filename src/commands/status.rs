use crate::store::Store;
use crate::types::FileChange;
use anyhow::Result;
use std::path::Path;

pub fn run(store_root: &Path) -> Result<()> {
    let store = Store::open(store_root)?;

    let changes = store.git.detect_changes()?;

    if changes.is_empty() {
        println!(
            "Store is clean. {} document(s) tracked.",
            store.manifest.len()
        );
        return Ok(());
    }

    println!("Changes detected:");
    for change in &changes {
        match change {
            FileChange::New(p) => {
                let indicator = if store.manifest.is_tracked(p) { "+" } else { "?" };
                println!("  [{}] {}", indicator, p.display());
            }
            FileChange::Modified(p) => println!("  [~] {}", p.display()),
            FileChange::Deleted(p) => println!("  [x] {}", p.display()),
            FileChange::Renamed { from, to } => {
                println!("  [>] {} -> {}", from.display(), to.display())
            }
        }
    }

    let untracked = changes.iter().filter(|c| {
        if let FileChange::New(p) = c {
            !store.manifest.is_tracked(p)
        } else {
            false
        }
    }).count();
    if untracked > 0 {
        println!("\n{} untracked file(s). Use `agent-trace add <type> <file>` to register.", untracked);
    }

    Ok(())
}
