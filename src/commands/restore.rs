use crate::git_store::GitStore;
use crate::manifest::Manifest;
use crate::types::DocType;
use anyhow::Result;
use std::path::Path;

pub fn run(store_root: &Path, file: &Path, version: u32) -> Result<()> {
    let git = GitStore::open(store_root)?;
    let manifest = Manifest::load(store_root)?;
    let doc_type = manifest.find_by_path(file)
        .map(|d| d.doc_type.clone())
        .unwrap_or(DocType::Scratch);
    git.restore_file(file, version, doc_type)?;
    println!("Restored {} to version {}", file.display(), version);
    Ok(())
}
