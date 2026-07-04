use crate::manifest::{DocumentEntry, Manifest};
use crate::types::DocType;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};

/// Flat list row: either a group header or a document entry.
#[derive(Debug, Clone, PartialEq)]
enum TreeRow {
    Header {
        doc_type: DocType,
        count: usize,
        expanded: bool,
    },
    Document(DocumentEntry),
}

/// Grouped document tree panel.
pub struct TreeState {
    rows: Vec<TreeRow>,
    list_state: ListState,
    collapsed: std::collections::HashSet<DocType>,
    selected_doc: Option<DocumentEntry>,
}

impl TreeState {
    pub fn new(manifest: &Manifest) -> Self {
        let mut state = Self {
            rows: Vec::new(),
            list_state: ListState::default(),
            collapsed: std::collections::HashSet::new(),
            selected_doc: None,
        };
        state.rebuild(manifest);
        state
    }

    pub fn update(&mut self, manifest: &Manifest) {
        let selected_path = self.selected_doc.as_ref().map(|d| d.path.clone());
        self.rebuild(manifest);
        if let Some(path) = selected_path {
            if let Some(idx) = self
                .rows
                .iter()
                .position(|r| matches!(r, TreeRow::Document(d) if d.path == path))
            {
                self.list_state.select(Some(idx));
            }
        }
    }

    pub fn selected_document(&self) -> Option<&DocumentEntry> {
        match self.list_state.selected().and_then(|i| self.rows.get(i)) {
            Some(TreeRow::Document(doc)) => Some(doc),
            _ => self.selected_doc.as_ref(),
        }
    }

    pub fn scroll_up(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) => i.saturating_sub(1),
            None => 0,
        };
        self.select_index(i);
    }

    pub fn scroll_down(&mut self) {
        let len = self.rows.len();
        let i = match self.list_state.selected() {
            Some(i) if i + 1 < len => i + 1,
            Some(i) => i,
            None => 0,
        };
        self.select_index(i);
    }

    pub fn is_header_selected(&self) -> bool {
        matches!(
            self.list_state.selected().and_then(|i| self.rows.get(i)),
            Some(TreeRow::Header { .. })
        )
    }

    pub fn toggle_group_at_selection(&mut self, manifest: &Manifest) {
        if let Some(idx) = self.list_state.selected() {
            if let Some(TreeRow::Header { doc_type, .. }) = self.rows.get(idx) {
                let dt = doc_type.clone();
                if self.collapsed.contains(&dt) {
                    self.collapsed.remove(&dt);
                } else {
                    self.collapsed.insert(dt);
                }
                self.rebuild(manifest);
                self.list_state
                    .select(Some(idx.min(self.rows.len().saturating_sub(1))));
            }
        }
    }

    fn select_index(&mut self, i: usize) {
        self.list_state.select(Some(i));
        if let Some(TreeRow::Document(doc)) = self.rows.get(i) {
            self.selected_doc = Some(doc.clone());
        }
    }

    fn rebuild(&mut self, manifest: &Manifest) {
        self.rows.clear();
        for doc_type in [
            DocType::Plan,
            DocType::Context,
            DocType::Log,
            DocType::Reference,
            DocType::Scratch,
        ] {
            let docs: Vec<_> = manifest
                .documents()
                .iter()
                .filter(|d| d.doc_type == doc_type)
                .cloned()
                .collect();
            if docs.is_empty() {
                continue;
            }
            let expanded = !self.collapsed.contains(&doc_type);
            self.rows.push(TreeRow::Header {
                doc_type: doc_type.clone(),
                count: docs.len(),
                expanded,
            });
            if expanded {
                for doc in docs {
                    self.rows.push(TreeRow::Document(doc));
                }
            }
        }
        if self.list_state.selected().is_none() && !self.rows.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    pub fn render_widget(&mut self, compact: bool, focused: bool) -> (List<'_>, &mut ListState) {
        let items: Vec<ListItem> = self
            .rows
            .iter()
            .map(|row| match row {
                TreeRow::Header {
                    doc_type,
                    count,
                    expanded,
                } => {
                    let arrow = if *expanded { "▼" } else { "▶" };
                    let label = if compact {
                        format!("{arrow} {} ({count})", doc_type.indicator())
                    } else {
                        format!("{arrow} {doc_type} ({count})")
                    };
                    ListItem::new(Line::from(Span::styled(
                        label,
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::BOLD),
                    )))
                }
                TreeRow::Document(doc) => {
                    let indicator = doc.doc_type.indicator();
                    let mut spans = vec![
                        Span::raw("  "),
                        Span::styled(format!("[{indicator}] "), Style::default().fg(Color::Cyan)),
                        Span::raw(doc.path.display().to_string()),
                    ];
                    if !compact && !doc.tags.is_empty() {
                        spans.push(Span::styled(
                            format!("  [{}]", doc.tags.join(",")),
                            Style::default().fg(Color::DarkGray),
                        ));
                    }
                    ListItem::new(Line::from(spans))
                }
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .title("Documents")
                    .borders(Borders::ALL)
                    .border_style(if focused {
                        Style::default().fg(Color::Yellow)
                    } else {
                        Style::default()
                    }),
            )
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        (list, &mut self.list_state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StoreInfo;
    use crate::manifest::Manifest;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn grouped_tree_lists_documents() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        let info = StoreInfo::new("test".into());
        let mut manifest = Manifest::create_empty(info, root).unwrap();
        manifest
            .register(&PathBuf::from("plan.md"), DocType::Plan, "")
            .unwrap();
        manifest
            .register(&PathBuf::from("notes.md"), DocType::Scratch, "")
            .unwrap();

        let tree = TreeState::new(&manifest);
        assert!(tree.rows.len() >= 4); // 2 headers + 2 docs
    }

    #[test]
    fn grouped_tree_handles_many_documents() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        let info = StoreInfo::new("test".into());
        let mut manifest = Manifest::create_empty(info, root).unwrap();
        for i in 0..500 {
            manifest
                .register(
                    &PathBuf::from(format!("scratch/file{i}.md")),
                    DocType::Scratch,
                    "",
                )
                .unwrap();
        }
        let tree = TreeState::new(&manifest);
        let doc_rows = tree
            .rows
            .iter()
            .filter(|r| matches!(r, TreeRow::Document(_)))
            .count();
        assert_eq!(doc_rows, 500);
    }
}
