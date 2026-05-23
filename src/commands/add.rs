use crate::git_store::CommitInfo;
use crate::observability::CliOutput;
use crate::store::Store;
use crate::types::{Action, Actor, DocType};
use anyhow::{bail, Result};
use std::path::Path;

pub fn run(
    store_root: &Path,
    doc_type: DocType,
    file: &Path,
    output: &dyn CliOutput,
) -> Result<()> {
    if !store_root.join(file).exists() && !file.exists() {
        bail!("File not found: {}", file.display());
    }

    // Resolve to a relative path from store root.
    let rel = if file.is_absolute() {
        file.strip_prefix(store_root).unwrap_or(file).to_path_buf()
    } else {
        file.to_path_buf()
    };

    let mut store = Store::open(store_root)?;

    if store.manifest.is_tracked(&rel) {
        bail!("File is already tracked: {}", rel.display());
    }

    store.manifest.register(&rel, doc_type.clone(), "")?;
    store.manifest.save(store_root)?;

    // Regenerate AGENT-TRACE.md so the new document appears.
    let agent_trace_content = crate::agent_trace_md::generate(store_root, &store.manifest);
    std::fs::write(store_root.join("AGENT-TRACE.md"), &agent_trace_content)?;

    let info = CommitInfo {
        action: Action::Create,
        files: vec![
            (rel.clone(), Action::Create, doc_type.clone()),
            (
                std::path::PathBuf::from("AGENT-TRACE.md"),
                Action::Modify,
                DocType::Reference,
            ),
        ],
        actor: Actor::User,
        summary: format!("add {}: {}", doc_type, rel.display()),
        agent_name: None,
        session_id: None,
    };
    store.commit(&info)?;

    output.line(&format!("Added {} as {}", rel.display(), doc_type))?;
    Ok(())
}
