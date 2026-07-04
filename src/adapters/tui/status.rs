use crate::config::MergedConfig;
use crate::llm::Llm;
use crate::manifest::Manifest;
use crate::running_summary::{load_summary_state, SummaryState};
use crate::session::load_session;
use chrono::{DateTime, Utc};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use std::path::Path;

/// How this TUI instance participates in store monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollRole {
    Leader,
    Observer,
    ReadOnly,
}

impl PollRole {
    pub fn from_tui_mode(readonly: bool, poll_leader: bool) -> Self {
        if readonly {
            PollRole::ReadOnly
        } else if poll_leader {
            PollRole::Leader
        } else {
            PollRole::Observer
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PollRole::Leader => "leader",
            PollRole::Observer => "observer",
            PollRole::ReadOnly => "readonly",
        }
    }

    pub fn dot_color(self) -> Color {
        match self {
            PollRole::Leader => Color::Green,
            PollRole::Observer => Color::Yellow,
            PollRole::ReadOnly => Color::Red,
        }
    }
}

/// Persistent status strip data.
#[derive(Debug, Clone)]
pub struct StatusBarState {
    pub poll_role: PollRole,
    pub agent_name: String,
    pub session_id: Option<String>,
    pub synthesis_label: String,
    pub synthesis_degraded: bool,
    pub doc_count: usize,
    pub alert_count: usize,
    pub synthesis_in_flight: bool,
    pub ops_pending: usize,
    pub last_activity: Option<DateTime<Utc>>,
}

impl StatusBarState {
    pub fn new(
        store_root: &Path,
        poll_role: PollRole,
        config: &MergedConfig,
        manifest: &Manifest,
    ) -> Self {
        let backend = Llm::backend_info_from_config(config);
        let session = load_session(store_root);
        Self {
            poll_role,
            agent_name: session
                .as_ref()
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "user".into()),
            session_id: session.as_ref().map(|s| s.session_id.clone()),
            synthesis_label: backend.label,
            synthesis_degraded: backend.degraded,
            doc_count: manifest.len(),
            alert_count: 0,
            synthesis_in_flight: false,
            ops_pending: 0,
            last_activity: None,
        }
    }

    pub fn refresh(
        &mut self,
        store_root: &Path,
        manifest: &Manifest,
        summary_state: &SummaryState,
        synthesis_in_flight: bool,
    ) {
        if let Some(session) = load_session(store_root) {
            self.agent_name = session.name;
            self.session_id = Some(session.session_id);
        } else {
            self.agent_name = "user".into();
            self.session_id = None;
        }
        self.doc_count = manifest.len();
        self.synthesis_in_flight = synthesis_in_flight;
        self.ops_pending = summary_state.ops_since_synthesis;
    }

    pub fn set_alert_count(&mut self, count: usize) {
        self.alert_count = count;
    }

    pub fn note_activity(&mut self, ts: DateTime<Utc>) {
        self.last_activity = Some(ts);
    }

    pub fn activity_age(&self) -> Option<std::time::Duration> {
        self.last_activity.map(|ts| {
            let now = Utc::now();
            (now - ts).to_std().unwrap_or(std::time::Duration::ZERO)
        })
    }

    pub fn render(&self, f: &mut Frame<'_>, area: ratatui::layout::Rect, compact: bool) {
        let dot = Span::styled("● ", Style::default().fg(self.poll_role.dot_color()));
        let agent = Span::styled(
            format!("{} ", self.agent_name),
            Style::default().add_modifier(Modifier::BOLD),
        );
        let session = self
            .session_id
            .as_deref()
            .map(|s| {
                let short = if s.len() > 14 { &s[..14] } else { s };
                format!("sess {short} │ ")
            })
            .unwrap_or_default();
        let poll = format!("poll:{} │ ", self.poll_role.label());
        let synth = if self.synthesis_degraded {
            "synth:degraded".to_string()
        } else if self.synthesis_in_flight {
            format!("synth:{} (refreshing)", self.synthesis_label)
        } else {
            format!("synth:{}", self.synthesis_label)
        };
        let docs = format!(" │ {} docs", self.doc_count);
        let delta = self
            .last_activity
            .map(|ts| {
                let mins = (Utc::now() - ts).num_minutes();
                if mins < 1 {
                    " │ Δ now".to_string()
                } else {
                    format!(" │ Δ {mins}m ago")
                }
            })
            .unwrap_or_default();
        let alerts = if self.alert_count > 0 {
            format!(" │ ⚠{}", self.alert_count)
        } else {
            String::new()
        };
        let alert_style = Style::default().fg(Color::Red);
        let pending = if self.ops_pending > 0 && !self.synthesis_in_flight {
            format!(" │ +{} ops", self.ops_pending)
        } else {
            String::new()
        };

        let line = if compact {
            Line::from(vec![
                dot,
                agent,
                Span::raw(format!("{poll}{synth}{docs}{delta}")),
                Span::styled(alerts, alert_style),
                Span::raw(pending),
            ])
        } else {
            Line::from(vec![
                dot,
                agent,
                Span::raw(session),
                Span::raw(poll),
                Span::raw(synth),
                Span::raw(docs),
                Span::raw(delta),
                Span::styled(alerts, Style::default().fg(Color::Red)),
                Span::raw(pending),
            ])
        };

        let block = Block::default().borders(Borders::ALL).title("agent-trace");
        let para = Paragraph::new(vec![line]).block(block);
        f.render_widget(para, area);
    }
}

pub fn load_summary_state_for_status(store_root: &Path) -> SummaryState {
    load_summary_state(store_root).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MergedConfig;
    use crate::manifest::Manifest;
    use crate::state::config::StoreInfo;
    use tempfile::TempDir;

    #[test]
    fn status_bar_new_has_defaults() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        let info = StoreInfo::new("test".into());
        let manifest = Manifest::create_empty(info, root).unwrap();
        let config = MergedConfig::default();
        let status = StatusBarState::new(root, PollRole::Leader, &config, &manifest);
        assert_eq!(status.poll_role, PollRole::Leader);
        assert_eq!(status.doc_count, 0);
    }

    #[test]
    fn poll_role_from_tui_mode() {
        assert_eq!(PollRole::from_tui_mode(false, true), PollRole::Leader);
        assert_eq!(PollRole::from_tui_mode(false, false), PollRole::Observer);
        assert_eq!(PollRole::from_tui_mode(true, true), PollRole::ReadOnly);
    }

    #[test]
    fn status_refresh_clears_session_when_lock_removed() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace/locks")).unwrap();
        let info = StoreInfo::new("test".into());
        let manifest = Manifest::create_empty(info, root).unwrap();
        let config = MergedConfig::default();
        let mut status = StatusBarState::new(root, PollRole::Leader, &config, &manifest);

        let session = crate::session::start_session(root, "claude", "cli").unwrap();
        status.refresh(root, &manifest, &SummaryState::default(), false);
        assert_eq!(status.agent_name, "claude");
        assert_eq!(
            status.session_id.as_deref(),
            Some(session.session_id.as_str())
        );

        crate::session::remove_session(root).unwrap();
        status.refresh(root, &manifest, &SummaryState::default(), false);
        assert_eq!(status.agent_name, "user");
        assert!(status.session_id.is_none());
    }
}
