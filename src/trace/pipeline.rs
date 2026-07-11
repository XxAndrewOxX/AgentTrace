use crate::git_store::CommitInfo;
use crate::llm::trace_insights::Llm;
use crate::permissions::{check_permission, PermissionResult};
use crate::runtime::UiEvent;
use crate::store::Store;
use crate::trace::context::synthesize_context_content;
use crate::trace::running_summary::{self, SummaryEvent};
use crate::trace::{
    agent_trace_md,
    logs::{append_agent_log, summarize_change_no_llm, LogSynthEntry},
};
use crate::types::{Action, Actor, DocType};
use chrono::Utc;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use thiserror::Error;
use tokio::sync::mpsc::Sender;

static CONTEXT_REFRESH_IN_FLIGHT: LazyLock<Mutex<HashMap<PathBuf, bool>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Error)]
pub enum WriteDocumentError {
    #[error("Permission denied: {path} — {reason}")]
    PermissionDenied { path: PathBuf, reason: String },
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

fn detected_by_from_source(source: &str) -> &'static str {
    match source {
        "mcp_write" | "mcp" => "mcp",
        "cli_write" | "cli" => "cli",
        "poll" => "poll",
        _ => "system",
    }
}

fn source_from_prefix(summary_prefix: &str) -> &'static str {
    if summary_prefix.starts_with("mcp") {
        "mcp_write"
    } else if summary_prefix.starts_with("agent") {
        "cli_write"
    } else {
        "system"
    }
}

pub fn write_document(
    root: &Path,
    file: &Path,
    content: &str,
    actor: &Actor,
    summary_prefix: &str,
    session_id: Option<&str>,
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
            return Err(WriteDocumentError::PermissionDenied { path: rel, reason });
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

    let files_to_commit = vec![(rel.clone(), action, doc_type)];

    if !was_tracked {
        store
            .manifest
            .register(&rel, DocType::Scratch, actor.agent_name().unwrap_or(""))
            .map_err(WriteDocumentError::Other)?;
        store
            .manifest
            .save(root)
            .map_err(WriteDocumentError::Other)?;
    }

    let info = CommitInfo {
        action: files_to_commit[0].1.clone(),
        files: files_to_commit,
        actor: actor.clone(),
        summary: format!("{}: {}", summary_prefix, rel.display()),
        agent_name: actor.agent_name().map(String::from),
        session_id: session_id.map(String::from),
    };
    store.commit(&info).map_err(WriteDocumentError::Other)?;

    let source = source_from_prefix(summary_prefix);
    apply_trace_hooks(
        root,
        &store.git,
        &store.manifest,
        actor,
        session_id,
        &info.files,
        source,
        None,
    )
    .map_err(WriteDocumentError::Other)?;

    Ok(rel)
}

/// Post-write trace pipeline: summaries, running context, agent logs, index sync.
///
/// Hot path uses template summaries and append-only event/log I/O. LLM context
/// synthesis is scheduled on a background thread when a non-degraded backend is
/// available so MCP/poll commits are not blocked on HTTP round-trips.
#[allow(clippy::too_many_arguments)]
pub fn apply_trace_hooks(
    store_root: &Path,
    git: &crate::git_store::GitStore,
    manifest: &crate::manifest::Manifest,
    actor: &Actor,
    session_id: Option<&str>,
    changed_files: &[(PathBuf, Action, DocType)],
    source: &str,
    ui_tx: Option<&Sender<UiEvent>>,
) -> anyhow::Result<()> {
    if changed_files.is_empty() {
        return Ok(());
    }

    // Gate: refuse writes that would emit degraded artifacts unless allowed.
    let backend = Llm::require_backend(Some(store_root))?;
    let agent_label = actor.agent_name().unwrap_or("system");

    // One cheap template summary per file on the hot path (no blocking LLM).
    let mut per_file: Vec<(PathBuf, Action, DocType, crate::types::DiffStats, String)> =
        Vec::with_capacity(changed_files.len());
    for (path, action, doc_type) in changed_files {
        let stats = git.diff_stats(path, None, None).unwrap_or_default();
        let summary = summarize_change_no_llm(path, doc_type, &stats, agent_label);
        per_file.push((
            path.clone(),
            action.clone(),
            doc_type.clone(),
            stats,
            summary,
        ));
    }

    if actor.is_agent() {
        if let (Some(agent_name), Some(sid)) = (actor.agent_name(), session_id) {
            let entries: Vec<LogSynthEntry> = per_file
                .iter()
                .map(|(path, _, _, _, summary)| LogSynthEntry {
                    timestamp: Utc::now(),
                    path: path.clone(),
                    summary: summary.clone(),
                })
                .collect();
            append_agent_log(store_root, git, agent_name, sid, &entries)?;
        }
    }

    for (path, action, doc_type, stats, event_summary) in &per_file {
        let event = SummaryEvent {
            timestamp: Utc::now().to_rfc3339(),
            session_id: session_id.map(String::from),
            agent_name: actor.agent_name().map(String::from),
            actor: actor.to_string(),
            action: action.to_string(),
            change_kind: action.to_string(),
            path: path.display().to_string(),
            doc_type: doc_type.to_string(),
            summary: event_summary.clone(),
            source: source.to_string(),
            detected_by: detected_by_from_source(source).to_string(),
            lines_added: stats.lines_added,
            lines_removed: stats.lines_removed,
        };
        running_summary::append_event(store_root, event.clone())?;
        if let Some(tx) = ui_tx {
            let _ = tx.try_send(UiEvent::SummaryAppended(event));
        }
    }

    if let Err(e) = running_summary::refresh_template(store_root, git, manifest) {
        tracing::warn!("running summary template refresh failed: {e}");
    } else if let Some(tx) = ui_tx {
        let _ = tx.try_send(UiEvent::RunningSummaryRefreshed);
    }
    running_summary::schedule_synthesis_refresh(store_root.to_path_buf());

    sync_agent_trace_md(store_root, git, manifest)?;

    let changed_paths: Vec<PathBuf> = changed_files
        .iter()
        .map(|(p, _, _)| p.clone())
        .filter(|p| crate::git_store::should_track_activity(p))
        .collect();
    if changed_paths.is_empty() {
        return Ok(());
    }

    let context_missing = !store_root.join("context.md").exists();
    if backend.degraded || context_missing {
        // Template path is cheap and keeps first-write / degraded stores consistent.
        let trace_insights = Llm::from_store_root(store_root)?;
        sync_context_md(
            store_root,
            git,
            manifest,
            &trace_insights,
            &changed_paths,
            ui_tx,
        )?;
    } else {
        schedule_context_refresh(store_root.to_path_buf(), changed_paths, ui_tx.cloned());
    }

    Ok(())
}

