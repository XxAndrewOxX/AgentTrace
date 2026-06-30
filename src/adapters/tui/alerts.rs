use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
    Frame,
};

const MAX_ALERTS: usize = 50;

/// Dedicated violation/alert stack — never hijacks the activity panel.
#[derive(Debug, Clone)]
pub struct AlertState {
    pub messages: Vec<String>,
    pub scroll: usize,
    pub list_state: ListState,
}

impl AlertState {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            scroll: 0,
            list_state: ListState::default(),
        }
    }

    pub fn push(&mut self, message: String) {
        self.messages.insert(0, message);
        if self.messages.len() > MAX_ALERTS {
            self.messages.truncate(MAX_ALERTS);
        }
        self.list_state.select(Some(0));
    }

    pub fn count(&self) -> usize {
        self.messages.len()
    }

    pub fn scroll_up(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) => i.saturating_sub(1),
            None => 0,
        };
        self.list_state.select(Some(i));
        self.scroll = i;
    }

    pub fn scroll_down(&mut self) {
        let len = self.messages.len();
        let i = match self.list_state.selected() {
            Some(i) if i + 1 < len => i + 1,
            Some(i) => i,
            None => 0,
        };
        self.list_state.select(Some(i));
        self.scroll = i;
    }

    pub fn render_column(
        &mut self,
        f: &mut Frame<'_>,
        area: ratatui::layout::Rect,
        focused: bool,
    ) {
        let border_style = if focused {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };
        let title = if self.messages.is_empty() {
            "Alerts".to_string()
        } else {
            format!("Alerts ({})", self.messages.len())
        };

        let items: Vec<ListItem> = self
            .messages
            .iter()
            .map(|msg| {
                ListItem::new(Line::from(Span::styled(
                    msg.clone(),
                    Style::default().fg(Color::Red),
                )))
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(border_style),
            )
            .style(Style::default());
        f.render_stateful_widget(list, area, &mut self.list_state);
    }

    pub fn render_badge_line(&self) -> Option<Line<'static>> {
        if self.messages.is_empty() {
            return None;
        }
        Some(Line::from(Span::styled(
            format!("⚠{}", self.messages.len()),
            Style::default().fg(Color::Red),
        )))
    }
}

impl Default for AlertState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alerts_push_and_cap() {
        let mut alerts = AlertState::new();
        for i in 0..60 {
            alerts.push(format!("alert {i}"));
        }
        assert!(alerts.count() <= MAX_ALERTS);
        assert_eq!(alerts.messages[0], "alert 59");
    }
}
