use ratatui::layout::{Constraint, Direction, Layout, Rect};

pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 24;
pub const COMMAND_ROWS: u16 = 3;
pub const STATUS_ROWS: u16 = 1;

/// Responsive terminal zones for the birds-eye dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DashboardLayout {
    pub status: Rect,
    pub context: Rect,
    pub documents: Rect,
    pub activity: Rect,
    pub alerts: Rect,
    pub command: Rect,
    pub compact: bool,
    pub show_alerts_column: bool,
}

impl DashboardLayout {
    pub fn compute(area: Rect, context_expanded: bool) -> Self {
        let compact = is_compact(area);
        let show_alerts_column = area.width >= 100;

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(STATUS_ROWS),
                Constraint::Length(context_rows(area, context_expanded, compact)),
                Constraint::Min(5),
                Constraint::Length(COMMAND_ROWS),
            ])
            .split(area);

        let body = if show_alerts_column {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(22),
                    Constraint::Percentage(58),
                    Constraint::Percentage(20),
                ])
                .split(rows[2]);
            (cols[0], cols[1], cols[2])
        } else {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
                .split(rows[2]);
            (cols[0], cols[1], Rect::default())
        };

        Self {
            status: rows[0],
            context: rows[1],
            documents: body.0,
            activity: body.1,
            alerts: body.2,
            command: rows[3],
            compact,
            show_alerts_column,
        }
    }
}

fn context_rows(area: Rect, expanded: bool, compact: bool) -> u16 {
    if compact {
        return if expanded { 3 } else { 1 };
    }
    if area.height >= 40 {
        return if expanded { 5 } else { 2 };
    }
    if expanded {
        4
    } else {
        2
    }
}

pub fn is_compact(area: Rect) -> bool {
    area.width <= MIN_WIDTH && area.height <= MIN_HEIGHT + 4
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    #[test]
    fn layout_splits_all_zones_at_minimum_size() {
        let layout = DashboardLayout::compute(Rect::new(0, 0, 80, 24), true);
        assert!(layout.status.height == 1);
        assert!(layout.context.height >= 1);
        assert!(layout.documents.height >= 1);
        assert!(layout.activity.height >= 1);
        assert!(layout.command.height == COMMAND_ROWS);
        assert!(!layout.show_alerts_column);
    }

    #[test]
    fn layout_includes_alerts_column_at_wide_width() {
        let layout = DashboardLayout::compute(Rect::new(0, 0, 120, 30), true);
        assert!(layout.show_alerts_column);
        assert!(layout.alerts.width > 0);
    }

    #[test]
    fn layout_compact_at_minimum() {
        assert!(is_compact(Rect::new(0, 0, 80, 24)));
        let collapsed = DashboardLayout::compute(Rect::new(0, 0, 80, 24), false);
        let expanded = DashboardLayout::compute(Rect::new(0, 0, 80, 24), true);
        assert!(collapsed.context.height < expanded.context.height);
    }

    #[test]
    fn layout_no_alerts_below_threshold() {
        let layout = DashboardLayout::compute(Rect::new(0, 0, 99, 30), true);
        assert!(!layout.show_alerts_column);
    }
}
