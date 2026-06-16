use crate::manifest::Manifest;
use crate::running_summary::{load_all_events, SummaryEvent};
use crate::types::DocType;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const DEFAULT_RECENT_EVENTS_LIMIT: usize = 20;

/// Options for assembling the resume briefing (MCP and CLI).
#[derive(Debug, Clone)]
pub struct BriefingOptions {
    pub recent_limit: usize,
    pub include_git_log: bool,
    pub include_prior_recap: bool,
    pub include_session_log: bool,
    pub git_log_limit: usize,
}

impl Default for BriefingOptions {
    fn default() -> Self {
        Self {
            recent_limit: DEFAULT_RECENT_EVENTS_LIMIT,
            include_git_log: false,
            include_prior_recap: true,
            include_session_log: false,
            git_log_limit: 10,
        }
    }
}

/// Extract the overall objective from plan content (§1).
pub fn extract_objective(plan_content: &str) -> String {
    const MAX_CHARS: usize = 400;

    let mut in_goal = false;
    let mut goal_lines = Vec::new();

    for line in plan_content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## Goal") {
            in_goal = true;
            continue;
        }
        if in_goal {
            if trimmed.starts_with("## ") && !trimmed.starts_with("## Goal") {
                break;
            }
            if !trimmed.is_empty() {
                goal_lines.push(trimmed);
            }
        }
    }

    let text = if !goal_lines.is_empty() {
        goal_lines.join(" ")
    } else {
        first_non_empty_paragraph(plan_content)
    };

    truncate_chars(&text, MAX_CHARS)
}

fn first_non_empty_paragraph(content: &str) -> String {
    let mut skipped_title = false;
    let mut paragraph = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !paragraph.is_empty() {
                break;
            }
            continue;
        }
        if !skipped_title && (trimmed.starts_with('#') || trimmed.is_empty()) {
            if trimmed.starts_with('#') {
                skipped_title = true;
            }
            continue;
        }
        skipped_title = true;
        paragraph.push(trimmed);
    }

    if paragraph.is_empty() {
        "Review plan.md for project goals.".into()
    } else {
        paragraph.join(" ")
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect::<String>() + "…"
    }
}

/// First unchecked phase line from plan (reuses running_summary heuristics).
pub fn extract_current_phase(plan_content: &str) -> String {
    for line in plan_content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("- [ ]") {
            return trimmed.to_string();
        }
    }
    for line in plan_content.lines() {
        let trimmed = line.trim();
        if trimmed.to_lowercase().contains("phase") {
            return trimmed.to_string();
        }
    }
    "Review plan.md for next steps.".to_string()
}

fn read_progress_tail(store_root: &Path, manifest: &Manifest) -> Option<String> {
    const MAX_CHARS: usize = 300;

    for entry in manifest.list(Some(&DocType::Scratch)) {
        let name = entry.path.to_string_lossy();
        if !name.contains("progress") {
            continue;
        }
        let content = std::fs::read_to_string(store_root.join(&entry.path)).ok()?;
        let mut paragraphs: Vec<&str> = Vec::new();
        for para in content.split("\n\n") {
            let trimmed = para.trim();
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                paragraphs.push(trimmed);
            }
        }
        if let Some(last) = paragraphs.last() {
            return Some(truncate_chars(last, MAX_CHARS));
        }
    }
    None
}

fn scan_blockers(store_root: &Path, manifest: &Manifest) -> Vec<String> {
    let mut blockers = Vec::new();
    let candidates: Vec<PathBuf> = manifest
        .list(None)
        .into_iter()
        .map(|e| e.path.clone())
        .filter(|p| {
            let s = p.to_string_lossy().to_lowercase();
            s.contains("progress") || s.contains("decisions")
        })
        .collect();

    for path in candidates {
        let Ok(content) = std::fs::read_to_string(store_root.join(&path)) else {
            continue;
        };
        for line in content.lines() {
            if line.to_lowercase().contains("blocker") {
                let trimmed = line.trim();
                if !trimmed.is_empty() && !blockers.contains(&trimmed.to_string()) {
                    blockers.push(trimmed.to_string());
                }
            }
        }
    }
    blockers.truncate(3);
    blockers
}

