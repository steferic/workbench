//! Tasks pane — a live mirror of what the selected agent is doing.
//!
//! It follows the Sessions pane above: whichever agent the session cursor is
//! on is the one whose task lists appear here. Rows come from
//! `app::tasks_view` so navigation and rendering can never disagree about what
//! is on screen. Nothing here mutates a task list: the agent owns it (see
//! `handlers::tasks`).

use crate::agent_tasks::TaskState;
use crate::app::{tasks_view, AppState, FocusPanel, TaskRow, TasksTab};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = crate::theme::current();
    let is_focused = state.ui.focus == FocusPanel::TasksPane;
    let border_style = if is_focused {
        Style::default().fg(t.border_focused)
    } else {
        Style::default().fg(t.border)
    };

    let rows = tasks_view::rows(state);
    let (open, total) = task_counts(state);

    let title_line = Line::from(vec![
        Span::raw(" Tasks "),
        if total > 0 {
            Span::styled(
                format!("({open}/{total}) "),
                if open > 0 {
                    Style::default().fg(t.active)
                } else {
                    Style::default().fg(t.fg_faint)
                },
            )
        } else {
            Span::raw("")
        },
    ]);

    let block = Block::default()
        .title(title_line)
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner_area);
    let (tab_area, list_area, action_area) = (chunks[0], chunks[1], chunks[2]);

    render_tab_bar(frame, tab_area, state, is_focused);
    render_action_bar(frame, action_area, state, is_focused);

    if state.ui.selected_tasks_tab == TasksTab::Reports {
        render_reports_tab(frame, list_area, state, is_focused);
        return;
    }

    if rows.is_empty() {
        let msg = Paragraph::new(Line::from(vec![Span::styled(
            "  Select an agent in Sessions.",
            Style::default().fg(t.fg_faint),
        )]));
        frame.render_widget(msg, list_area);
        return;
    }

    let selected = state.ui.selected_task_row.min(rows.len() - 1);
    let width = list_area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| render_row(state, row, i == selected && is_focused, width))
        .collect();

    let highlight_style = if is_focused {
        Style::default().bg(t.selection_bg)
    } else {
        Style::default()
    };
    let list = List::new(items).highlight_style(highlight_style);

    let mut list_state = ListState::default();
    list_state.select(Some(selected));
    frame.render_stateful_widget(list, list_area, &mut list_state);
}

/// Open vs. total tasks in the selected agent's current list — older lists are
/// history, not work in flight.
fn task_counts(state: &AppState) -> (usize, usize) {
    let Some(batch) = tasks_view::selected_agent(state)
        .and_then(|agent| state.system.agent_tasks.get(&agent.session_id))
        .and_then(|tracker| tracker.current())
    else {
        return (0, 0);
    };
    (batch.tasks.len() - batch.completed(), batch.tasks.len())
}

fn render_tab_bar(frame: &mut Frame, area: Rect, state: &AppState, is_focused: bool) {
    let t = crate::theme::current();
    let dim = if is_focused {
        Style::default().fg(t.fg_faint)
    } else {
        Style::default().fg(t.inactive)
    };
    let active = if is_focused {
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.fg_dim)
    };

    let reports_count = state
        .selected_workspace()
        .and_then(|ws| ws.active_parallel_task())
        .map(|t| t.attempts.len())
        .unwrap_or(0);
    let (tasks_style, reports_style) = match state.ui.selected_tasks_tab {
        TasksTab::Tasks => (active, dim),
        TasksTab::Reports => (dim, active),
    };

    let tab_bar = Paragraph::new(Line::from(vec![
        Span::styled(" Tasks ", tasks_style),
        Span::styled("│", dim),
        Span::styled(" Reports", reports_style),
        Span::styled(format!("({reports_count}) "), reports_style),
    ]));
    frame.render_widget(tab_bar, area);
}