fn schedule_context_refresh(
    store_root: PathBuf,
    changed_paths: Vec<PathBuf>,
    ui_tx: Option<Sender<UiEvent>>,
) {
    let should_spawn = {
        let mut in_flight = CONTEXT_REFRESH_IN_FLIGHT
            .lock()
            .expect("context refresh lock poisoned");
        if *in_flight.get(&store_root).unwrap_or(&false) {
            false
        } else {
            in_flight.insert(store_root.clone(), true);
            true
        }
    };
    if !should_spawn {
        return;
    }

    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<()> {
            let git = crate::git_store::GitStore::open(&store_root)?;
            let manifest = crate::manifest::Manifest::load(&store_root)?;
            let trace_insights = Llm::from_store_root(&store_root)?;
            sync_context_md(
                &store_root,
                &git,
                &manifest,
                &trace_insights,
                &changed_paths,
                ui_tx.as_ref(),
            )
        })();
        if let Err(e) = result {
            tracing::warn!("background context refresh failed: {e}");
        }
        CONTEXT_REFRESH_IN_FLIGHT
            .lock()
            .expect("context refresh lock poisoned")
            .insert(store_root, false);
    });
}

fn sync_agent_trace_md(
    store_root: &Path,
    git: &crate::git_store::GitStore,
    manifest: &crate::manifest::Manifest,
) -> anyhow::Result<()> {
    agent_trace_md::sync(store_root, manifest, git)
}

