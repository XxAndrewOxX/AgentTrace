use crate::config::StoreConfig;
use crate::git_store::GitStore;
use crate::manifest::Manifest;
use crate::store::Store;
use crate::types::DocType;
use anyhow::Result;
use std::path::{Path, PathBuf};

pub fn run(store_root: &Path) -> Result<()> {
    // Try Store::open first; if manifest is corrupt, fall back to rebuilding.
    let (git, mut manifest) = match Store::open(store_root) {
        Ok(store) => (store.git, store.manifest),
        Err(_) => {
            let git = GitStore::open(store_root)?;
            let config = StoreConfig::load(store_root)?;
            let manifest = Manifest::create_empty(config.store, store_root)?;
            (git, manifest)
        }
    };

    // Get all files tracked in git HEAD.
    let head = git.head_md_files()?;

    let mut added = 0;
    let mut removed = 0;

    // Add files in git but not in manifest.
    for path in &head {
        let rel = path.strip_prefix(store_root).unwrap_or(path);
        if !manifest.is_tracked(rel) {
            manifest.register(rel, DocType::Scratch, "")?;
            added += 1;
        }
    }

    // Remove manifest entries whose files are gone.
    let stale: Vec<PathBuf> = manifest.documents().iter()
        .filter(|d| !store_root.join(&d.path).exists())
        .map(|d| d.path.clone())
        .collect();
    for path in &stale {
        let _ = manifest.untrack(path);
        removed += 1;
    }

    manifest.save(store_root)?;
    println!(
        "Repair complete: {} added, {} removed. {} documents tracked.",
        added, removed, manifest.len()
    );
    Ok(())
}
