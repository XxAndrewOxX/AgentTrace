use crate::git_store::{CommitInfo, GitStore};
use crate::llm::trace_insights::TraceInsightsFacade;
use crate::manifest::Manifest;
use crate::types::{Action, Actor, DocType};
use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

static REFRESH_IN_FLIGHT: LazyLock<Mutex<HashMap<PathBuf, bool>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

const EVENTS_FILE: &str = ".agent-trace/summary_events.jsonl";
const SUMMARY_STATE_FILE: &str = ".agent-trace/summary_state.toml";
const RUNNING_SUMMARY_FILE: &str = "running_summary.md";
const MAX_EVENTS_RETAINED: usize = 500;
const RECENT_ACTIVITY_LIMIT: usize = 15;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SummaryState {
    pub events_count_at_refresh: usize,
    #[serde(default)]
    pub ops_since_refresh: usize,
}

pub fn summary_state_path(store_root: &Path) -> PathBuf {
    store_root.join(SUMMARY_STATE_FILE)
}

pub fn load_summary_state(store_root: &Path) -> Result<SummaryState> {
    let path = summary_state_path(store_root);
    if !path.exists() {
        return Ok(SummaryState::default());
    }
    let content = std::fs::read_to_string(&path)?;
    Ok(toml::from_str(&content).unwrap_or_default())
}