fn render_action_bar(frame: &mut Frame, area: Rect, state: &AppState, is_focused: bool) {
    let t = crate::theme::current();

    // A recent action's outcome outranks the key hints.
    if let Some(status) = state.ui.task_status() {
        let msg = Paragraph::new(Line::from(Span::styled(
            format!(" {status}"),
            Style::default().fg(t.info),
        )));
        frame.render_widget(msg, area);
        return;
    }

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

    let hints: &[(&str, &str)] = if state.ui.selected_tasks_tab == TasksTab::Reports {
        &[("v", ":view "), ("m", ":merge "), ("h", ":help")]
    } else {
        &[
            ("n", ":add "),
            ("e", ":edit "),
            ("d", ":drop "),
            ("Enter", ":open "),
            ("h", ":help"),
        ]
    };
    let spans: Vec<Span> = hints
        .iter()
        .flat_map(|(key, label)| {
            [
                Span::styled(*key, key_style),
                Span::styled(*label, action_style),
            ]
        })
        .collect();
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_row<'a>(
    state: &'a AppState,
    row: &'a TaskRow,
    is_selected: bool,
    width: usize,
) -> ListItem<'a> {
    let t = crate::theme::current();
    let prefix = if is_selected { "> " } else { "  " };

    match row {
        TaskRow::Agent { session_id } => {
            let Some(agent) = tasks_view::selected_agent(state)
                .filter(|a| a.session_id == *session_id)
            else {
                return ListItem::new(Line::from(""));
            };

            let (status_icon, status_color) = if !agent.running {
                ("○", t.fg_faint)
            } else if agent.idle {
                ("◆", t.success)
            } else {
                (state.spinner_char(), t.active)
            };

            let label = agent.alias.clone().unwrap_or_else(|| agent.name.clone());
            let mut spans = vec![
                Span::styled(prefix, Style::default().fg(t.fg)),
                Span::styled(
                    format!("[{}] ", agent.agent_type.badge()),
                    Style::default().fg(t.special),
                ),
                Span::styled(label, Style::default().fg(t.fg).add_modifier(Modifier::BOLD)),
                Span::raw(" "),
                Span::styled(status_icon, Style::default().fg(status_color)),
            ];
            if let Some(branch) = &agent.branch {
                spans.push(Span::styled(
                    format!("  {branch}"),
                    Style::default().fg(t.accent),
                ));
            }
            ListItem::new(Line::from(spans))
        }

        TaskRow::Prompt { session_id, batch } => {
            let entry = state
                .system
                .agent_tasks
                .get(session_id)
                .and_then(|tracker| tracker.batches().get(*batch));
            let prompt = entry
                .map(|b| {
                    if b.prompt.is_empty() {
                        "(no prompt captured)".to_string()
                    } else {
                        b.prompt.clone()
                    }
                })
                .unwrap_or_default();
            let age = entry.and_then(|b| b.age()).unwrap_or_default();

            // The prompt is context, not a task: two lines at most.
            let budget = width.saturating_sub(8 + age.chars().count());
            let lines = wrap(&single_line(&prompt), budget, 2);
            let last = lines.len().saturating_sub(1);
            let style = Style::default().fg(t.fg_dim).add_modifier(Modifier::ITALIC);
            ListItem::new(
                lines
                    .into_iter()
                    .enumerate()
                    .map(|(i, chunk)| {
                        let mut spans = vec![
                            Span::styled(if i == 0 { prefix } else { "  " }, style),
                            Span::raw(if i == 0 { "  ❝ " } else { "    " }),
                            Span::styled(chunk, style),
                        ];
                        if i == last && !age.is_empty() {
                            spans.push(Span::styled(
                                format!("  {age}"),
                                Style::default().fg(t.fg_faint),
                            ));
                        }
                        Line::from(spans)
                    })
                    .collect::<Vec<_>>(),
            )
        }

        TaskRow::Task {
            session_id,
            batch,
            task,
        } => {
            let Some(task) = tasks_view::task_at(state, *session_id, *batch, *task) else {
                return ListItem::new(Line::from(""));
            };
            let (icon, color) = match task.state {
                TaskState::Pending => ("○", t.fg_dim),
                TaskState::InProgress => ("◐", t.active),
                TaskState::Completed => ("✓", t.success),
            };
            let text_style = match task.state {
                TaskState::Completed => Style::default().fg(t.fg_faint),
                TaskState::InProgress => Style::default().fg(t.fg).add_modifier(Modifier::BOLD),
                TaskState::Pending => Style::default().fg(t.fg),
            };

            const INDENT: usize = 8;
            let mut lines: Vec<Line> = wrap(&single_line(&task.subject), width.saturating_sub(INDENT), 2)
                .into_iter()
                .enumerate()
                .map(|(i, chunk)| {
                    if i == 0 {
                        Line::from(vec![
                            Span::styled(prefix, text_style),
                            Span::raw("   "),
                            Span::styled(icon, Style::default().fg(color)),
                            Span::raw(" "),
                            Span::styled(chunk, text_style),
                        ])
                    } else {
                        Line::from(vec![
                            Span::raw(" ".repeat(INDENT)),
                            Span::styled(chunk, text_style),
                        ])
                    }
                })
                .collect();

            // The agent's own note about the task — only worth the rows when
            // the cursor is on it.
            if is_selected {
                if let Some(detail) = &task.detail {
                    for chunk in wrap(&single_line(detail), width.saturating_sub(INDENT + 2), 2) {
                        lines.push(Line::from(vec![
                            Span::raw(" ".repeat(INDENT + 2)),
                            Span::styled(chunk, Style::default().fg(t.fg_faint)),
                        ]));
                    }
                }
            }

            ListItem::new(lines)
        }

        TaskRow::Note { text, .. } => ListItem::new(Line::from(vec![
            Span::styled(prefix, Style::default().fg(t.fg_faint)),
            Span::raw("    "),
            Span::styled(text.clone(), Style::default().fg(t.fg_faint)),
        ])),
    }
}

