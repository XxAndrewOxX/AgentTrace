use crate::git_store::CommitInfo;
use crate::observability::CliOutput;
use crate::store::Store;
use crate::types::{Action, Actor, DocType};
use anyhow::Result;
use std::path::Path;

pub fn run(
    store_root: &Path,
    file: &Path,
    new_type: DocType,
    output: &dyn CliOutput,
) -> Result<()> {
    let mut store = Store::open(store_root)?;

    if !store.manifest.is_tracked(file) {
        anyhow::bail!("File not tracked: {}", file.display());
    }

    if matches!(new_type, DocType::Context | DocType::Log) {
        output.warn(&format!(
            "Warning: '{new_type}' is system-managed. Agents/system control this type."
        ))?;
    }

    store.manifest.reclassify(file, new_type.clone())?;
    store.manifest.save(store_root)?;

    let info = CommitInfo {
        action: Action::Modify,
        files: vec![(file.to_path_buf(), Action::Modify, new_type.clone())],
        actor: Actor::User,
        summary: format!("reclassify {} -> {}", file.display(), new_type),
        agent_name: None,
        session_id: None,
    };
    store.commit(&info)?;

    output.line(&format!("Reclassified {} as {}", file.display(), new_type))?;
    Ok(())
}
