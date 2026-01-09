use crate::app::{AppState, FocusPanel, TodoPaneMode, TodosTab};
use crate::models::{Difficulty, Importance, TodoStatus};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let is_focused = state.ui.focus == FocusPanel::TodosPane;
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // Count todos for title
    let active_count = state.selected_workspace()
        .map(|ws| ws.todos.iter().filter(|t| !t.is_done() && !t.is_archived()).count())
        .unwrap_or(0);
    let review_count = state.selected_workspace()
        .map(|ws| ws.todos.iter().filter(|t| t.is_ready_for_review()).count())
        .unwrap_or(0);
    let archived_count = state.selected_workspace()
        .map(|ws| ws.todos.iter().filter(|t| t.is_archived()).count())
        .unwrap_or(0);

    // Build title with mode indicator
    let mode_indicator = match state.ui.todo_pane_mode {
        TodoPaneMode::Write => "[W]",
        TodoPaneMode::Autorun => "[A]",
    };
    let mode_color = match state.ui.todo_pane_mode {
        TodoPaneMode::Write => Color::Blue,
        TodoPaneMode::Autorun => Color::Green,
    };

    let count_str = if review_count > 0 {
        format!("({})", review_count)
    } else if active_count > 0 {
        format!("({})", active_count)
    } else {
        String::new()
    };

    let title_style = if review_count > 0 {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    // Compose title as Line with multiple spans
    let title_line = Line::from(vec![
        Span::raw(" Todos "),
        Span::styled(mode_indicator, Style::default().fg(mode_color)),
        if !count_str.is_empty() {
            Span::styled(format!(" {} ", count_str), title_style)
        } else {
            Span::raw(" ")
        },
    ]);

    let block = Block::default()
        .title(title_line)
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    // Split inner area: tab bar (1 row) + list + action bar (1 row)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)])
        .split(inner_area);

    let tab_area = chunks[0];
    let list_area = chunks[1];
    let action_area = chunks[2];

    // Render tab bar
    let tab_style = if is_focused {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::Rgb(60, 60, 60))
    };
    let active_tab_style = if is_focused {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    let inactive_tab_style = tab_style;

    let (active_style, archived_style) = match state.ui.selected_todos_tab {
        TodosTab::Active => (active_tab_style, inactive_tab_style),
        TodosTab::Archived => (inactive_tab_style, active_tab_style),
    };

    let tab_bar = Paragraph::new(Line::from(vec![
        Span::styled(" Active", active_style),
        Span::styled(format!("({}) ", active_count), active_style),
        Span::styled("│", tab_style),
        Span::styled(" Archived", archived_style),
        Span::styled(format!("({}) ", archived_count), archived_style),
    ]));
    frame.render_widget(tab_bar, tab_area);

    // Filter todos based on selected tab
    let todos: Vec<_> = state.selected_workspace()
        .map(|ws| {
            ws.todos.iter().filter(|t| {
                match state.ui.selected_todos_tab {
                    TodosTab::Active => !t.is_archived(),
                    TodosTab::Archived => t.is_archived(),
                }
            }).collect()
        })
        .unwrap_or_default();

    // Render action bar (1 row, compact) - different for each tab
    let action_style = if is_focused {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::Rgb(60, 60, 60))
    };
    let key_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let action_bar = match state.ui.selected_todos_tab {
        TodosTab::Active => Paragraph::new(Line::from(vec![
            Span::styled("Tab", key_style),
            Span::styled(":arc ", action_style),
            Span::styled("n", key_style),
            Span::styled(":new ", action_style),
            Span::styled("⏎", key_style),
            Span::styled(":run ", action_style),
            Span::styled("y/Y", key_style),
            Span::styled(":ok ", action_style),
            Span::styled("x/X", key_style),
            Span::styled(":done/arc ", action_style),
            Span::styled("d", key_style),
            Span::styled(":del", action_style),
        ])),
        TodosTab::Archived => Paragraph::new(Line::from(vec![
            Span::styled("Tab", key_style),
            Span::styled(":active ", action_style),
            Span::styled("d", key_style),
            Span::styled(":delete", action_style),
        ])),
    };
    frame.render_widget(action_bar, action_area);

    if todos.is_empty() {
        let msg = match state.ui.selected_todos_tab {
            TodosTab::Active => Paragraph::new(Line::from(vec![
                Span::styled("  No todos. Press ", Style::default().fg(Color::DarkGray)),
                Span::styled("[n]", Style::default().fg(Color::Cyan)),
                Span::styled(" to add.", Style::default().fg(Color::DarkGray)),
            ])),
            TodosTab::Archived => Paragraph::new(Line::from(vec![
                Span::styled("  No archived todos.", Style::default().fg(Color::DarkGray)),
            ])),
        };
        frame.render_widget(msg, list_area);
        return;
    }

    // Calculate available width for text (account for prefix, icon, and padding)
    let available_width = list_area.width.saturating_sub(6) as usize; // "  ? " = 4 chars + some margin

    let items: Vec<ListItem> = todos
        .iter()
        .enumerate()
        .map(|(i, todo)| {
            let is_selected = i == state.ui.selected_todo_idx && is_focused;

            let (status_icon, status_color) = match &todo.status {
                TodoStatus::Suggested => ("?", Color::Cyan),
                TodoStatus::Pending => ("○", Color::Gray),
                TodoStatus::Queued => ("◎", Color::Magenta),
                TodoStatus::InProgress { .. } => ("◐", Color::Yellow),
                TodoStatus::ReadyForReview { .. } => ("◉", Color::Green),
                TodoStatus::Done => ("✓", Color::DarkGray),
                TodoStatus::Archived => ("📦", Color::DarkGray),
            };

            let name_style = if is_selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else if matches!(todo.status, TodoStatus::Done) {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(Color::White)
            };

            let prefix = if is_selected { "> " } else { "  " };

            // Build difficulty badge
            let diff_spans: Vec<Span> = if let Some(diff) = &todo.difficulty {
                let (label, bg_color) = match diff {
                    Difficulty::Easy => ("E", Color::Green),
                    Difficulty::Med => ("M", Color::Yellow),
                    Difficulty::Hard => ("H", Color::Red),
                };
                vec![Span::styled(
                    label,
                    Style::default().fg(Color::Black).bg(bg_color),
                )]
            } else {
                vec![]
            };

            // Build importance badge
            let imp_spans: Vec<Span> = if let Some(imp) = &todo.importance {
                let (label, bg_color) = match imp {
                    Importance::Low => ("L", Color::DarkGray),
                    Importance::Med => ("M", Color::Blue),
                    Importance::High => ("H", Color::Magenta),
                    Importance::Critical => ("!", Color::Red),
                };
                vec![Span::styled(
                    label,
                    Style::default().fg(Color::White).bg(bg_color),
                )]
            } else {
                vec![]
            };

            // Status label for special states
            let status_label = match &todo.status {
                TodoStatus::Suggested => " ?",
                TodoStatus::Queued => " Q",
                TodoStatus::InProgress { .. } => " ⟳",
                TodoStatus::ReadyForReview { .. } => " ✓",
                _ => "",
            };
            let status_label_style = match &todo.status {
                TodoStatus::Suggested => Style::default().fg(Color::Cyan),
                TodoStatus::Queued => Style::default().fg(Color::Magenta),
                TodoStatus::InProgress { .. } => Style::default().fg(Color::Yellow),
                TodoStatus::ReadyForReview { .. } => Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                _ => Style::default(),
            };

            // Calculate badge width for available space
            let badge_width = diff_spans.len() + imp_spans.len() + if !diff_spans.is_empty() && !imp_spans.is_empty() { 1 } else { 0 };
            let desc = &todo.description;
            let first_line_width = available_width.saturating_sub(status_label.len() + badge_width + 1);
            let continuation_width = available_width;

            let mut lines: Vec<Line> = Vec::new();

            if desc.len() <= first_line_width {
                // Single line - fits entirely
                let mut spans = vec![
                    Span::styled(prefix, name_style),
                    Span::styled(status_icon, Style::default().fg(status_color)),
                    Span::raw(" "),
                ];
                // Add badges
                spans.extend(diff_spans.clone());
                spans.extend(imp_spans.clone());
                if badge_width > 0 {
                    spans.push(Span::raw(" "));
                }
                spans.push(Span::styled(desc.clone(), name_style));
                spans.push(Span::styled(status_label, status_label_style));
                lines.push(Line::from(spans));
            } else {
                // Multi-line wrapping needed
                let mut remaining = desc.as_str();
                let mut is_first = true;

                while !remaining.is_empty() {
                    let max_width = if is_first { first_line_width } else { continuation_width };

                    // Find wrap point (prefer word boundary)
                    let wrap_at = if remaining.len() <= max_width {
                        remaining.len()
                    } else {
                        let search_range = &remaining[..max_width.min(remaining.len())];
                        search_range.rfind(' ').unwrap_or(max_width.min(remaining.len()))
                    };

                    let (chunk, rest) = remaining.split_at(wrap_at);
                    let chunk = chunk.trim();
                    remaining = rest.trim_start();

                    if is_first {
                        let mut spans = vec![
                            Span::styled(prefix, name_style),
                            Span::styled(status_icon, Style::default().fg(status_color)),
                            Span::raw(" "),
                        ];
                        // Add badges on first line only
                        spans.extend(diff_spans.clone());
                        spans.extend(imp_spans.clone());
                        if badge_width > 0 {
                            spans.push(Span::raw(" "));
                        }
                        spans.push(Span::styled(chunk.to_string(), name_style));
                        if remaining.is_empty() {
                            spans.push(Span::styled(status_label, status_label_style));
                        }
                        lines.push(Line::from(spans));
                        is_first = false;
                    } else {
                        // Continuation lines - indent to align with text
                        let indent = "     ".to_string() + &" ".repeat(badge_width + if badge_width > 0 { 1 } else { 0 });
                        lines.push(Line::from(vec![
                            Span::raw(indent),
                            Span::styled(chunk.to_string(), name_style),
                            if remaining.is_empty() {
                                Span::styled(status_label, status_label_style)
                            } else {
                                Span::raw("")
                            },
                        ]));
                    }
                }
            }

            ListItem::new(lines)
        })
        .collect();

    // Highlight style with full row background when focused
    let highlight_style = if is_focused {
        Style::default()
            .bg(Color::Rgb(40, 50, 60))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    let list = List::new(items)
        .highlight_style(highlight_style);

    let mut list_state = ListState::default();
    if !todos.is_empty() {
        list_state.select(Some(state.ui.selected_todo_idx));
    }

    frame.render_stateful_widget(list, list_area, &mut list_state);
}
