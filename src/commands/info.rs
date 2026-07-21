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

    // Single history walk: count + newest/oldest bounds (avoids count + log double scan).
    let (version_count_usize, newest, oldest) = store.git.file_history_bounds(file)?;
    let version_count = version_count_usize as u32;
    let created = oldest
        .as_ref()
        .map(|e| e.timestamp.to_string())
        .unwrap_or_else(|| "unknown".into());
    let last_modified = newest
        .as_ref()
        .map(|e| e.timestamp.to_string())
        .unwrap_or_else(|| "unknown".into());
    let created_by = oldest
        .as_ref()
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
