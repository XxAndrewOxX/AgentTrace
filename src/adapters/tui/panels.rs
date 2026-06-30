use ratatui::widgets::{Block, Borders, Paragraph};

// ── Chat Input State ──────────────────────────────────────────────────────────

pub struct ChatState {
    pub input: String,
    pub cursor: usize,
    pub history: Vec<String>,
    pub history_idx: Option<usize>,
}

impl ChatState {
    pub fn new(history: Vec<String>) -> Self {
        Self {
            input: String::new(),
            cursor: 0,
            history,
            history_idx: None,
        }
    }

    pub fn push_char(&mut self, c: char) {
        self.input.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let prev = self.input[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.input.remove(prev);
            self.cursor = prev;
        }
    }

    pub fn take_input(&mut self) -> String {
        let cmd = self.input.clone();
        if !cmd.trim().is_empty() {
            self.history.push(cmd.clone());
        }
        self.input.clear();
        self.cursor = 0;
        self.history_idx = None;
        cmd
    }

    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.history_idx {
            None => self.history.len() - 1,
            Some(i) => i.saturating_sub(1),
        };
        self.history_idx = Some(idx);
        self.input = self.history[idx].clone();
        self.cursor = self.input.len();
    }

    pub fn history_down(&mut self) {
        match self.history_idx {
            None => {}
            Some(i) => {
                if i + 1 < self.history.len() {
                    self.history_idx = Some(i + 1);
                    self.input = self.history[i + 1].clone();
                    self.cursor = self.input.len();
                } else {
                    self.history_idx = None;
                    self.input.clear();
                    self.cursor = 0;
                }
            }
        }
    }
}

// ── Transient overlay (command output / doc preview) ──────────────────────────

pub struct OverlayState {
    pub title: String,
    pub body: String,
}

impl OverlayState {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
        }
    }

    pub fn render(&self, f: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect) {
        let para = Paragraph::new(self.body.clone())
            .block(
                Block::default()
                    .title(self.title.clone())
                    .borders(Borders::ALL),
            )
            .wrap(ratatui::widgets::Wrap { trim: false });
        f.render_widget(para, area);
    }
}

// ── Panel Focus ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Context,
    Tree,
    Activity,
    Alerts,
    Command,
}

impl Focus {
    pub fn next(self) -> Self {
        match self {
            Focus::Context => Focus::Tree,
            Focus::Tree => Focus::Activity,
            Focus::Activity => Focus::Alerts,
            Focus::Alerts => Focus::Command,
            Focus::Command => Focus::Context,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Focus::Context => Focus::Command,
            Focus::Tree => Focus::Context,
            Focus::Activity => Focus::Tree,
            Focus::Alerts => Focus::Activity,
            Focus::Command => Focus::Alerts,
        }
    }

    pub fn from_index(n: u8) -> Option<Self> {
        match n {
            1 => Some(Focus::Context),
            2 => Some(Focus::Tree),
            3 => Some(Focus::Activity),
            4 => Some(Focus::Alerts),
            5 => Some(Focus::Command),
            _ => None,
        }
    }
}

// Backward-compatible alias for tests that referenced the old changelog focus.
pub type ChangelogFocus = Focus;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_push_and_take() {
        let mut chat = ChatState::new(vec![]);
        chat.push_char('l');
        chat.push_char('s');
        assert_eq!(chat.input, "ls");
        let cmd = chat.take_input();
        assert_eq!(cmd, "ls");
        assert!(chat.input.is_empty());
    }

    #[test]
    fn test_chat_backspace() {
        let mut chat = ChatState::new(vec![]);
        chat.push_char('a');
        chat.push_char('b');
        chat.backspace();
        assert_eq!(chat.input, "a");
    }

    #[test]
    fn test_chat_history() {
        let mut chat = ChatState::new(vec!["ls".into(), "info prd.md".into()]);
        chat.history_up();
        assert_eq!(chat.input, "info prd.md");
        chat.history_up();
        assert_eq!(chat.input, "ls");
        chat.history_down();
        assert_eq!(chat.input, "info prd.md");
    }

    #[test]
    fn test_focus_cycles() {
        let f = Focus::Context;
        assert_eq!(f.next(), Focus::Tree);
        assert_eq!(f.next().next(), Focus::Activity);
        assert_eq!(f.next().next().next(), Focus::Alerts);
        assert_eq!(f.next().next().next().next(), Focus::Command);
        assert_eq!(f.next().next().next().next().next(), Focus::Context);
    }
}