fn sync_context_md(
    store_root: &Path,
    git: &crate::git_store::GitStore,
    manifest: &crate::manifest::Manifest,
    trace_insights: &Llm,
    changed_paths: &[PathBuf],
    ui_tx: Option<&Sender<UiEvent>>,
) -> anyhow::Result<()> {
    let (new_content, commit_label) =
        synthesize_context_content(store_root, manifest, trace_insights, changed_paths)?;
    let target = store_root.join("context.md");
    let existing = std::fs::read_to_string(&target).unwrap_or_default();
    if existing == new_content {
        return Ok(());
    }

    crate::trace::context::write_context(store_root, &new_content)?;
    let info = CommitInfo {
        action: Action::Modify,
        files: vec![(
            PathBuf::from("context.md"),
            Action::Modify,
            DocType::Context,
        )],
        actor: Actor::System,
        summary: format!("refresh synthesized context ({commit_label})"),
        agent_name: None,
        session_id: None,
    };
    git.commit(&info)?;
    if let Some(tx) = ui_tx {
        let _ = tx.try_send(UiEvent::ContextRefreshed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StoreInfo;
    use crate::git_store::GitStore;
    use crate::manifest::Manifest;
    use tempfile::TempDir;

    fn setup(tmp: &TempDir) -> (PathBuf, Manifest, GitStore) {
        let root = tmp.path().to_path_buf();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        let git = GitStore::init(&root).unwrap();
        let info = StoreInfo::new("test".into());
        let manifest = Manifest::create_empty(info.clone(), &root).unwrap();
        let store_cfg = crate::config::StoreConfig {
            store: info,
            llm: None,
            synthesis: Some(crate::config::SynthesisConfig::for_unit_tests_degraded()),
            polling: crate::config::PollingConfig::default(),
        };
        store_cfg.save(&root).unwrap();
        (root, manifest, git)
    }

    #[test]
    fn scratch_write_triggers_context_refresh() {
        std::env::set_var("AGENT_TRACE_ALLOW_DEGRADED", "1");
        let tmp = TempDir::new().unwrap();
        let (root, mut manifest, git) = setup(&tmp);
        let scratch_path = PathBuf::from("notes.md");
        std::fs::write(
            root.join(&scratch_path),
            "scratch body: reconnect watermark test",
        )
        .unwrap();
        manifest
            .register(&scratch_path, DocType::Scratch, "")
            .unwrap();
        manifest.save(&root).unwrap();

        let changed = vec![(scratch_path, Action::Modify, DocType::Scratch)];
        apply_trace_hooks(
            &root,
            &git,
            &manifest,
            &Actor::User,
            None,
            &changed,
            "cli_write",
            None,
        )
        .unwrap();

        let ctx = std::fs::read_to_string(root.join("context.md")).expect("context.md created");
        assert!(ctx.contains("reconnect watermark test"));
        assert!(ctx.contains("[scratch] notes.md:"));
    }

    #[test]
    fn append_event_is_append_only_across_many_writes() {
        std::env::set_var("AGENT_TRACE_ALLOW_DEGRADED", "1");
        let tmp = TempDir::new().unwrap();
        let (root, mut manifest, git) = setup(&tmp);
        let path = PathBuf::from("notes.md");
        std::fs::write(root.join(&path), "body").unwrap();
        manifest.register(&path, DocType::Scratch, "").unwrap();
        manifest.save(&root).unwrap();

        for i in 0..5 {
            std::fs::write(root.join(&path), format!("body {i}")).unwrap();
            apply_trace_hooks(
                &root,
                &git,
                &manifest,
                &Actor::User,
                None,
                &[(path.clone(), Action::Modify, DocType::Scratch)],
                "cli_write",
                None,
            )
            .unwrap();
        }
        let events = running_summary::load_all_events(&root).unwrap();
        assert!(events.len() >= 5);
        let raw = std::fs::read_to_string(running_summary::events_path(&root)).unwrap();
        assert!(raw.lines().filter(|l| !l.trim().is_empty()).count() >= 5);
    }

    #[test]
    fn build_trace_documents_includes_unmanifested_changed_paths() {
        let tmp = TempDir::new().unwrap();
        let (root, mut manifest, _git) = setup(&tmp);

        std::fs::write(root.join("plan.md"), "# Plan\n- [ ] step one\n").unwrap();
        manifest
            .register(&PathBuf::from("plan.md"), DocType::Plan, "")
            .unwrap();

        std::fs::write(root.join("worker.py"), "print('worker activity')\n").unwrap();

        let docs = crate::trace::context::build_trace_documents(
            &root,
            &manifest,
            &[PathBuf::from("worker.py")],
        );

        assert!(
            docs.iter().any(|d| d.path == "plan.md"),
            "should include manifest plan document"
        );
        let worker = docs
            .iter()
            .find(|d| d.path == "worker.py")
            .expect("should include unmanifested changed path");
        assert_eq!(worker.doc_type, DocType::Scratch);
        assert!(worker.content_snippet.contains("worker activity"));
        assert!(!manifest.is_tracked(&PathBuf::from("worker.py")));
    }

    #[test]
    fn build_trace_documents_does_not_duplicate_managed_paths() {
        let tmp = TempDir::new().unwrap();
        let (root, mut manifest, _git) = setup(&tmp);
        std::fs::write(root.join("notes.md"), "scratch note\n").unwrap();
        manifest
            .register(&PathBuf::from("notes.md"), DocType::Scratch, "")
            .unwrap();

        let docs = crate::trace::context::build_trace_documents(
            &root,
            &manifest,
            &[PathBuf::from("notes.md")],
        );
        let count = docs.iter().filter(|d| d.path == "notes.md").count();
        assert_eq!(count, 1, "managed + changed path must not be duplicated");
    }

    #[test]
    fn agent_log_appends_without_full_rewrite() {
        std::env::set_var("AGENT_TRACE_ALLOW_DEGRADED", "1");
        let tmp = TempDir::new().unwrap();
        let (root, mut manifest, git) = setup(&tmp);
        let path = PathBuf::from("prd.md");
        std::fs::write(root.join(&path), "v1").unwrap();
        manifest.register(&path, DocType::Plan, "bot").unwrap();
        manifest.save(&root).unwrap();
        let actor = Actor::Agent {
            name: "bot".into(),
        };
        apply_trace_hooks(
            &root,
            &git,
            &manifest,
            &actor,
            Some("ses1"),
            &[(path.clone(), Action::Modify, DocType::Plan)],
            "mcp_write",
            None,
        )
        .unwrap();
        apply_trace_hooks(
            &root,
            &git,
            &manifest,
            &actor,
            Some("ses1"),
            &[(path, Action::Modify, DocType::Plan)],
            "mcp_write",
            None,
        )
        .unwrap();
        let log = std::fs::read_to_string(root.join("logs").join("bot-ses1.md")).unwrap();
        assert!(log.contains("Agent Log: bot"));
        assert_eq!(log.matches("## ").count(), 2);
    }
}