/// Collapse newlines so a multi-line prompt still renders as prose.
fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Wrap on word boundaries into at most `max_lines`, ellipsizing the rest.
fn wrap(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    if width == 0 || text.is_empty() || max_lines == 0 {
        return vec![text.to_string()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut truncated = false;
    let mut words = text.split(' ').peekable();

    while let Some(word) = words.next() {
        let sep = usize::from(!current.is_empty());
        if current.chars().count() + sep + word.chars().count() > width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            if lines.len() == max_lines {
                truncated = true;
                break;
            }
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
        if words.peek().is_none() {
            lines.push(std::mem::take(&mut current));
        }
    }

    if truncated {
        if let Some(last) = lines.last_mut() {
            while last.chars().count() >= width && last.pop().is_some() {}
            last.push('…');
        }
    }
    lines
}

/// Render the Reports tab content showing parallel task attempts
fn render_reports_tab(frame: &mut Frame, area: Rect, state: &AppState, is_focused: bool) {
    use crate::models::AttemptStatus;

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
                Style::default().fg(if running > 0 { t.active } else { t.success }),
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
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
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
                Span::styled(attempt.branch_name.clone(), Style::default().fg(t.accent)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TasksTab;
    use ratatui::{backend::TestBackend, Terminal};

    /// Render the pane over a fixture workspace and return the screen as text.
    fn screen(state: &AppState, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), state))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn pane_shows_the_agent_its_prompt_and_its_tasks() {
        let (state, _session_id, _dir) = crate::app::tasks_view::tests::fixture();
        let out = screen(&state, 60, 12);

        assert!(out.contains("Tasks"), "{out}");
        // The prompt that produced the list, and the list itself.
        assert!(out.contains("make the tasks pane"), "{out}");
        assert!(out.contains("Parse agent logs"), "{out}");
        // In-progress marker, not a pending one.
        assert!(out.contains("◐"), "{out}");
        // Counts open/total for this agent's current list.
        assert!(out.contains("(1/1)"), "{out}");
    }

    #[test]
    fn pane_points_at_the_sessions_list_when_no_agent_is_selected() {
        let state = AppState::default();
        let out = screen(&state, 60, 8);
        assert!(out.contains("Select an agent in Sessions"), "{out}");
    }

    #[test]
    fn reports_tab_still_renders_from_the_same_pane() {
        let (mut state, _session_id, _dir) = crate::app::tasks_view::tests::fixture();
        state.ui.selected_tasks_tab = TasksTab::Reports;
        let out = screen(&state, 60, 8);
        assert!(out.contains("No active parallel task"), "{out}");
    }

    #[test]
    fn wrap_breaks_on_words_within_the_line_budget() {
        let lines = wrap("add a live view of every agent task list", 12, 2);
        assert!(lines.iter().all(|l| l.chars().count() <= 12), "{lines:?}");
        assert_eq!(lines[0], "add a live");
    }

    #[test]
    fn wrap_ellipsizes_what_does_not_fit() {
        let lines = wrap("one two three four five six seven eight nine", 10, 2);
        assert_eq!(lines.len(), 2);
        assert!(lines.last().unwrap().ends_with('…'), "{lines:?}");
        assert!(lines.iter().all(|l| l.chars().count() <= 10), "{lines:?}");
    }

    #[test]
    fn wrap_keeps_text_that_fits_untouched() {
        assert_eq!(wrap("short", 20, 2), vec!["short".to_string()]);
        assert_eq!(
            wrap("two lines", 8, 2),
            vec!["two".to_string(), "lines".to_string()]
        );
    }

    #[test]
    fn single_line_flattens_multiline_prompts() {
        assert_eq!(single_line("make it\n  show   tasks\n"), "make it show tasks");
    }
}
