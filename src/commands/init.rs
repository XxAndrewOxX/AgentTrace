use crate::config::{PollingConfig, StoreConfig, StoreInfo};
use crate::git_store::GitStore;
use crate::manifest::Manifest;
use crate::observability::CliOutput;
use crate::types::DocType;
use anyhow::Result;
use std::path::Path;

pub fn run(path: &Path, scan: bool, output: &dyn CliOutput) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    // Check if already initialised.
    let store_dir = path.join(".agent-trace");
    if store_dir.join("config.toml").exists() {
        output.line(&format!("Store already initialised at {}", path.display()))?;
        output.line(&format!(
            "  Config: {}",
            store_dir.join("config.toml").display()
        ))?;
        output.line(&format!(
            "  Manifest: {}",
            store_dir.join("manifest.toml").display()
        ))?;
        return Ok(());
    }

    // Create .agent-trace/ with restricted permissions.
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&store_dir)?;
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(&store_dir)?;
    }

    // Create subdirectories.
    std::fs::create_dir_all(store_dir.join("locks"))?;

    // Create empty files.
    let context_updates = store_dir.join("context_updates.jsonl");
    if !context_updates.exists() {
        std::fs::write(&context_updates, "")?;
    }
    let summary_events = store_dir.join("summary_events.jsonl");
    if !summary_events.exists() {
        std::fs::write(&summary_events, "")?;
    }
    let cmd_history = store_dir.join("command_history.txt");
    if !cmd_history.exists() {
        std::fs::write(&cmd_history, "")?;
    }

    // Generate store config.
    let store_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "agent-trace-store".to_string());
    let store_info = StoreInfo::new(store_name);
    let store_config = StoreConfig {
        store: store_info.clone(),
        llm: None,
        synthesis: None,
        polling: PollingConfig::default(),
    };
    store_config.save(&path)?;

    // Initialise git store.
    let git = GitStore::init(&path)?;

    // Create empty manifest.
    let mut manifest = Manifest::create_empty(store_info, &path)?;

    // Write .gitignore at store root.
    let gitignore = path.join(".gitignore");
    if !gitignore.exists() {
        std::fs::write(&gitignore, DEFAULT_GITIGNORE)?;
    }

    // Generate initial AGENT-TRACE.md and commit it.
    let agent_trace_content = crate::agent_trace_md::generate(&path, &manifest);
    std::fs::write(path.join("AGENT-TRACE.md"), &agent_trace_content)?;
    {
        let agent_trace_info = crate::git_store::CommitInfo {
            action: crate::types::Action::Create,
            files: vec![(
                std::path::PathBuf::from("AGENT-TRACE.md"),
                crate::types::Action::Create,
                crate::types::DocType::Reference,
            )],
            actor: crate::types::Actor::System,
            summary: "init: create AGENT-TRACE.md".into(),
            agent_name: None,
            session_id: None,
        };
        git.commit(&agent_trace_info)?;
    }

    // If --scan: register all .md files.
    if scan {
        let count = scan_and_register(&path, &mut manifest)?;
        manifest.save(&path)?;
        // Commit the scanned files (not AGENT-TRACE.md — already committed).
        if count > 0 {
            // Regenerate AGENT-TRACE.md now that the manifest has content.
            let agent_trace_content = crate::agent_trace_md::generate(&path, &manifest);
            std::fs::write(path.join("AGENT-TRACE.md"), &agent_trace_content)?;

            let mut files: Vec<_> = manifest
                .documents()
                .iter()
                .map(|d| {
                    (
                        d.path.clone(),
                        crate::types::Action::Create,
                        d.doc_type.clone(),
                    )
                })
                .collect();
            files.push((
                std::path::PathBuf::from("AGENT-TRACE.md"),
                crate::types::Action::Modify,
                crate::types::DocType::Reference,
            ));
            let info = crate::git_store::CommitInfo {
                action: crate::types::Action::Init,
                files,
                actor: crate::types::Actor::System,
                summary: format!("scanned {count} existing markdown files"),
                agent_name: None,
                session_id: None,
            };
            git.commit(&info)?;
            output.line(&format!(
                "Registered {count} existing markdown files as scratch."
            ))?;
        }
    }

    output.line(&format!(
        "Initialised agent-trace store at {}",
        path.display()
    ))?;
    output.line("Next: run `agent-trace model setup` (or `model pull 0.5b` for offline GGUF).")?;
    Ok(())
}