fn open_items_from_plan(plan_content: &str, limit: usize) -> Vec<String> {
    plan_content
        .lines()
        .map(|l| l.trim())
        .filter(|l| l.starts_with("- [ ]"))
        .take(limit)
        .map(|l| l.to_string())
        .collect()
}

/// Build §2 Current State markdown body.
pub fn build_current_state(
    store_root: &Path,
    manifest: &Manifest,
    plan_content: &str,
    briefing_events: &[SummaryEvent],
) -> String {
    let mut out = String::new();

    out.push_str(&format!("Phase: {}\n", extract_current_phase(plan_content)));

    if let Some(status) = read_progress_tail(store_root, manifest) {
        out.push_str(&format!("Status: {status}\n"));
    }

    if let Some(newest) = briefing_events.first() {
        out.push_str(&format!("Last focus: {}\n", newest.path));
    }

    let open = open_items_from_plan(plan_content, 5);
    if !open.is_empty() {
        out.push_str("Open items:\n");
        for item in &open {
            out.push_str(&format!("{item}\n"));
        }
    }

    let blockers = scan_blockers(store_root, manifest);
    if !blockers.is_empty() {
        out.push_str("Blockers:\n");
        for b in &blockers {
            out.push_str(&format!("- {b}\n"));
        }
    }

    out
}

/// Select events for §3: current session first (newest first), backfill from older sessions.
pub fn select_briefing_events(
    all_events: &[SummaryEvent],
    session_id: Option<&str>,
    limit: usize,
) -> Vec<SummaryEvent> {
    if limit == 0 || all_events.is_empty() {
        return Vec::new();
    }

    let (current, other): (Vec<_>, Vec<_>) = if let Some(sid) = session_id {
        all_events
            .iter()
            .cloned()
            .partition(|e| e.session_id.as_deref() == Some(sid))
    } else {
        (all_events.to_vec(), Vec::new())
    };

    let mut selected: Vec<SummaryEvent> = current.iter().rev().take(limit).cloned().collect();

    if selected.len() < limit && !other.is_empty() {
        let need = limit - selected.len();
        let backfill: Vec<SummaryEvent> = other.iter().rev().take(need).cloned().collect();
        selected.extend(backfill);
    }

    selected
}

/// Events not included in the briefing recent-activity list (for §4 history summary).
pub fn events_excluding_briefing(
    all_events: &[SummaryEvent],
    briefing_events: &[SummaryEvent],
) -> Vec<SummaryEvent> {
    let selected: HashSet<(&str, &str, &str)> = briefing_events
        .iter()
        .map(|e| (e.timestamp.as_str(), e.path.as_str(), e.summary.as_str()))
        .collect();
    all_events
        .iter()
        .filter(|e| {
            !selected.contains(&(
                e.timestamp.as_str(),
                e.path.as_str(),
                e.summary.as_str(),
            ))
        })
        .cloned()
        .collect()
}

