use crate::config::StoreConfig;
use crate::git_store::GitStore;
use crate::manifest::Manifest;
use crate::types::DocType;
use anyhow::Result;
use std::path::{Path, PathBuf};

pub fn run(store_root: &Path) -> Result<()> {
    let git = GitStore::open(store_root)?;
    let config = StoreConfig::load(store_root)?;

    // Get all files tracked in git HEAD.
    let head = git_head_files(&git, store_root)?;

    // Rebuild manifest from git state.
    let mut manifest = if Manifest::load(store_root).is_ok() {
        Manifest::load(store_root)?
    } else {
        Manifest::create_empty(config.store, store_root)?
    };

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
    let stale: Vec<PathBuf> = manifest.documents.iter()
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
        added, removed, manifest.documents.len()
    );
    Ok(())
}

fn git_head_files(git: &GitStore, _store_root: &Path) -> Result<Vec<PathBuf>> {
    // Walk the git HEAD tree to get tracked .md files (relative paths).
    git.head_md_files()
}