fn scan_and_register(root: &Path, manifest: &mut Manifest) -> Result<usize> {
    let mut count = 0;
    for entry in walkdir_md(root) {
        let rel = entry.strip_prefix(root).unwrap_or(&entry);
        // Skip .agent-trace directory and agent-trace-managed files.
        if rel.starts_with(".agent-trace") {
            continue;
        }
        if rel == std::path::Path::new("AGENT-TRACE.md")
            || rel == std::path::Path::new("context.md")
        {
            continue;
        }
        if manifest.is_tracked(rel) {
            continue;
        }
        manifest.register(rel, DocType::Scratch, "")?;
        count += 1;
    }
    Ok(count)
}

fn walkdir_md(root: &Path) -> Vec<std::path::PathBuf> {
    let mut results = Vec::new();
    walk(root, &mut results);
    results
}

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip hidden directories.
            if path
                .file_name()
                .map(|n| n.to_string_lossy().starts_with('.'))
                .unwrap_or(false)
            {
                continue;
            }
            walk(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            out.push(path);
        }
    }
}

const DEFAULT_GITIGNORE: &str = r#"# agent-trace defaults
.DS_Store
*.tmp
*.swp
*.swo
~*
.venv/
venv/
node_modules/
__pycache__/
*.pyc
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::NoopOutput;
    use tempfile::TempDir;

    #[test]
    fn test_init_empty_directory() {
        let tmp = TempDir::new().unwrap();
        run(tmp.path(), false, &NoopOutput).unwrap();
        assert!(tmp.path().join(".agent-trace").exists());
        assert!(tmp.path().join(".agent-trace").join("config.toml").exists());
        assert!(tmp
            .path()
            .join(".agent-trace")
            .join("manifest.toml")
            .exists());
        assert!(tmp.path().join(".agent-trace").join("repo").exists());
        assert!(tmp.path().join(".agent-trace").join("locks").exists());
        assert!(tmp.path().join(".gitignore").exists());
    }

    #[test]
    fn test_init_idempotent() {
        let tmp = TempDir::new().unwrap();
        run(tmp.path(), false, &NoopOutput).unwrap();
        // Second init should not error.
        run(tmp.path(), false, &NoopOutput).unwrap();
    }

    #[test]
    fn test_init_with_scan() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("prd.md"), "# PRD").unwrap();
        std::fs::write(tmp.path().join("notes.md"), "notes").unwrap();
        run(tmp.path(), true, &NoopOutput).unwrap();
        let manifest = crate::manifest::Manifest::load(tmp.path()).unwrap();
        assert_eq!(manifest.len(), 2);
        assert!(manifest
            .documents()
            .iter()
            .all(|d| d.doc_type == DocType::Scratch));
    }

    #[test]
    fn test_config_has_uuid() {
        let tmp = TempDir::new().unwrap();
        run(tmp.path(), false, &NoopOutput).unwrap();
        let cfg = crate::config::StoreConfig::load(tmp.path()).unwrap();
        assert!(cfg.store.id.0.parse::<uuid::Uuid>().is_ok());
        assert!(!cfg.store.agent_trace_version.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn test_agent_trace_dir_permissions() {
        use std::os::unix::fs::MetadataExt;
        let tmp = TempDir::new().unwrap();
        run(tmp.path(), false, &NoopOutput).unwrap();
        let meta = std::fs::metadata(tmp.path().join(".agent-trace")).unwrap();
        // 0700 = rwx------
        assert_eq!(meta.mode() & 0o777, 0o700);
    }
}
