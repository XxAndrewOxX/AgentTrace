use crate::git_store::{CommitInfo, GitStore};
use crate::manifest::Manifest;
use crate::types::{Action, Actor, DocType};
use anyhow::{bail, Result};
use std::path::Path;

pub fn run(store_root: &Path, doc_type: DocType, file: &Path) -> Result<()> {
    if !store_root.join(file).exists() && !file.exists() {
        bail!("File not found: {}", file.display());
    }

    // Resolve to a relative path from store root.
    let rel = if file.is_absolute() {
        file.strip_prefix(store_root)
            .unwrap_or(file)
            .to_path_buf()
    } else {
        file.to_path_buf()
    };

    let git = GitStore::open(store_root)?;
    let mut manifest = Manifest::load(store_root)?;

    if manifest.is_tracked(&rel) {
        bail!("File is already tracked: {}", rel.display());
    }

    manifest.register(&rel, doc_type.clone(), "")?;
    manifest.save(store_root)?;

    let info = CommitInfo {
        action: Action::Create,
        files: vec![(rel.clone(), Action::Create, doc_type.clone())],
        actor: Actor::User,
        summary: format!("add {}: {}", doc_type, rel.display()),
        agent_name: None,
        session_id: None,
    };
    git.commit(&info)?;

    println!("Added {} as {}", rel.display(), doc_type);
    Ok(())
}
