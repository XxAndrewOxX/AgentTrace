use crate::briefing::{build_current_state, load_briefing_events};
use crate::manifest::Manifest;
use crate::running_summary::resume_here_lines;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use std::path::Path;

/// Pinned running-context panel state.
#[derive(Debug, Clone)]
pub struct ContextState {
    pub phase: String,
    pub resume: String,
    pub status: String,
    pub last_focus: String,
    pub open_items: Vec<String>,
    pub blockers: Vec<String>,
    pub scroll: usize,
    pub expanded: bool,
}

impl ContextState {
    pub fn new(store_root: &Path, manifest: &Manifest, session_id: Option<&str>) -> Self {
        let mut state = Self {
            phase: String::new(),
            resume: String::new(),
            status: String::new(),
            last_focus: String::new(),
            open_items: Vec::new(),
            blockers: Vec::new(),
            scroll: 0,
            expanded: true,
        };
        state.reload(store_root, manifest, session_id);
        state
    }

    pub fn reload(&mut self, store_root: &Path, manifest: &Manifest, session_id: Option<&str>) {
        let events = load_briefing_events(store_root, session_id, 20).unwrap_or_default();
        let plan_path = manifest
            .list(Some(&crate::types::DocType::Plan))
            .first()
            .map(|d| d.path.clone())
            .unwrap_or_else(|| Path::new("plan.md").to_path_buf());
        let plan_content = std::fs::read_to_string(store_root.join(&plan_path)).unwrap_or_default();

        let current = build_current_state(store_root, manifest, &plan_content, &events);
        self.phase = extract_field(&current, "Phase:");
        self.status = extract_field(&current, "Status:");
        self.last_focus = extract_field(&current, "Last focus:");
        self.open_items = extract_list(&current, "Open items:");
        self.blockers = extract_list(&current, "Blockers:");

        let resume_lines = resume_here_lines(store_root);
        self.resume = if resume_lines.is_empty() {
            self.phase.clone()
        } else {
            resume_lines.join(" ")
        };
    }

    pub fn toggle_expanded(&mut self) {
        self.expanded = !self.expanded;
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    pub fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(1);
    }

    pub fn glance_line(&self) -> String {
        let mut parts = Vec::new();
        if !self.phase.is_empty() {
            parts.push(self.phase.clone());
        }
        if !self.resume.is_empty() && self.resume != self.phase {
            parts.push(self.resume.clone());
        }
        if !self.open_items.is_empty() {
            parts.push(self.open_items.join("  "));
        }
        if !self.blockers.is_empty() {
            parts.push(format!("Blockers: {}", self.blockers.join("; ")));
        }
        if parts.is_empty() {
            "No running context yet.".to_string()
        } else {
            parts.join(" — ")
        }
    }

    pub fn render(
        &self,
        f: &mut Frame<'_>,
        area: ratatui::layout::Rect,
        focused: bool,
        compact: bool,
    ) {
        let border_style = if focused {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };

        if compact && !self.expanded {
            let line = Line::from(Span::raw(truncate(
                &self.glance_line(),
                area.width as usize,
            )));
            let block = Block::default()
                .title("Context")
                .borders(Borders::ALL)
                .border_style(border_style);
            f.render_widget(Paragraph::new(vec![line]).block(block), area);
            return;
        }

        let mut lines = Vec::new();
        if !self.phase.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("Phase: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(self.phase.clone()),
            ]));
        }
        if !self.resume.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("Resume: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(self.resume.clone()),
            ]));
        }
        if !self.status.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("Status: ", Style::default().fg(Color::DarkGray)),
                Span::raw(self.status.clone()),
            ]));
        }
        if !self.last_focus.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("Focus: ", Style::default().fg(Color::Cyan)),
                Span::raw(self.last_focus.clone()),
            ]));
        }
        if !self.open_items.is_empty() {
            lines.push(Line::from(Span::styled(
                "Open:",
                Style::default().add_modifier(Modifier::BOLD),
            )));
            for item in &self.open_items {
                lines.push(Line::from(Span::raw(format!("  {item}"))));
            }
        }
        if !self.blockers.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("Blockers: {}", self.blockers.join("; ")),
                Style::default().fg(Color::Red),
            )));
        }
        if lines.is_empty() {
            lines.push(Line::from("No running context yet."));
        }

        let visible_height = area.height.saturating_sub(2) as usize;
        let start = self.scroll.min(lines.len().saturating_sub(1));
        let visible: Vec<Line> = lines.into_iter().skip(start).take(visible_height).collect();

        let block = Block::default()
            .title("Running Context")
            .borders(Borders::ALL)
            .border_style(border_style);
        f.render_widget(Paragraph::new(visible).block(block), area);
    }
}

fn extract_field(body: &str, prefix: &str) -> String {
    body.lines()
        .find(|l| l.starts_with(prefix))
        .map(|l| l[prefix.len()..].trim().to_string())
        .unwrap_or_default()
}

fn extract_list(body: &str, header: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut in_section = false;
    for line in body.lines() {
        if line.starts_with(header) {
            in_section = true;
            continue;
        }
        if in_section {
            if line.ends_with(':') && !line.starts_with("- ") && !line.starts_with("☐") {
                break;
            }
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                items.push(trimmed.to_string());
            }
        }
    }
    items
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!(
            "{}…",
            s.chars().take(max.saturating_sub(1)).collect::<String>()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StoreInfo;
    use crate::manifest::Manifest;
    use crate::types::DocType;
    use tempfile::TempDir;

    #[test]
    fn context_loads_plan_phase() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        let info = StoreInfo::new("test".into());
        let mut manifest = Manifest::create_empty(info, root).unwrap();
        manifest
            .register(&std::path::PathBuf::from("plan.md"), DocType::Plan, "")
            .unwrap();
        std::fs::write(
            root.join("plan.md"),
            "# Plan\n\n## Phase 1\n\n- [ ] First task\n",
        )
        .unwrap();

        let ctx = ContextState::new(root, &manifest, None);
        assert!(ctx.phase.contains("Phase 1") || !ctx.open_items.is_empty());
    }

    #[test]
    fn glance_line_joins_sections() {
        let ctx = ContextState {
            phase: "Phase 2".into(),
            resume: "OAuth next".into(),
            status: String::new(),
            last_focus: String::new(),
            open_items: vec!["☐ tests".into()],
            blockers: vec![],
            scroll: 0,
            expanded: true,
        };
        let line = ctx.glance_line();
        assert!(line.contains("Phase 2"));
        assert!(line.contains("OAuth"));
    }
}
