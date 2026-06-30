use crate::running_summary::SummaryEvent;
use crate::types::LogEntry;
use chrono::DateTime;
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

const MAX_ACTIVITY_ENTRIES: usize = 200;

/// Semantic activity stream backed by SummaryEvent, with git audit fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityMode {
    Semantic,
    GitAudit,
}

#[derive(Debug, Clone)]
pub struct ActivityState {
    pub events: Vec<SummaryEvent>,
    pub git_entries: Vec<LogEntry>,
    pub mode: ActivityMode,
    pub scroll: usize,
}

impl ActivityState {
    pub fn new(events: Vec<SummaryEvent>, git_entries: Vec<LogEntry>) -> Self {
        Self {
            events,
            git_entries,
            mode: ActivityMode::Semantic,
            scroll: 0,
        }
    }

    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            ActivityMode::Semantic => ActivityMode::GitAudit,
            ActivityMode::GitAudit => ActivityMode::Semantic,
        };
        self.scroll = 0;
    }

    pub fn push_event(&mut self, event: SummaryEvent) {
        self.events.insert(0, event);
        if self.events.len() > MAX_ACTIVITY_ENTRIES {
            self.events.truncate(MAX_ACTIVITY_ENTRIES);
            self.scroll = self.scroll.min(MAX_ACTIVITY_ENTRIES.saturating_sub(1));
        }
    }

    pub fn push_git(&mut self, entry: LogEntry) {
        self.git_entries.insert(0, entry);
        if self.git_entries.len() > MAX_ACTIVITY_ENTRIES {
            self.git_entries.truncate(MAX_ACTIVITY_ENTRIES);
            self.scroll = self.scroll.min(MAX_ACTIVITY_ENTRIES.saturating_sub(1));
        }
    }

    pub fn reload_events(&mut self, events: Vec<SummaryEvent>) {
        self.events = events;
        if self.events.len() > MAX_ACTIVITY_ENTRIES {
            let skip = self.events.len() - MAX_ACTIVITY_ENTRIES;
            self.events = self.events.split_off(skip);
        }
        self.scroll = 0;
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    pub fn scroll_down(&mut self) {
        let len = match self.mode {
            ActivityMode::Semantic => self.events.len(),
            ActivityMode::GitAudit => self.git_entries.len(),
        };
        if self.scroll + 1 < len {
            self.scroll += 1;
        }
    }

    pub fn title(&self) -> &'static str {
        match self.mode {
            ActivityMode::Semantic => "Activity",
            ActivityMode::GitAudit => "Git Audit",
        }
    }

    pub fn render(&self, f: &mut Frame<'_>, area: ratatui::layout::Rect, focused: bool) {
        let border_style = if focused {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };
        let visible_height = area.height.saturating_sub(2) as usize;

        let lines: Vec<Line> = match self.mode {
            ActivityMode::Semantic => self
                .events
                .iter()
                .skip(self.scroll)
                .take(visible_height)
                .map(render_summary_event)
                .collect(),
            ActivityMode::GitAudit => self
                .git_entries
                .iter()
                .skip(self.scroll)
                .take(visible_height)
                .map(render_log_entry)
                .collect(),
        };

        let block = Block::default()
            .title(self.title())
            .borders(Borders::ALL)
            .border_style(border_style);
        f.render_widget(Paragraph::new(lines).block(block), area);
    }
}

fn render_summary_event(event: &SummaryEvent) -> Line<'static> {
    let time = format_time(&event.timestamp);
    let actor = event.agent_name.as_deref().unwrap_or_else(|| {
        event
            .actor
            .strip_prefix("agent:")
            .or_else(|| event.actor.strip_prefix("user"))
            .unwrap_or(&event.actor)
    });
    let actor_color = if event.actor.contains("agent") {
        Color::Magenta
    } else {
        Color::White
    };
    let delta = if event.lines_added > 0 || event.lines_removed > 0 {
        format!(" +{}/-{}", event.lines_added, event.lines_removed)
    } else {
        String::new()
    };
    Line::from(vec![
        Span::styled(time, Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(actor.to_string(), Style::default().fg(actor_color)),
        Span::raw(" "),
        Span::styled(event.path.clone(), Style::default().fg(Color::Cyan)),
        Span::raw(delta),
        Span::raw("  "),
        Span::raw(event.summary.clone()),
    ])
}

fn render_log_entry(entry: &LogEntry) -> Line<'static> {
    let time = entry.timestamp.format("%H:%M:%S").to_string();
    let actor_color = if entry.actor.is_agent() {
        Color::Magenta
    } else {
        Color::White
    };
    Line::from(vec![
        Span::styled(time, Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(entry.actor.to_string(), Style::default().fg(actor_color)),
        Span::raw(" "),
        Span::raw(entry.summary.clone()),
    ])
}

fn format_time(ts: &str) -> String {
    if let Ok(dt) = DateTime::parse_from_rfc3339(ts) {
        return dt.format("%H:%M:%S").to_string();
    }
    ts.split('T')
        .nth(1)
        .and_then(|t| t.split('Z').next())
        .unwrap_or(ts)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Action, Actor, CommitId};
    use chrono::Utc;

    fn sample_event(i: usize) -> SummaryEvent {
        SummaryEvent {
            timestamp: Utc::now().to_rfc3339(),
            session_id: None,
            agent_name: Some("claude".into()),
            actor: "agent:claude".into(),
            action: "modify".into(),
            change_kind: "modify".into(),
            path: format!("file{i}.md"),
            doc_type: "scratch".into(),
            summary: format!("change {i}"),
            source: "mcp_write".into(),
            detected_by: "mcp".into(),
            lines_added: 1,
            lines_removed: 0,
        }
    }

    #[test]
    fn activity_caps_entries() {
        let mut activity = ActivityState::new(vec![], vec![]);
        for i in 0..250 {
            activity.push_event(sample_event(i));
        }
        assert!(activity.events.len() <= MAX_ACTIVITY_ENTRIES);
    }

    #[test]
    fn activity_toggles_mode() {
        let mut activity = ActivityState::new(vec![], vec![]);
        assert_eq!(activity.mode, ActivityMode::Semantic);
        activity.toggle_mode();
        assert_eq!(activity.mode, ActivityMode::GitAudit);
        assert_eq!(activity.title(), "Git Audit");
    }

    #[test]
    fn git_push_caps_entries() {
        let mut activity = ActivityState::new(vec![], vec![]);
        for i in 0..250 {
            activity.push_git(LogEntry {
                commit_id: CommitId(format!("{i:040x}")),
                timestamp: Utc::now(),
                action: Action::Modify,
                actor: Actor::User,
                agent_name: None,
                files: vec![],
                summary: format!("commit {i}"),
            });
        }
        assert!(activity.git_entries.len() <= MAX_ACTIVITY_ENTRIES);
    }
}
