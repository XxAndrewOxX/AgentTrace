use crate::git_store::GitStore;
use crate::manifest::Manifest;
use anyhow::Result;
use std::path::Path;

pub fn run(store_root: &Path, file: &Path) -> Result<()> {
    let manifest = Manifest::load(store_root)?;
    let entry = manifest.find_by_path(file)
        .ok_or_else(|| anyhow::anyhow!("File not tracked: {}", file.display()))?;

    let git = GitStore::open(store_root)?;

    // Count commits efficiently without loading all entries into memory.
    let version_count = git.count_file_commits(file)? as u32;

    // We only need the first and last entries for timestamps/actor; load at most 1 to get
    // the most recent, and separately retrieve the oldest via bounded log call if needed.
    // log_file returns newest-first, so first() = most recent, last() = oldest in the slice.
    // Load at most version_count entries (bounded), which is correct and avoids usize::MAX.
    let history = git.log_file(file, version_count as usize)?;
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