pub fn save_summary_state(store_root: &Path, state: &SummaryState) -> Result<()> {
    let path = summary_state_path(store_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(state)?;
    crate::util::atomic_write(&path, &content)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SummaryEvent {
    pub timestamp: String,
    pub session_id: Option<String>,
    pub agent_name: Option<String>,
    pub actor: String,
    pub action: String,
    pub path: String,
    pub doc_type: String,
    pub summary: String,
    pub source: String,
    pub lines_added: usize,
    pub lines_removed: usize,
}

pub fn events_path(store_root: &Path) -> PathBuf {
    store_root.join(EVENTS_FILE)
}

pub fn append_event(store_root: &Path, event: SummaryEvent) -> Result<()> {
    let path = events_path(store_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    increment_ops(store_root)?;
    let mut events = load_all_events(store_root)?;
    events.push(event);
    if events.len() > MAX_EVENTS_RETAINED {
        let skip = events.len() - MAX_EVENTS_RETAINED;
        events = events.split_off(skip);
    }
    let content = events
        .iter()
        .map(|e| serde_json::to_string(e).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    let content = if content.is_empty() {
        String::new()
    } else {
        content + "\n"
    };
    std::fs::write(&path, content)?;
    Ok(())
}

fn load_all_events(store_root: &Path) -> Result<Vec<SummaryEvent>> {
    let path = events_path(store_root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)?;
    Ok(content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect())
}

pub fn event_count(store_root: &Path) -> Result<usize> {
    Ok(load_all_events(store_root)?.len())
}

fn save_events_watermark(store_root: &Path, events_synthesized: usize) -> Result<()> {
    let mut state = load_summary_state(store_root)?;
    state.events_count_at_refresh = events_synthesized;
    state.ops_since_refresh = 0;
    save_summary_state(store_root, &state)
}

pub fn increment_ops(store_root: &Path) -> Result<usize> {
    let mut state = load_summary_state(store_root)?;
    state.ops_since_refresh += 1;
    let n = state.ops_since_refresh;
    save_summary_state(store_root, &state)?;
    Ok(n)
}

fn refresh_threshold(store_root: &Path) -> usize {
    crate::config::MergedConfig::load(store_root)
        .map(|c| c.synthesis.refresh_every_ops)
        .unwrap_or(10)
        .max(1)
}

pub fn load_recent_events(store_root: &Path, limit: usize) -> Result<Vec<SummaryEvent>> {
    let mut events = load_all_events(store_root)?;
    if events.len() > limit {
        let skip = events.len() - limit;
        events = events.split_off(skip);
    }
    Ok(events)
}

pub fn format_events_for_prompt(events: &[SummaryEvent]) -> String {
    events
        .iter()
        .map(|e| format!("[{}] {} {} — {}", e.timestamp, e.action, e.path, e.summary))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn read_plan_snippet(store_root: &Path, manifest: &Manifest) -> String {
    let plans = manifest.list(Some(&DocType::Plan));
    let plan_path = plans
        .first()
        .map(|p| p.path.clone())
        .unwrap_or_else(|| PathBuf::from("plan.md"));
    let content = std::fs::read_to_string(store_root.join(&plan_path)).unwrap_or_default();
    content.chars().take(2000).collect()
}

fn extract_resume_from_plan(plan_snippet: &str) -> String {
    for line in plan_snippet.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("- [ ]")
            || trimmed.to_lowercase().contains("phase")
            || trimmed.to_uppercase().contains("RESUME")
        {
            return trimmed.to_string();
        }
    }
    "Review plan.md for next steps.".to_string()
}

pub fn synthesize_template_summary(
    store_root: &Path,
    manifest: &Manifest,
    events: &[SummaryEvent],
) -> Result<String> {
    let plan_snippet = read_plan_snippet(store_root, manifest);
    let resume = extract_resume_from_plan(&plan_snippet);
    let now = Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
    let session = events
        .last()
        .and_then(|e| e.agent_name.as_deref())
        .unwrap_or("system");
    let session_id = events
        .last()
        .and_then(|e| e.session_id.as_deref())
        .unwrap_or("—");

    let mut out = String::from("# Running Summary\n\n");
    out.push_str(&format!(
        "*Last updated: {now} by agent-trace*\n\
         *Session: {session} / {session_id}*\n\n"
    ));

    out.push_str("## Current Status\n\n");
    if plan_snippet.is_empty() {
        out.push_str(
            "No plan document tracked yet. Add a plan via `agent-trace add plan plan.md`.\n\n",
        );
    } else {
        let status: String = plan_snippet.chars().take(500).collect();
        out.push_str(&status);
        out.push_str("\n\n");
    }

    out.push_str("## Recent Activity (rolling)\n\n");
    if events.is_empty() {
        out.push_str("*(no activity yet)*\n\n");
    } else {
        let recent: Vec<_> = events.iter().rev().take(RECENT_ACTIVITY_LIMIT).collect();
        for e in recent {
            let time = e
                .timestamp
                .split('T')
                .nth(1)
                .and_then(|t| t.split('Z').next())
                .unwrap_or(&e.timestamp);
            out.push_str(&format!("- [{time}] {}: {}\n", e.path, e.summary));
        }
        out.push('\n');
    }

    out.push_str("## Resume Here\n\n");
    out.push_str(&resume);
    out.push_str("\n\n");

    out.push_str("## Key Documents\n\n");
    for p in manifest.list(Some(&DocType::Plan)) {
        out.push_str(&format!("- {} — phase checklist\n", p.path.display()));
    }
    for p in manifest.list(Some(&DocType::Scratch)) {
        if p.path.to_string_lossy().contains("progress") {
            out.push_str(&format!("- {} — detailed progress\n", p.path.display()));
        }
    }
    if manifest.list(Some(&DocType::Plan)).is_empty() {
        out.push_str("*(no plan documents)*\n");
    }
    out.push('\n');

    out.push_str("## Open Items\n\n");
    for line in plan_snippet.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("- [ ]") {
            out.push_str(&format!("{trimmed}\n"));
        }
    }
    if !plan_snippet.contains("- [ ]") {
        out.push_str("*(see plan.md for open items)*\n");
    }

    Ok(out)
}

pub fn write_running_summary(
    store_root: &Path,
    content: &str,
    git: &GitStore,
    manifest: &mut Manifest,
) -> Result<()> {
    let rel = PathBuf::from(RUNNING_SUMMARY_FILE);
    let existed = store_root.join(&rel).exists();
    std::fs::write(store_root.join(&rel), content)?;

    if !manifest.is_tracked(&rel) {
        manifest.register(&rel, DocType::Context, "")?;
        manifest.save(store_root)?;
    }

    let action = if existed {
        Action::Modify
    } else {
        Action::Create
    };
    let info = CommitInfo {
        action: action.clone(),
        files: vec![(rel, action, DocType::Context)],
        actor: Actor::System,
        summary: "refresh running summary".into(),
        agent_name: None,
        session_id: None,
    };
    git.commit(&info)?;
    Ok(())
}

pub fn refresh_template(store_root: &Path, git: &GitStore, manifest: &Manifest) -> Result<()> {
    let events = load_recent_events(store_root, 50)?;
    let content = synthesize_template_summary(store_root, manifest, &events)?;
    let mut manifest_mut = Manifest::load(store_root)?;
    write_running_summary(store_root, &content, git, &mut manifest_mut)?;
    save_events_watermark(store_root, events.len())
}

pub fn refresh(store_root: &Path, git: &GitStore, manifest: &Manifest) -> Result<()> {
    let events = load_recent_events(store_root, 50)?;
    let previous =
        std::fs::read_to_string(store_root.join(RUNNING_SUMMARY_FILE)).unwrap_or_default();
    let plan_snippet = read_plan_snippet(store_root, manifest);
    let events_str = format_events_for_prompt(&events);

    let content = if let Ok(api) = TraceInsightsFacade::from_store_root(store_root) {
        match api.update_running_summary(&previous, &events_str, &plan_snippet) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("synthesis running summary failed: {e}");
                synthesize_template_summary(store_root, manifest, &events)?
            }
        }
    } else {
        synthesize_template_summary(store_root, manifest, &events)?
    };

    let mut manifest_mut = Manifest::load(store_root)?;
    write_running_summary(store_root, &content, git, &mut manifest_mut)?;
    save_events_watermark(store_root, events.len())
}

pub fn refresh_from_path(store_root: &Path) -> Result<()> {
    let git = GitStore::open(store_root)?;
    let manifest = Manifest::load(store_root)?;
    refresh(store_root, &git, &manifest)
}

#[cfg(test)]
pub fn wait_refresh_idle(store_root: &Path) {
    for _ in 0..150 {
        let busy = REFRESH_IN_FLIGHT
            .lock()
            .expect("refresh lock poisoned")
            .get(&store_root.to_path_buf())
            .copied()
            .unwrap_or(false);
        if !busy {
            let current = event_count(store_root).unwrap_or(0);
            let watermark = load_summary_state(store_root)
                .map(|s| s.events_count_at_refresh)
                .unwrap_or(0);
            if current <= watermark {
                return;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

pub fn schedule_refresh(store_root: PathBuf) {
    schedule_refresh_inner(store_root, false);
}

fn schedule_refresh_inner(store_root: PathBuf, force: bool) {
    let threshold = refresh_threshold(&store_root);
    if !force {
        let ops = load_summary_state(&store_root)
            .map(|s| s.ops_since_refresh)
            .unwrap_or(0);
        if ops < threshold {
            return;
        }
    }

    let should_spawn = {
        let mut in_flight = REFRESH_IN_FLIGHT.lock().expect("refresh lock poisoned");
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
        let events_before = event_count(&store_root).unwrap_or(0);
        if let Err(e) = refresh_from_path(&store_root) {
            tracing::warn!("running summary background refresh failed: {e}");
        }
        let events_after = event_count(&store_root).unwrap_or(events_before);
        let watermark = load_summary_state(&store_root)
            .map(|s| s.events_count_at_refresh)
            .unwrap_or(0);
        REFRESH_IN_FLIGHT
            .lock()
            .expect("refresh lock poisoned")
            .insert(store_root.clone(), false);
        let pending_ops = load_summary_state(&store_root)
            .map(|s| s.ops_since_refresh)
            .unwrap_or(0);
        if events_after > watermark {
            schedule_refresh_inner(store_root.clone(), true);
        } else if pending_ops >= threshold {
            schedule_refresh_inner(store_root, false);
        }
    });
}

pub fn refresh_if_stale(store_root: &Path) -> Result<()> {
    let summary_path = store_root.join(RUNNING_SUMMARY_FILE);
    let current_count = event_count(store_root)?;
    if current_count == 0 {
        return Ok(());
    }
    let watermark = load_summary_state(store_root)?.events_count_at_refresh;
    if !summary_path.exists() || current_count > watermark {
        refresh_from_path(store_root)?;
    }
    Ok(())
}

pub fn assemble_resume_context(
    store_root: &Path,
    actor: &Actor,
    include_git_log: bool,
    git_log_limit: usize,
) -> Result<String> {
    use crate::session;

    let mut out = String::from("=== Agent Trace Resume Context ===\n\n");

    out.push_str("SESSION\n");
    if let Some(sess) = session::load_session(store_root) {
        if sess.is_stale() {
            out.push_str("  (stale session — reconnect to start fresh)\n");
        }
        out.push_str(&format!("  Agent: {}\n", sess.name));
        out.push_str(&format!("  Session ID: {}\n", sess.session_id));
        out.push_str(&format!("  Transport: {}\n", sess.transport));
        out.push_str(&format!("  Started: {}\n", sess.started_at));
    } else if let Some(name) = actor.agent_name() {
        out.push_str(&format!("  Agent: {name}\n"));
        out.push_str("  Session ID: (none — call connect or use MCP with --actor)\n");
    } else {
        out.push_str("  Actor: user (no agent session)\n");
    }
    out.push('\n');

    out.push_str("--- Running Summary ---\n");
    let summary_path = store_root.join(RUNNING_SUMMARY_FILE);
    let summary_body = if summary_path.exists() {
        std::fs::read_to_string(&summary_path)?
    } else {
        let git = GitStore::open(store_root)?;
        let manifest = Manifest::load(store_root)?;
        refresh_template(store_root, &git, &manifest)?;
        std::fs::read_to_string(&summary_path).unwrap_or_default()
    };
    out.push_str(&summary_body);
    out.push_str("\n\n");

    let context_path = store_root.join("context.md");
    if context_path.exists() {
        out.push_str("--- Context (context.md) ---\n");
        let ctx = std::fs::read_to_string(&context_path)?;
        let truncated: String = ctx.chars().take(4000).collect();
        out.push_str(&truncated);
        if ctx.len() > 4000 {
            out.push_str("\n...(truncated)");
        }
        out.push_str("\n\n");
    }

    let manifest = Manifest::load(store_root)?;
    let plans = manifest.list(Some(&DocType::Plan));
    if let Some(plan) = plans.first() {
        out.push_str(&format!("--- Plan excerpt ({}) ---\n", plan.path.display()));
        let plan_content = std::fs::read_to_string(store_root.join(&plan.path)).unwrap_or_default();
        let excerpt: String = plan_content.chars().take(2000).collect();
        out.push_str(&excerpt);
        if plan_content.len() > 2000 {
            out.push_str("\n...(truncated)");
        }
        out.push_str("\n\n");
    }

    if include_git_log {
        if let Ok(git) = GitStore::open(store_root) {
            let entries = git.log(git_log_limit)?;
            if !entries.is_empty() {
                out.push_str(&format!(
                    "--- Recent git activity ({git_log_limit} entries) ---\n"
                ));
                for entry in entries {
                    out.push_str(&format!(
                        "{} {} {} — {}\n",
                        entry.timestamp.format("%Y-%m-%d %H:%M:%S"),
                        entry.action,
                        entry.actor,
                        entry.summary
                    ));
                }
                out.push('\n');
            }
        }
    }

    if let Some(sess) = session::load_session(store_root) {
        let log_path = store_root
            .join("logs")
            .join(format!("{}-{}.md", sess.name, sess.session_id));
        if log_path.exists() {
            out.push_str(&format!(
                "--- Session log tail (logs/{}-{}.md) ---\n",
                sess.name, sess.session_id
            ));
            let log_content = std::fs::read_to_string(&log_path)?;
            let lines: Vec<&str> = log_content.lines().collect();
            let tail_start = lines.len().saturating_sub(20);
            for line in &lines[tail_start..] {
                out.push_str(line);
                out.push('\n');
            }
            out.push('\n');
        }
    }

    out.push_str("INSTRUCTIONS\n");
    out.push_str(
        "Do not re-read files above unless editing them. \
         Continue from \"Resume Here\". Read source files only for the current phase.\n",
    );

    Ok(out)
}

pub fn resume_here_lines(store_root: &Path) -> Vec<String> {
    let path = store_root.join(RUNNING_SUMMARY_FILE);
    if !path.exists() {
        return Vec::new();
    }
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let mut in_section = false;
    let mut lines = Vec::new();
    for line in content.lines() {
        if line.starts_with("## Resume Here") {
            in_section = true;
            continue;
        }
        if in_section {
            if line.starts_with("## ") {
                break;
            }
            if !line.trim().is_empty() {
                lines.push(line.to_string());
                if lines.len() >= 3 {
                    break;
                }
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StoreInfo;
    use tempfile::TempDir;

    fn setup(tmp: &TempDir) -> (PathBuf, Manifest, GitStore) {
        let root = tmp.path().to_path_buf();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        let git = GitStore::init(&root).unwrap();
        let info = StoreInfo::new("test".into());
        let manifest = Manifest::create_empty(info, &root).unwrap();
        (root, manifest, git)
    }

    #[test]
    fn append_event_creates_jsonl() {
        let tmp = TempDir::new().unwrap();
        let (root, _, _) = setup(&tmp);
        let event = SummaryEvent {
            timestamp: Utc::now().to_rfc3339(),
            session_id: Some("20260605-120000".into()),
            agent_name: Some("claude".into()),
            actor: "agent:claude".into(),
            action: "modify".into(),
            path: "plan.md".into(),
            doc_type: "plan".into(),
            summary: "Updated phase 2".into(),
            source: "mcp_write".into(),
            lines_added: 5,
            lines_removed: 1,
        };
        append_event(&root, event).unwrap();
        assert!(events_path(&root).exists());
        let events = load_recent_events(&root, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].path, "plan.md");
    }

    #[test]
    fn template_summary_includes_recent_events() {
        let tmp = TempDir::new().unwrap();
        let (root, manifest, _) = setup(&tmp);
        std::fs::write(
            root.join("plan.md"),
            "# Plan\n- [x] Phase 1\n- [ ] Phase 2 idempotency\n",
        )
        .unwrap();
        let mut m = manifest;
        m.register(&PathBuf::from("plan.md"), DocType::Plan, "")
            .unwrap();

        let events = vec![SummaryEvent {
            timestamp: "2026-06-05T20:58:09Z".into(),
            session_id: Some("s1".into()),
            agent_name: Some("claude".into()),
            actor: "agent:claude".into(),
            action: "modify".into(),
            path: "plan.md".into(),
            doc_type: "plan".into(),
            summary: "Phase 2 complete".into(),
            source: "mcp_write".into(),
            lines_added: 3,
            lines_removed: 0,
        }];
        let summary = synthesize_template_summary(&root, &m, &events).unwrap();
        assert!(summary.contains("# Running Summary"));
        assert!(summary.contains("Phase 2 complete"));
        assert!(summary.contains("## Resume Here"));
        assert!(summary.contains("Phase 2 idempotency"));
    }

    #[test]
    fn write_running_summary_commits_and_registers() {
        let tmp = TempDir::new().unwrap();
        let (root, manifest, git) = setup(&tmp);
        let mut m = manifest;
        let content = "# Running Summary\n\ntest\n";
        write_running_summary(&root, content, &git, &mut m).unwrap();
        assert!(root.join(RUNNING_SUMMARY_FILE).exists());
        assert!(m.is_tracked(&PathBuf::from(RUNNING_SUMMARY_FILE)));
        assert_eq!(
            m.find_by_path(&PathBuf::from(RUNNING_SUMMARY_FILE))
                .unwrap()
                .doc_type,
            DocType::Context
        );
    }

    #[test]
    fn refresh_without_llm_writes_template() {
        let tmp = TempDir::new().unwrap();
        let (root, manifest, git) = setup(&tmp);
        std::fs::write(root.join("plan.md"), "# Plan\n- [ ] Next step\n").unwrap();
        let mut m = manifest;
        m.register(&PathBuf::from("plan.md"), DocType::Plan, "")
            .unwrap();
        append_event(
            &root,
            SummaryEvent {
                timestamp: Utc::now().to_rfc3339(),
                session_id: Some("s1".into()),
                agent_name: Some("bot".into()),
                actor: "agent:bot".into(),
                action: "modify".into(),
                path: "plan.md".into(),
                doc_type: "plan".into(),
                summary: "updated plan".into(),
                source: "mcp_write".into(),
                lines_added: 2,
                lines_removed: 0,
            },
        )
        .unwrap();
        refresh(&root, &git, &m).unwrap();
        let content = std::fs::read_to_string(root.join(RUNNING_SUMMARY_FILE)).unwrap();
        assert!(content.contains("# Running Summary"));
        assert!(content.contains("updated plan"));
    }

    fn sample_event(path: &str, summary: &str) -> SummaryEvent {
        SummaryEvent {
            timestamp: Utc::now().to_rfc3339(),
            session_id: Some("s1".into()),
            agent_name: Some("bot".into()),
            actor: "agent:bot".into(),
            action: "modify".into(),
            path: path.into(),
            doc_type: "scratch".into(),
            summary: summary.into(),
            source: "mcp_write".into(),
            lines_added: 1,
            lines_removed: 0,
        }
    }

    #[test]
    fn refresh_if_stale_refreshes_when_events_exceed_watermark() {
        let tmp = TempDir::new().unwrap();
        let (root, manifest, git) = setup(&tmp);
        std::fs::write(root.join("plan.md"), "# Plan\n- [ ] Next\n").unwrap();
        let mut m = manifest;
        m.register(&PathBuf::from("plan.md"), DocType::Plan, "")
            .unwrap();
        append_event(&root, sample_event("plan.md", "first event")).unwrap();
        refresh_template(&root, &git, &m).unwrap();
        assert_eq!(
            load_summary_state(&root).unwrap().events_count_at_refresh,
            1
        );

        append_event(&root, sample_event("notes.md", "second event")).unwrap();
        refresh_if_stale(&root).unwrap();

        assert_eq!(
            load_summary_state(&root).unwrap().events_count_at_refresh,
            2
        );
        let summary = std::fs::read_to_string(root.join(RUNNING_SUMMARY_FILE)).unwrap();
        assert!(summary.contains("second event"));
    }

    #[test]
    fn refresh_if_stale_skips_when_watermark_is_current() {
        let tmp = TempDir::new().unwrap();
        let (root, manifest, git) = setup(&tmp);
        std::fs::write(root.join("plan.md"), "# Plan\n").unwrap();
        let mut m = manifest;
        m.register(&PathBuf::from("plan.md"), DocType::Plan, "")
            .unwrap();
        append_event(&root, sample_event("plan.md", "only event")).unwrap();
        refresh_template(&root, &git, &m).unwrap();
        let before = std::fs::read_to_string(root.join(RUNNING_SUMMARY_FILE)).unwrap();

        refresh_if_stale(&root).unwrap();

        let after = std::fs::read_to_string(root.join(RUNNING_SUMMARY_FILE)).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn schedule_refresh_reschedules_when_events_arrive_during_refresh() {
        let tmp = TempDir::new().unwrap();
        let (root, manifest, git) = setup(&tmp);
        let store_cfg = crate::config::StoreConfig {
            store: manifest.store.clone(),
            llm: None,
            synthesis: Some(crate::config::SynthesisConfig {
                refresh_every_ops: 1,
                ..Default::default()
            }),
            polling: crate::config::PollingConfig::default(),
        };
        store_cfg.save(&root).unwrap();
        std::fs::write(root.join("plan.md"), "# Plan\n- [ ] Next\n").unwrap();
        let mut m = manifest;
        m.register(&PathBuf::from("plan.md"), DocType::Plan, "")
            .unwrap();
        append_event(&root, sample_event("plan.md", "seed event")).unwrap();
        refresh_template(&root, &git, &m).unwrap();
        append_event(&root, sample_event("plan.md", "pre-refresh event")).unwrap();

        wait_refresh_idle(&root);
        schedule_refresh(root.clone());
        append_event(&root, sample_event("notes.md", "late event")).unwrap();

        wait_refresh_idle(&root);
        refresh_if_stale(&root).unwrap();

        let expected_events = event_count(&root).unwrap();
        assert_eq!(
            load_summary_state(&root).unwrap().events_count_at_refresh,
            expected_events
        );
        let summary = std::fs::read_to_string(root.join(RUNNING_SUMMARY_FILE)).unwrap();
        assert!(
            summary.contains("late event"),
            "summary missing late event:\n{summary}"
        );
    }

    #[test]
    fn events_retention_truncates_old() {
        let tmp = TempDir::new().unwrap();
        let (root, _, _) = setup(&tmp);
        for i in 0..MAX_EVENTS_RETAINED + 10 {
            append_event(
                &root,
                SummaryEvent {
                    timestamp: format!("2026-06-05T00:{i:02}Z"),
                    session_id: None,
                    agent_name: None,
                    actor: "system".into(),
                    action: "modify".into(),
                    path: format!("f{i}.md"),
                    doc_type: "scratch".into(),
                    summary: format!("event {i}"),
                    source: "poll".into(),
                    lines_added: 0,
                    lines_removed: 0,
                },
            )
            .unwrap();
        }
        let events = load_all_events(&root).unwrap();
        assert_eq!(events.len(), MAX_EVENTS_RETAINED);
        assert_eq!(events[0].path, "f10.md");
    }
}