/// Format §3 Recent Activity lines (newest first).
pub fn format_recent_activity(events: &[SummaryEvent]) -> String {
    if events.is_empty() {
        return "*(no activity yet)*\n".to_string();
    }
    events
        .iter()
        .map(|e| format!("- [{}] {} — {}", e.timestamp, e.path, e.summary))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// Load and select briefing events for the store.
pub fn load_briefing_events(
    store_root: &Path,
    session_id: Option<&str>,
    limit: usize,
) -> anyhow::Result<Vec<SummaryEvent>> {
    let all = load_all_events(store_root)?;
    Ok(select_briefing_events(&all, session_id, limit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StoreInfo;
    use crate::git_store::GitStore;
    use chrono::Utc;

    fn sample_event(path: &str, summary: &str, session: Option<&str>) -> SummaryEvent {
        SummaryEvent {
            timestamp: Utc::now().to_rfc3339(),
            session_id: session.map(|s| s.to_string()),
            agent_name: Some("bot".into()),
            actor: "agent:bot".into(),
            action: "modify".into(),
            change_kind: "modify".into(),
            path: path.into(),
            doc_type: "plan".into(),
            summary: summary.into(),
            source: "test".into(),
            detected_by: "test".into(),
            lines_added: 1,
            lines_removed: 0,
        }
    }

    #[test]
    fn extract_objective_from_goal_section() {
        let plan = "# Plan\n\n## Goal\n\nBuild a ledger API.\n\n## Phases\n";
        let obj = extract_objective(plan);
        assert!(obj.contains("ledger API"));
    }

    #[test]
    fn extract_objective_fallback_first_paragraph() {
        let plan = "# My Project\n\nShip the feature by Friday.\n\n## Details\n";
        let obj = extract_objective(plan);
        assert!(obj.contains("Ship the feature"));
    }

    #[test]
    fn extract_objective_truncates_long_text() {
        let long = "x".repeat(500);
        let plan = format!("# Plan\n\n## Goal\n\n{long}\n");
        assert!(extract_objective(&plan).chars().count() <= 401);
    }

    #[test]
    fn select_briefing_events_prefers_current_session() {
        let events: Vec<SummaryEvent> = (0..5)
            .map(|i| sample_event(&format!("old{i}.md"), "old", Some("sess-a")))
            .chain((0..3).map(|i| sample_event(&format!("cur{i}.md"), "cur", Some("sess-b"))))
            .collect();

        let selected = select_briefing_events(&events, Some("sess-b"), 3);
        assert_eq!(selected.len(), 3);
        assert!(selected
            .iter()
            .all(|e| e.session_id.as_deref() == Some("sess-b")));
        assert_eq!(selected[0].path, "cur2.md");
    }

    #[test]
    fn select_briefing_events_backfills_from_other_sessions() {
        let events: Vec<SummaryEvent> = (0..15)
            .map(|i| sample_event(&format!("old{i}.md"), "old", Some("sess-a")))
            .chain(std::iter::once(sample_event(
                "cur.md",
                "current",
                Some("sess-b"),
            )))
            .collect();

        let selected = select_briefing_events(&events, Some("sess-b"), 5);
        assert_eq!(selected.len(), 5);
        assert_eq!(selected[0].path, "cur.md");
        assert_eq!(selected[0].summary, "current");
        assert!(selected[1..]
            .iter()
            .all(|e| e.session_id.as_deref() == Some("sess-a")));
    }

    #[test]
    fn format_recent_activity_lists_newest_first() {
        let events = vec![
            sample_event("b.md", "second", Some("s")),
            sample_event("a.md", "first", Some("s")),
        ];
        let out = format_recent_activity(&events);
        assert!(out.find("b.md").unwrap() < out.find("a.md").unwrap());
    }

    #[test]
    fn build_current_state_includes_phase_and_open_items() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        let git = GitStore::init(root).unwrap();
        let info = StoreInfo::new("test".into());
        let manifest = Manifest::create_empty(info, root).unwrap();
        let plan = "# Plan\n\n- [ ] Phase 1: setup\n- [ ] Phase 2: ship\n";
        let events = vec![sample_event("plan.md", "edited plan", Some("s1"))];
        let body = build_current_state(root, &manifest, plan, &events);
        assert!(body.contains("Phase: - [ ] Phase 1"));
        assert!(body.contains("Last focus: plan.md"));
        assert!(body.contains("Open items:"));
        assert!(body.contains("Phase 2"));
        drop(git);
    }

    #[test]
    fn events_excluding_briefing_omits_selected() {
        let events: Vec<SummaryEvent> = (0..5)
            .map(|i| sample_event(&format!("f{i}.md"), "e", None))
            .collect();
        let briefing = select_briefing_events(&events, None, 2);
        let older = events_excluding_briefing(&events, &briefing);
        assert_eq!(older.len(), 3);
    }
}
