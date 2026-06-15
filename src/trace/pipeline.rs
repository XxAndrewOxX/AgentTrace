use crate::git_store::CommitInfo;
use crate::llm::trace_insights::{TraceDocument, TraceInsightsFacade};
use crate::permissions::{check_permission, PermissionResult};
use crate::store::Store;
use crate::trace::context::load_pending_updates;
use crate::trace::running_summary::{self, SummaryEvent};
use crate::trace::{
    agent_trace_md,
    logs::{append_agent_log, summarize_change_no_llm, LogSynthEntry},
};
use crate::types::{Action, Actor, DocType};
use chrono::Utc;
use std::path::{Path, PathBuf};
use thiserror::Error;

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
    )
    .map_err(WriteDocumentError::Other)?;

    Ok(rel)
}

pub fn apply_trace_hooks(
    store_root: &Path,
    git: &crate::git_store::GitStore,
    manifest: &crate::manifest::Manifest,
    actor: &Actor,
    session_id: Option<&str>,
    changed_files: &[(PathBuf, Action, DocType)],
    source: &str,
) -> anyhow::Result<()> {
    if changed_files.is_empty() {
        return Ok(());
    }

    // Pipeline synthesis gate: never emit degraded artifacts from the post-write
    // pipeline. When no reachable backend is configured (and the test/escape
    // hatch is not set) this fails fast instead of silently degrading.
    crate::runtime::require_synthesis_backend(Some(store_root))?;

    let trace_insights = TraceInsightsFacade::from_store_root(store_root)?;

    if actor.is_agent() {
        if let (Some(agent_name), Some(sid)) = (actor.agent_name(), session_id) {
            let entries: Vec<LogSynthEntry> = changed_files
                .iter()
                .map(|(path, _, doc_type)| {
                    let stats = git.diff_stats(path, None, None).unwrap_or_default();
                    let summary = {
                        let diff = format!(
                            "+{} lines\n-{} lines\n",
                            stats.lines_added, stats.lines_removed
                        );
                        match trace_insights.summarize_change(path, doc_type, &diff) {
                            Ok(s) => s,
                            Err(e) => {
                                tracing::warn!(
                                    "LLM summarize_change failed for {}, using template: {}",
                                    path.display(),
                                    e
                                );
                                summarize_change_no_llm(path, doc_type, &stats, agent_name)
                            }
                        }
                    };
                    Ok(LogSynthEntry {
                        timestamp: Utc::now(),
                        path: path.clone(),
                        summary,
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            append_agent_log(store_root, git, agent_name, sid, &entries)?;
        }
    }

    for (path, action, doc_type) in changed_files {
        let stats = git.diff_stats(path, None, None).unwrap_or_default();
        let event_summary = {
            let diff = format!(
                "+{} lines\n-{} lines\n",
                stats.lines_added, stats.lines_removed
            );
            match trace_insights.summarize_change(path, doc_type, &diff) {
                Ok(s) => s,
                Err(e) => {
                    if trace_insights.is_degraded() {
                        summarize_change_no_llm(
                            path,
                            doc_type,
                            &stats,
                            actor.agent_name().unwrap_or("system"),
                        )
                    } else {
                        tracing::warn!(
                            "LLM summarize_change failed for event {}, using template: {}",
                            path.display(),
                            e
                        );
                        summarize_change_no_llm(
                            path,
                            doc_type,
                            &stats,
                            actor.agent_name().unwrap_or("system"),
                        )
                    }
                }
            }
        };
        let event = SummaryEvent {
            timestamp: Utc::now().to_rfc3339(),
            session_id: session_id.map(String::from),
            agent_name: actor.agent_name().map(String::from),
            actor: actor.to_string(),
            action: action.to_string(),
            change_kind: action.to_string(),
            path: path.display().to_string(),
            doc_type: doc_type.to_string(),
            summary: event_summary,
            source: source.to_string(),
            detected_by: detected_by_from_source(source).to_string(),
            lines_added: stats.lines_added,
            lines_removed: stats.lines_removed,
        };
        running_summary::append_event(store_root, event)?;
    }

    if let Err(e) = running_summary::refresh_template(store_root, git, manifest) {
        tracing::warn!("running summary template refresh failed: {e}");
    }
    running_summary::schedule_synthesis_refresh(store_root.to_path_buf());

    sync_agent_trace_md(store_root, git, manifest)?;

    let refresh_context = changed_files.iter().any(|(_, _, doc_type)| {
        matches!(
            doc_type,
            DocType::Plan | DocType::Reference | DocType::Scratch
        )
    });
    if refresh_context {
        sync_context_md(store_root, git, manifest, &trace_insights)?;
    }

    Ok(())
}

fn sync_agent_trace_md(
    store_root: &Path,
    git: &crate::git_store::GitStore,
    manifest: &crate::manifest::Manifest,
) -> anyhow::Result<()> {
    let new_content = agent_trace_md::generate(store_root, manifest);
    let target = store_root.join("AGENT-TRACE.md");
    let existing = std::fs::read_to_string(&target).unwrap_or_default();
    if existing == new_content {
        return Ok(());
    }

    let tmp = store_root.join(".agent-trace").join("AGENT-TRACE.md.tmp");
    std::fs::write(&tmp, &new_content)?;
    std::fs::rename(&tmp, &target)?;

    let info = CommitInfo {
        action: Action::Modify,
        files: vec![(
            PathBuf::from("AGENT-TRACE.md"),
            Action::Modify,
            DocType::Reference,
        )],
        actor: Actor::System,
        summary: "update AGENT-TRACE.md index".into(),
        agent_name: None,
        session_id: None,
    };
    git.commit(&info)?;
    Ok(())
}

fn sync_context_md(
    store_root: &Path,
    git: &crate::git_store::GitStore,
    manifest: &crate::manifest::Manifest,
    trace_insights: &TraceInsightsFacade,
) -> anyhow::Result<()> {
    // `is_degraded()` is only reachable under the test/escape-hatch path (the
    // gate above already bailed otherwise). It must use the template — never the
    // backend, which would emit a "*(Synthesis degraded …)*" artifact.
    let (new_content, commit_label) = if trace_insights.is_degraded() {
        (
            crate::trace::context::synthesize_no_llm(store_root, manifest)?,
            "template".to_string(),
        )
    } else {
        let docs = build_trace_documents(store_root, manifest);
        let updates = load_pending_updates(store_root)?
            .into_iter()
            .map(|u| u.update)
            .collect::<Vec<_>>();
        let start = std::time::Instant::now();
        match trace_insights.synthesize_context(&docs, &updates) {
            Ok(s) => {
                tracing::info!(
                    "LLM context synthesis succeeded (backend={}, latency_ms={})",
                    trace_insights.backend_label,
                    start.elapsed().as_millis()
                );
                (s, format!("llm: {}", trace_insights.backend_label))
            }
            Err(e) => {
                tracing::warn!("LLM synthesize_context failed, using template: {e}");
                (
                    crate::trace::context::synthesize_no_llm(store_root, manifest)?,
                    "template".to_string(),
                )
            }
        }
    };
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
    Ok(())
}

fn build_trace_documents(
    store_root: &Path,
    manifest: &crate::manifest::Manifest,
) -> Vec<TraceDocument> {
    manifest
        .documents()
        .iter()
        .filter(|d| {
            matches!(
                d.doc_type,
                DocType::Plan | DocType::Reference | DocType::Scratch
            )
        })
        .map(|d| {
            let content = std::fs::read_to_string(store_root.join(&d.path)).unwrap_or_default();
            let snippet: String = content.chars().take(2000).collect();
            TraceDocument {
                path: d.path.display().to_string(),
                doc_type: d.doc_type.clone(),
                content_snippet: snippet,
            }
        })
        .collect()
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
            synthesis: None,
            polling: crate::config::PollingConfig::default(),
        };
        store_cfg.save(&root).unwrap();
        (root, manifest, git)
    }

    #[test]
    fn scratch_write_triggers_context_refresh() {
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
        )
        .unwrap();

        let ctx = std::fs::read_to_string(root.join("context.md")).expect("context.md created");
        assert!(ctx.contains("reconnect watermark test"));
        assert!(ctx.contains("[scratch] notes.md:"));
    }
}
