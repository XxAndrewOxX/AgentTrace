use crate::git_store::{CommitInfo, GitStore};
use crate::manifest::Manifest;
use crate::types::{Action, Actor, DocType};
use anyhow::Result;
use std::path::Path;

pub fn run(store_root: &Path, file: &Path, new_type: DocType) -> Result<()> {
    let mut manifest = Manifest::load(store_root)?;

    if !manifest.is_tracked(file) {
        anyhow::bail!("File not tracked: {}", file.display());
    }

    if matches!(new_type, DocType::Context | DocType::Log) {
        eprintln!("Warning: '{}' is system-managed. Agents/system control this type.", new_type);
    }

    manifest.reclassify(file, new_type.clone())?;
    manifest.save(store_root)?;

    let git = GitStore::open(store_root)?;
    let info = CommitInfo {
        action: Action::Modify,
        files: vec![(file.to_path_buf(), Action::Modify, new_type.clone())],
        actor: Actor::User,
        summary: format!("reclassify {} -> {}", file.display(), new_type),
        agent_name: None,
        session_id: None,
    };
    git.commit(&info)?;

    println!("Reclassified {} as {}", file.display(), new_type);
    Ok(())
}
