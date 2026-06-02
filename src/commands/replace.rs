use crate::git_store::CommitInfo;
use crate::observability::CliOutput;
use crate::permissions::{check_permission, PermissionResult};
use crate::store::Store;
use crate::types::{Action, Actor, DocType};
use anyhow::Result;
use std::path::Path;

pub fn run(
    store_root: &Path,
    find: &str,
    replace: &str,
    type_filter: Option<&DocType>,
    dry_run: bool,
    output: &dyn CliOutput,
) -> Result<()> {
    let store = Store::open(store_root)?;
    let docs = store.manifest.list(type_filter);

    let mut matches: Vec<(std::path::PathBuf, DocType, String)> = Vec::new();

    for doc in &docs {
        let full = store_root.join(&doc.path);
        if !full.exists() {
            continue;
        }
        let content = std::fs::read_to_string(&full)?;
        if content.contains(find) {
            matches.push((doc.path.clone(), doc.doc_type.clone(), content));
        }
    }

    if matches.is_empty() {
        output.line("No matches found.")?;
        return Ok(());
    }

    output.line(&format!("Found {} file(s) with matches:", matches.len()))?;
    for (path, dt, _) in &matches {
        output.line(&format!("  [{}] {}", dt.indicator(), path.display()))?;
    }

    if dry_run {
        output.line("(dry-run: no changes applied)")?;
        return Ok(());
    }

    let mut committed: Vec<(std::path::PathBuf, Action, DocType)> = Vec::new();

    for (path, doc_type, content) in matches {
        // Check permission.
        let perm = check_permission(&doc_type, &Actor::User, &store.overrides, Some(&path));
        if let PermissionResult::Denied { reason } = perm {
            output.warn(&format!("Skipping {} (denied): {}", path.display(), reason))?;
            continue;
        }

        let new_content = content.replace(find, replace);
        std::fs::write(store_root.join(&path), &new_content)?;
        committed.push((path, Action::Modify, doc_type));
    }

    if !committed.is_empty() {
        let info = CommitInfo {
            action: Action::Modify,
            files: committed,
            actor: Actor::User,
            summary: format!("replace '{find}' with '{replace}'"),
            agent_name: None,
            session_id: None,
        };
        store.commit(&info)?;
        output.line("Changes applied.")?;
    }

    Ok(())
}
