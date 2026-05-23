use crate::git_store::CommitInfo;
use crate::observability::CliOutput;
use crate::store::Store;
use crate::types::{Action, Actor, DocType};
use anyhow::Result;
use std::path::Path;

pub fn run(store_root: &Path, file: &Path, output: &dyn CliOutput) -> Result<()> {
    let mut store = Store::open(store_root)?;

    let doc_type = store
        .manifest
        .find_by_path(file)
        .map(|d| d.doc_type.clone())
        .unwrap_or(DocType::Scratch);

    store.manifest.untrack(file)?;
    store.manifest.save(store_root)?;

    let info = CommitInfo {
        action: Action::Delete,
        files: vec![(file.to_path_buf(), Action::Delete, doc_type)],
        actor: Actor::User,
        summary: format!("untrack {}", file.display()),
        agent_name: None,
        session_id: None,
    };
    store.commit(&info)?;

    output.line(&format!(
        "Untracked {} (file remains on disk)",
        file.display()
    ))?;
    Ok(())
}
