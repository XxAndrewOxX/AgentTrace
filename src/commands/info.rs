use crate::git_store::GitStore;
use crate::manifest::Manifest;
use anyhow::{bail, Result};
use std::path::Path;

pub fn run(store_root: &Path, file: &Path) -> Result<()> {
    let manifest = Manifest::load(store_root)?;
    let entry = manifest.find_by_path(file)
        .ok_or_else(|| anyhow::anyhow!("File not tracked: {}", file.display()))?;

    let git = GitStore::open(store_root)?;
    let history = git.log_file(file, usize::MAX)?;
    let version_count = history.len() as u32;
    let created = history.last().map(|e| e.timestamp.to_string()).unwrap_or_else(|| "unknown".into());
    let last_modified = history.first().map(|e| e.timestamp.to_string()).unwrap_or_else(|| "unknown".into());
    let created_by = history.last()
        .map(|e| e.actor.to_string())
        .unwrap_or_else(|| "unknown".into());

    println!("Path:          {}", entry.path.display());
    println!("Type:          {}", entry.doc_type);
    println!("ID:            {}", entry.id);
    println!("Tags:          {}", entry.tags.join(", "));
    println!("Description:   {}", entry.description);
    println!("Agent:         {}", entry.agent_name);
    println!("Versions:      {}", version_count);
    println!("Created:       {}", created);
    println!("Last modified: {}", last_modified);
    println!("Created by:    {}", created_by);

    Ok(())
}
