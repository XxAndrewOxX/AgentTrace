use crate::git_store::{CommitInfo, GitStore};
use crate::manifest::Manifest;
use crate::permissions::{check_permission, Overrides, PermissionResult};
use crate::types::{Action, Actor, DocType};
use anyhow::Result;
use std::path::Path;

pub fn run(
    store_root: &Path,
    find: &str,
    replace: &str,
    type_filter: Option<&DocType>,
    dry_run: bool,
) -> Result<()> {
    let manifest = Manifest::load(store_root)?;
    let overrides = Overrides::load(store_root)?;
    let docs = manifest.list(type_filter);

    let mut matches: Vec<(std::path::PathBuf, DocType, String)> = Vec::new();

    for doc in &docs {
        let full = store_root.join(&doc.path);
        if !full.exists() { continue; }
        let content = std::fs::read_to_string(&full)?;
        if content.contains(find) {
            matches.push((doc.path.clone(), doc.doc_type.clone(), content));
        }
    }

    if matches.is_empty() {
        println!("No matches found.");
        return Ok(());
    }

    println!("Found {} file(s) with matches:", matches.len());
    for (path, dt, _) in &matches {
        println!("  [{}] {}", dt.indicator(), path.display());
    }

    if dry_run {
        println!("(dry-run: no changes applied)");
        return Ok(());
    }

    let git = GitStore::open(store_root)?;
    let mut committed: Vec<(std::path::PathBuf, Action, DocType)> = Vec::new();

    for (path, doc_type, content) in matches {
        // Check permission.
        let perm = check_permission(&doc_type, &Actor::User, &Action::Modify, &overrides, Some(&path));
        if let PermissionResult::Denied { reason } = perm {
            eprintln!("Skipping {} (denied): {}", path.display(), reason);
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
            summary: format!("replace '{}' with '{}'", find, replace),
            agent_name: None,
            session_id: None,
        };
        git.commit(&info)?;
        println!("Changes applied.");
    }

    Ok(())
}
