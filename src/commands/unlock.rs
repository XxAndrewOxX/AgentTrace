use crate::observability::CliOutput;
use crate::permissions::OverrideEntry;
use crate::store::Store;
use anyhow::Result;
use chrono::{Duration, Utc};
use std::path::Path;

pub fn run(
    store_root: &Path,
    file: &Path,
    for_actor: &str,
    duration_minutes: u32,
    output: &dyn CliOutput,
) -> Result<()> {
    let mut store = Store::open(store_root)?;
    let entry = store
        .manifest
        .find_by_path(file)
        .ok_or_else(|| anyhow::anyhow!("File not tracked: {}", file.display()))?;

    store.overrides.prune_expired();

    let now = Utc::now();
    let expires = now + Duration::minutes(duration_minutes as i64);

    store.overrides.add(OverrideEntry {
        doc_id: entry.id.0.clone(),
        path: file.to_path_buf(),
        allow_actor: for_actor.to_string(),
        granted_at: now,
        expires_at: expires,
        granted_by: "user".to_string(),
    })?;
    store.overrides.save(store_root)?;

    output.line(&format!(
        "Override granted: {} can write {} for {} minutes (expires {})",
        for_actor,
        file.display(),
        duration_minutes,
        expires.format("%H:%M:%S")
    ))?;
    Ok(())
}
