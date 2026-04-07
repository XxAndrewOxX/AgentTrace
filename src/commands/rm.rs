use crate::git_store::{CommitInfo, GitStore};
use crate::manifest::Manifest;
use crate::types::{Action, Actor, DocType};
use anyhow::Result;
use std::path::Path;

pub fn run(store_root: &Path, file: &Path) -> Result<()> {
    let mut manifest = Manifest::load(store_root)?;

    let doc_type = manifest
        .find_by_path(file)
        .map(|d| d.doc_type.clone())
        .unwrap_or(DocType::Scratch);

    manifest.untrack(file)?;
    manifest.save(store_root)?;

    let full_path = store_root.join(file);
    if full_path.exists() {
        std::fs::remove_file(&full_path)?;
    }

    let git = GitStore::open(store_root)?;
    let info = CommitInfo {
        action: Action::Delete,
        files: vec![(file.to_path_buf(), Action::Delete, doc_type)],
        actor: Actor::User,
        summary: format!("rm {}", file.display()),
        agent_name: None,
        session_id: None,
    };
    git.commit(&info)?;

    println!("Removed {}", file.display());
    Ok(())
}
