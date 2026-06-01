use crate::observability::CliOutput;
use crate::store::Store;
use anyhow::Result;
use std::path::Path;

pub fn run(store_root: &Path, file: &Path, output: &dyn CliOutput) -> Result<()> {
    let store = Store::open(store_root)?;
    let entry = store
        .manifest
        .find_by_path(file)
        .ok_or_else(|| anyhow::anyhow!("File not tracked: {}", file.display()))?;

    // Count commits efficiently without loading all entries into memory.
    let version_count = store.git.count_file_commits(file)? as u32;

    // We only need the first and last entries for timestamps/actor; load at most 1 to get
    // the most recent, and separately retrieve the oldest via bounded log call if needed.
    // log_file returns newest-first, so first() = most recent, last() = oldest in the slice.
    // Load at most version_count entries (bounded), which is correct and avoids usize::MAX.
    let history = store.git.log_file(file, version_count as usize)?;
    let created = history
        .last()
        .map(|e| e.timestamp.to_string())
        .unwrap_or_else(|| "unknown".into());
    let last_modified = history
        .first()
        .map(|e| e.timestamp.to_string())
        .unwrap_or_else(|| "unknown".into());
    let created_by = history
        .last()
        .map(|e| e.actor.to_string())
        .unwrap_or_else(|| "unknown".into());

    output.line(&format!("Path:          {}", entry.path.display()))?;
    output.line(&format!("Type:          {}", entry.doc_type))?;
    output.line(&format!("ID:            {}", entry.id))?;
    output.line(&format!("Tags:          {}", entry.tags.join(", ")))?;
    output.line(&format!("Description:   {}", entry.description))?;
    output.line(&format!("Agent:         {}", entry.agent_name))?;
    output.line(&format!("Versions:      {version_count}"))?;
    output.line(&format!("Created:       {created}"))?;
    output.line(&format!("Last modified: {last_modified}"))?;
    output.line(&format!("Created by:    {created_by}"))?;

    Ok(())
}
