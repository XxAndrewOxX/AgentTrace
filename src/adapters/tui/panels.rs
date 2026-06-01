use crate::manifest::{DocumentEntry, Manifest};
use crate::types::LogEntry;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};

// ── Tree Panel State ──────────────────────────────────────────────────────────

pub struct TreeState {
    pub documents: Vec<DocumentEntry>,
    pub list_state: ListState,
}

impl TreeState {
    pub fn new(manifest: &Manifest) -> Self {
        Self {
            documents: manifest.documents().to_vec(),
            list_state: ListState::default(),
        }
    }

    pub fn update(&mut self, manifest: &Manifest) {
        self.documents = manifest.documents().to_vec();
    }

    pub fn scroll_up(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) => {
                if i == 0 {
                    0
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    pub fn scroll_down(&mut self) {
        let len = self.documents.len();
        let i = match self.list_state.selected() {
            Some(i) => {
                if i >= len.saturating_sub(1) {
                    i
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    pub fn render_widget(&mut self) -> (List<'_>, &mut ListState) {
        let items: Vec<ListItem> = self
            .documents
            .iter()
            .map(|doc| {
                let indicator = doc.doc_type.indicator();
                let line = Line::from(vec![
                    Span::styled(format!("[{indicator}] "), Style::default().fg(Color::Cyan)),
                    Span::raw(doc.path.display().to_string()),
                ]);
                ListItem::new(line)
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().title("Documents").borders(Borders::ALL))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        (list, &mut self.list_state)
    }
}

// ── Changelog Panel State ─────────────────────────────────────────────────────

const MAX_CHANGELOG_ENTRIES: usize = 200;

pub struct ChangelogState {
    pub entries: Vec<LogEntry>,
    pub scroll: usize,
}

impl ChangelogState {
    pub fn new(initial: Vec<LogEntry>) -> Self {
        Self {
            entries: initial,
            scroll: 0,
        }
    }

    pub fn push(&mut self, entry: LogEntry) {
        self.entries.insert(0, entry);
        if self.entries.len() > MAX_CHANGELOG_ENTRIES {
            self.entries.truncate(MAX_CHANGELOG_ENTRIES);
            self.scroll = self.scroll.min(MAX_CHANGELOG_ENTRIES.saturating_sub(1));
        }
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    pub fn scroll_down(&mut self) {
        if self.scroll + 1 < self.entries.len() {
            self.scroll += 1;
        }
    }
}

// ── Chat Input State ──────────────────────────────────────────────────────────

pub struct ChatState {
    pub input: String,
    pub cursor: usize,
    pub history: Vec<String>,
    pub history_idx: Option<usize>,
    pub output: Option<String>,
}

impl ChatState {
    pub fn new(history: Vec<String>) -> Self {
        Self {
            input: String::new(),
            cursor: 0,
            history,
            history_idx: None,
            output: None,
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

// ── Panel Focus ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Focus {
    Tree,
    Changelog,
    Chat,
}

impl Focus {
    pub fn next(&self) -> Self {
        match self {
            Focus::Tree => Focus::Changelog,
            Focus::Changelog => Focus::Chat,
            Focus::Chat => Focus::Tree,
        }
    }
}

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
        let f = Focus::Tree;
        assert_eq!(f.next(), Focus::Changelog);
        assert_eq!(f.next().next(), Focus::Chat);
        assert_eq!(f.next().next().next(), Focus::Tree);
    }

    #[test]
    fn test_changelog_state_push() {
        let mut log = ChangelogState::new(vec![]);
        use chrono::Utc;
        let entry = LogEntry {
            commit_id: "abc".into(),
            timestamp: Utc::now(),
            action: crate::types::Action::Create,
            actor: crate::types::Actor::User,
            agent_name: None,
            files: vec![],
            summary: "test".into(),
        };
        log.push(entry);
        assert_eq!(log.entries.len(), 1);
    }
}
