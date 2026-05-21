use crate::agent_trace_md;
use crate::git_store::CommitInfo;
use crate::permissions::{check_permission, PermissionResult};
use crate::store::Store;
use crate::types::{Action, Actor, DocType};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WriteDocumentError {
    #[error("Permission denied: {path} — {reason}")]
    PermissionDenied { path: PathBuf, reason: String },
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub fn write_document(
    root: &Path,
    file: &Path,
    content: &str,
    actor: &Actor,
    summary_prefix: &str,
) -> std::result::Result<PathBuf, WriteDocumentError> {
    let rel = if file.is_absolute() {
        file.strip_prefix(root).unwrap_or(file).to_path_buf()
    } else {
        file.to_path_buf()
    };

    let mut store = Store::open(root).map_err(WriteDocumentError::Other)?;

    let (doc_type, was_tracked) = match store.manifest.find_by_path(&rel) {
        Some(entry) => (entry.doc_type.clone(), true),
        None => (DocType::Scratch, false),
    };

    match check_permission(&doc_type, actor, &store.overrides, Some(&rel)) {
        PermissionResult::Denied { reason } => {
            return Err(WriteDocumentError::PermissionDenied {
                path: rel,
                reason,
            });
        }
        PermissionResult::Allowed | PermissionResult::RequiresConfirmation { .. } => {}
    }

    let full_path = root.join(&rel);
    if let Some(parent) = full_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| WriteDocumentError::Other(e.into()))?;
    }

    let action = if full_path.exists() {
        Action::Modify
    } else {
        Action::Create
    };
    std::fs::write(&full_path, content).map_err(|e| WriteDocumentError::Other(e.into()))?;

    let mut files_to_commit = vec![(rel.clone(), action, doc_type)];

    if !was_tracked {
        store
            .manifest
            .register(&rel, DocType::Scratch, actor.agent_name().unwrap_or(""))
            .map_err(WriteDocumentError::Other)?;
        store.manifest.save(root).map_err(WriteDocumentError::Other)?;
        let at_content = agent_trace_md::generate(root, &store.manifest);
        let at_tmp = root.join(".agent-trace").join("AGENT-TRACE.md.tmp");
        std::fs::write(&at_tmp, &at_content).map_err(|e| WriteDocumentError::Other(e.into()))?;
        std::fs::rename(&at_tmp, root.join("AGENT-TRACE.md"))
            .map_err(|e| WriteDocumentError::Other(e.into()))?;
        files_to_commit.push((
            PathBuf::from("AGENT-TRACE.md"),
            Action::Modify,
            DocType::Reference,
        ));
    }

    let info = CommitInfo {
        action: files_to_commit[0].1.clone(),
        files: files_to_commit,
        actor: actor.clone(),
        summary: format!("{}: {}", summary_prefix, rel.display()),
        agent_name: actor.agent_name().map(String::from),
        session_id: None,
    };
    store.commit(&info).map_err(WriteDocumentError::Other)?;

    // Keep context synthesis consistent for synchronous writes too.
    if matches!(info.files[0].2, DocType::Plan | DocType::Reference) {
        if let Ok(content) = crate::context::synthesize_no_llm(root, &store.manifest) {
            let _ = crate::context::write_context(root, &content);
        }
    }

    Ok(rel)
}
