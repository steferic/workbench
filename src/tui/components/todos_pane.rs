use crate::app::{AppState, FocusPanel, TodoPaneMode, TodosTab};
use crate::models::{Difficulty, Importance, TodoStatus};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = crate::theme::current();
    let is_focused = state.ui.focus == FocusPanel::TodosPane;
    let border_style = if is_focused {
        Style::default().fg(t.border_focused)
    } else {
        Style::default().fg(t.border)
    };

    // Count todos for title
    let active_count = state
        .selected_workspace()
        .map(|ws| {
            ws.todos
                .iter()
                .filter(|t| !t.is_done() && !t.is_archived())
                .count()
        })
        .unwrap_or(0);
    let review_count = state
        .selected_workspace()
        .map(|ws| ws.todos.iter().filter(|t| t.is_ready_for_review()).count())
        .unwrap_or(0);
    let archived_count = state
        .selected_workspace()
        .map(|ws| ws.todos.iter().filter(|t| t.is_archived()).count())
        .unwrap_or(0);

    // Build title with mode indicator
    let mode_indicator = match state.ui.todo_pane_mode {
        TodoPaneMode::Write => "[W]",
        TodoPaneMode::Autorun => "[A]",
    };
    let mode_color = match state.ui.todo_pane_mode {
        TodoPaneMode::Write => t.info,
        TodoPaneMode::Autorun => t.success,
    };

    let count_str = if review_count > 0 {
        format!("({})", review_count)
    } else if active_count > 0 {
        format!("({})", active_count)
    } else {
        String::new()
    };

    let title_style = if review_count > 0 {
        Style::default()
            .fg(t.active)
            .add_modifier(Modifier::BOLD)
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
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner_area);

    let tab_area = chunks[0];
    let list_area = chunks[1];
    let action_area = chunks[2];

    // Render tab bar
    let tab_style = if is_focused {
        Style::default().fg(t.fg_faint)
    } else {
        Style::default().fg(t.inactive)
    };
    let active_tab_style = if is_focused {
        Style::default()
            .fg(t.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.fg_dim)
    };
    let inactive_tab_style = tab_style;

    // Get reports count from active parallel task
    let reports_count = state
        .selected_workspace()
        .and_then(|ws| ws.active_parallel_task())
        .map(|t| t.attempts.len())
        .unwrap_or(0);

    let (active_style, archived_style, reports_style) = match state.ui.selected_todos_tab {
        TodosTab::Active => (active_tab_style, inactive_tab_style, inactive_tab_style),
        TodosTab::Archived => (inactive_tab_style, active_tab_style, inactive_tab_style),
        TodosTab::Reports => (inactive_tab_style, inactive_tab_style, active_tab_style),
    };

    let tab_bar = Paragraph::new(Line::from(vec![
        Span::styled(" Active", active_style),
        Span::styled(format!("({}) ", active_count), active_style),
        Span::styled("│", tab_style),
        Span::styled(" Archived", archived_style),
        Span::styled(format!("({}) ", archived_count), archived_style),
        Span::styled("│", tab_style),
        Span::styled(" Reports", reports_style),
        Span::styled(format!("({}) ", reports_count), reports_style),
    ]));
    frame.render_widget(tab_bar, tab_area);

    // Filter todos based on selected tab (Reports tab handled separately)
    let todos: Vec<_> = if state.ui.selected_todos_tab == TodosTab::Reports {
        Vec::new() // Reports tab shows parallel task attempts, not todos
    } else {
        state
            .selected_workspace()
            .map(|ws| {
                ws.todos
                    .iter()
                    .filter(|t| {
                        match state.ui.selected_todos_tab {
                            TodosTab::Active => !t.is_archived(),
                            TodosTab::Archived => t.is_archived(),
                            TodosTab::Reports => false, // Handled separately
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    // Render action bar (1 row, compact) - different for each tab
    let action_style = if is_focused {
        Style::default().fg(t.fg_faint)
    } else {
        Style::default().fg(t.inactive)
    };
    let key_style = if is_focused {
        Style::default().fg(t.accent)
    } else {
        Style::default().fg(t.fg_faint)
    };

    let action_bar = Paragraph::new(Line::from(vec![
        Span::styled("h", key_style),
        Span::styled(":help", action_style),
    ]));
    frame.render_widget(action_bar, action_area);

    // Handle Reports tab separately
    if state.ui.selected_todos_tab == TodosTab::Reports {
        render_reports_tab(frame, list_area, state, is_focused);
        return;
    }

    if todos.is_empty() {
        let msg = match state.ui.selected_todos_tab {
            TodosTab::Active => Paragraph::new(Line::from(vec![
                Span::styled("  No todos. Press ", Style::default().fg(t.fg_faint)),
                Span::styled("[n]", Style::default().fg(t.accent)),
                Span::styled(" to add.", Style::default().fg(t.fg_faint)),
            ])),
            TodosTab::Archived => Paragraph::new(Line::from(vec![Span::styled(
                "  No archived todos.",
                Style::default().fg(t.fg_faint),
            )])),
            TodosTab::Reports => Paragraph::new(Line::from(vec![
                Span::styled(
                    "  No reports yet. Press ",
                    Style::default().fg(t.fg_faint),
                ),
                Span::styled("[P]", Style::default().fg(t.accent)),
                Span::styled(
                    " in Sessions to start a parallel task.",
                    Style::default().fg(t.fg_faint),
                ),
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
                TodoStatus::Suggested => ("?", t.accent),
                TodoStatus::Pending => ("○", t.fg_dim),
                TodoStatus::Queued => ("◎", t.special),
                TodoStatus::InProgress { .. } => ("◐", t.active),
                TodoStatus::ReadyForReview { .. } => ("◉", t.success),
                TodoStatus::Done => ("✓", t.fg_faint),
                TodoStatus::Archived => ("📦", t.fg_faint),
            };

            let name_style = if is_selected {
                Style::default()
                    .fg(t.accent)
                    .add_modifier(Modifier::BOLD)
            } else if matches!(todo.status, TodoStatus::Done) {
                Style::default().fg(t.fg_faint)
            } else {
                Style::default().fg(t.fg)
            };

            let prefix = if is_selected { "> " } else { "  " };

            // Build difficulty badge
            let diff_spans: Vec<Span> = if let Some(diff) = &todo.difficulty {
                let (label, bg_color) = match diff {
                    Difficulty::Easy => ("E", t.success),
                    Difficulty::Med => ("M", t.active),
                    Difficulty::Hard => ("H", t.error),
                };
                vec![Span::styled(
                    label,
                    Style::default().fg(t.on_accent).bg(bg_color),
                )]
            } else {
                vec![]
            };

            // Build importance badge
            let imp_spans: Vec<Span> = if let Some(imp) = &todo.importance {
                let (label, bg_color) = match imp {
                    Importance::Low => ("L", t.fg_faint),
                    Importance::Med => ("M", t.info),
                    Importance::High => ("H", t.special),
                    Importance::Critical => ("!", t.error),
                };
                vec![Span::styled(
                    label,
                    Style::default().fg(t.fg).bg(bg_color),
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
                TodoStatus::Suggested => Style::default().fg(t.accent),
                TodoStatus::Queued => Style::default().fg(t.special),
                TodoStatus::InProgress { .. } => Style::default().fg(t.active),
                TodoStatus::ReadyForReview { .. } => Style::default()
                    .fg(t.success)
                    .add_modifier(Modifier::BOLD),
                _ => Style::default(),
            };

            // Calculate badge width for available space
            let badge_width = diff_spans.len()
                + imp_spans.len()
                + if !diff_spans.is_empty() && !imp_spans.is_empty() {
                    1
                } else {
                    0
                };
            let desc = &todo.description;
            let first_line_width =
                available_width.saturating_sub(status_label.len() + badge_width + 1);
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
                    let max_width = if is_first {
                        first_line_width
                    } else {
                        continuation_width
                    };

                    // Find wrap point (prefer word boundary)
                    let wrap_at = if remaining.len() <= max_width {
                        remaining.len()
                    } else {
                        let search_range = &remaining[..max_width.min(remaining.len())];
                        search_range
                            .rfind(' ')
                            .unwrap_or(max_width.min(remaining.len()))
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
                        let indent = "     ".to_string()
                            + &" ".repeat(badge_width + if badge_width > 0 { 1 } else { 0 });
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
            .bg(t.selection_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    let list = List::new(items).highlight_style(highlight_style);

    let mut list_state = ListState::default();
    if !todos.is_empty() {
        list_state.select(Some(state.ui.selected_todo_idx));
    }

    frame.render_stateful_widget(list, list_area, &mut list_state);
}

/// Render the Reports tab content showing parallel task attempts
fn render_reports_tab(frame: &mut Frame, area: Rect, state: &AppState, is_focused: bool) {
    use crate::models::AttemptStatus;
    use ratatui::layout::{Constraint, Direction, Layout};

    let t = crate::theme::current();

    let parallel_task = state
        .selected_workspace()
        .and_then(|ws| ws.active_parallel_task());

    let Some(task) = parallel_task else {
        let msg = Paragraph::new(Line::from(vec![
            Span::styled(
                "  No active parallel task. Press ",
                Style::default().fg(t.fg_faint),
            ),
            Span::styled("[P]", Style::default().fg(t.accent)),
            Span::styled(
                " in Sessions pane to start one.",
                Style::default().fg(t.fg_faint),
            ),
        ]));
        frame.render_widget(msg, area);
        return;
    };

    // Split area for header and list
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);

    let header_area = chunks[0];
    let list_area = chunks[1];

    // Render task header with prompt preview
    let prompt_preview: String = task.prompt.chars().take(50).collect();
    let prompt_display = if task.prompt.len() > 50 {
        format!("{}...", prompt_preview)
    } else {
        prompt_preview
    };

    let running = task
        .attempts
        .iter()
        .filter(|a| a.status == AttemptStatus::Running)
        .count();
    let completed = task
        .attempts
        .iter()
        .filter(|a| a.status == AttemptStatus::Completed)
        .count();
    let total = task.attempts.len();

    let status_text = if running > 0 {
        format!("{} working, {}/{} done", running, completed, total)
    } else if total > 0 {
        format!("{}/{} completed - select winner to merge", completed, total)
    } else {
        "Starting...".to_string()
    };

    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("  Task: ", Style::default().fg(t.fg_faint)),
            Span::styled(prompt_display, Style::default().fg(t.fg)),
        ]),
        Line::from(vec![
            Span::styled("  Status: ", Style::default().fg(t.fg_faint)),
            Span::styled(
                status_text,
                Style::default().fg(if running > 0 {
                    t.active
                } else {
                    t.success
                }),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Source: ", Style::default().fg(t.fg_faint)),
            Span::styled(task.source_branch.clone(), Style::default().fg(t.accent)),
        ]),
    ]);
    frame.render_widget(header, header_area);

    if task.attempts.is_empty() {
        let msg = Paragraph::new("  No attempts yet - agents spawning...")
            .style(Style::default().fg(t.active));
        frame.render_widget(msg, list_area);
        return;
    }

    let items: Vec<ListItem> = task
        .attempts
        .iter()
        .enumerate()
        .map(|(i, attempt)| {
            let is_selected = i == state.ui.parallel_task.selected_report_idx && is_focused;

            let style = if is_selected {
                Style::default()
                    .fg(t.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.fg_dim)
            };

            let prefix = if is_selected { "> " } else { "  " };

            let (status_icon, status_color) = match attempt.status {
                AttemptStatus::Running => (state.spinner_char(), t.active),
                AttemptStatus::Completed => ("◆", t.success),
                AttemptStatus::Failed => ("✗", t.error),
            };

            let agent_badge = attempt.agent_type.badge();
            let agent_name = attempt.agent_type.display_name();

            // First line: agent info and status
            let line1 = Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(
                    format!("[{}] ", agent_badge),
                    Style::default().fg(t.special),
                ),
                Span::styled(format!("{} ", agent_name), style),
                Span::styled(status_icon, Style::default().fg(status_color)),
                Span::styled(
                    format!(" {}", attempt.status.display()),
                    Style::default().fg(status_color),
                ),
            ]);

            // Second line: branch name
            let line2 = Line::from(vec![
                Span::raw("      "),
                Span::styled("branch: ", Style::default().fg(t.fg_faint)),
                Span::styled(
                    attempt.branch_name.clone(),
                    Style::default().fg(t.accent),
                ),
            ]);

            // Third line: report preview (if available)
            let mut lines = vec![line1, line2];
            if let Some(preview) = attempt.report_preview() {
                // Truncate preview to fit in available width
                let max_chars = 60;
                let truncated: String = preview.chars().take(max_chars).collect();
                let display_preview = if preview.len() > max_chars {
                    format!("{}...", truncated.trim())
                } else {
                    truncated.trim().to_string()
                };
                lines.push(Line::from(vec![
                    Span::raw("      "),
                    Span::styled("report: ", Style::default().fg(t.fg_faint)),
                    Span::styled(display_preview, Style::default().fg(t.success)),
                ]));
            }

            ListItem::new(lines)
        })
        .collect();

    // Highlight style with full row background when focused
    let highlight_style = if is_focused {
        Style::default()
            .bg(t.selection_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    let list = List::new(items).highlight_style(highlight_style);

    let mut list_state = ListState::default();
    if !task.attempts.is_empty() {
        list_state.select(Some(state.ui.parallel_task.selected_report_idx));
    }

    frame.render_stateful_widget(list, list_area, &mut list_state);
}
