//! Tasks pane — the work queued for the selected agent, and how it is going.
//!
//! It follows the Sessions pane above: whichever agent the session cursor is
//! on is the one whose queue appears here. Your items are the pane; under the
//! one currently running, the agent's own steps show as progress detail. Rows
//! come from `app::tasks_view` so navigation and rendering can never disagree
//! about what is on screen.

use crate::agent_tasks::TaskState;
use crate::app::{tasks_view, AppState, FocusPanel, TaskRow, TasksTab};
use crate::models::TodoState;
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
    let (left, total) = queue_counts(state);

    let mut title_spans = vec![Span::raw(" TODO ")];
    if total > 0 {
        title_spans.push(Span::styled(
            format!("({left} left) "),
            if left > 0 {
                Style::default().fg(t.active)
            } else {
                Style::default().fg(t.fg_faint)
            },
        ));
    }
    // Why nothing is moving, when nothing is moving — a queue that silently
    // sits there is indistinguishable from a broken one.
    if let Some(reason) = holding_reason(state) {
        title_spans.push(Span::styled(
            format!("{reason} "),
            Style::default().fg(t.warning),
        ));
    }
    let title_line = Line::from(title_spans);

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

    if state.ui.selected_tasks_tab == TasksTab::Objectives {
        render_objectives_tab(frame, list_area, state, is_focused);
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

/// Items still to run vs. items in the queue.
fn queue_counts(state: &AppState) -> (usize, usize) {
    let Some(session) =
        tasks_view::selected_agent(state).and_then(|agent| state.get_session(agent.session_id))
    else {
        return (0, 0);
    };
    let queue = &session.todo_queue;
    let left = queue.pending_count() + usize::from(queue.running().is_some());
    (left, queue.items.len())
}

/// The queue's state, when it is worth saying out loud.
fn holding_reason(state: &AppState) -> Option<&'static str> {
    use crate::app::todo_dispatch::Holding;
    let agent = tasks_view::selected_agent(state)?;
    match crate::app::todo_dispatch::holding(state, agent.session_id) {
        // Nothing to say: either idle with nothing queued, or about to send.
        Holding::Empty => None,
        // Visible on the item itself.
        Holding::Running => None,
        other => Some(other.label()),
    }
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
    let objectives_count = state
        .selected_workspace()
        .map(|ws| ws.objectives.len())
        .unwrap_or(0);
    let (tasks_style, objectives_style, reports_style) = match state.ui.selected_tasks_tab {
        TasksTab::Tasks => (active, dim, dim),
        TasksTab::Objectives => (dim, active, dim),
        TasksTab::Reports => (dim, dim, active),
    };

    let tab_bar = Paragraph::new(Line::from(vec![
        Span::styled(" TODO ", tasks_style),
        Span::styled("│", dim),
        Span::styled(" Objectives", objectives_style),
        Span::styled(format!("({objectives_count}) "), objectives_style),
        Span::styled("│", dim),
        Span::styled(" Reports", reports_style),
        Span::styled(format!("({reports_count}) "), reports_style),
    ]));
    frame.render_widget(tab_bar, area);
}

/// The project's standing priorities, in priority order.
///
/// Rank is position, so the top line is what matters most — which is the only
/// thing a manager will need to read off this list. State is shown as a word
/// rather than a colour alone, because "held" and "met" are decisions worth
/// spelling out.
fn render_objectives_tab(frame: &mut Frame, area: Rect, state: &AppState, is_focused: bool) {
    use crate::models::{ObjectiveState, ProposalState, Verdict};

    let t = crate::theme::current();
    let Some(workspace) = state.selected_workspace() else {
        let msg = Paragraph::new(Line::from(Span::styled(
            "  Open a project first.",
            Style::default().fg(t.fg_faint),
        )));
        frame.render_widget(msg, area);
        return;
    };

    if workspace.objectives.is_empty() {
        let msg = Paragraph::new(vec![
            Line::from(Span::styled(
                "  No objectives yet. Press n to write one.",
                Style::default().fg(t.fg_faint),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Standing priorities for this project — what",
                Style::default().fg(t.fg_faint),
            )),
            Line::from(Span::styled(
                "  work should keep moving toward.",
                Style::default().fg(t.fg_faint),
            )),
        ]);
        frame.render_widget(msg, area);
        return;
    }

    // Drawn from the same row list the keys walk, so the highlighted line and
    // the line a keypress acts on can never drift apart.
    let rows = crate::app::objectives_view::rows(state);
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .map(|(row_index, row)| {
            let selected = is_focused && row_index == state.ui.selected_objective;
            let marker = if selected { "> " } else { "  " };

            match row {
                crate::app::objectives_view::ObjectiveRow::Objective { id, index } => {
                    let Some(objective) = workspace.objectives.iter().find(|o| o.id == *id) else {
                        return Line::from("");
                    };
                    let (state_label, state_color) = match objective.state {
                        ObjectiveState::Active => ("", t.fg),
                        ObjectiveState::Held => ("held ", t.fg_faint),
                        ObjectiveState::Met => ("met ", t.success),
                    };
                    let text_style = match objective.state {
                        ObjectiveState::Active if selected => {
                            Style::default().fg(t.fg).add_modifier(Modifier::BOLD)
                        }
                        ObjectiveState::Active => Style::default().fg(t.fg),
                        _ => Style::default().fg(t.fg_faint),
                    };

                    let mut spans = vec![
                        Span::styled(marker, Style::default().fg(t.accent)),
                        Span::styled(
                            format!("{}. ", index + 1),
                            Style::default().fg(t.fg_faint),
                        ),
                    ];
                    if !state_label.is_empty() {
                        spans.push(Span::styled(state_label, Style::default().fg(state_color)));
                    }
                    spans.push(Span::styled(objective.text.clone(), text_style));
                    match objective.done_when.as_ref() {
                        Some(check) if check.proposed => spans.push(Span::styled(
                            format!("  ?{}", check.command),
                            Style::default().fg(t.warning),
                        )),
                        Some(check) => spans.push(Span::styled(
                            format!("  ✓{}", check.command),
                            Style::default().fg(t.success),
                        )),
                        None => {}
                    }
                    Line::from(spans)
                }
                crate::app::objectives_view::ObjectiveRow::Proposal { id } => {
                    let Some(proposal) = workspace.proposals.iter().find(|p| p.id == *id) else {
                        return Line::from("");
                    };
                    // Approved ones stay on the list, marked, until the queue
                    // is done with them: seeing that a suggestion became work
                    // is most of what this view is for.
                    // Once a check has spoken, its verdict is the headline:
                    // that is the fact worth reading, and the only one a
                    // manager could not have written itself.
                    let (verb, verb_color) = match (&proposal.verdict, proposal.state) {
                        (Some(Verdict::Verified), _) => ("verified ", t.success),
                        (Some(Verdict::Rejected { .. }), _) => ("rejected ", t.error),
                        (Some(Verdict::Inconclusive { .. }), _) => ("unclear ", t.warning),
                        (None, ProposalState::Approved) => ("queued ", t.info),
                        _ => ("proposes ", t.info),
                    };
                    let body_style = if selected {
                        Style::default().fg(t.fg)
                    } else {
                        Style::default().fg(t.fg_dim)
                    };
                    Line::from(vec![
                        Span::styled(format!("   {marker}"), Style::default().fg(t.accent)),
                        Span::styled(verb, Style::default().fg(verb_color)),
                        Span::styled(
                            proposal.agent.clone().unwrap_or_else(|| "nobody".into()),
                            Style::default().fg(t.accent),
                        ),
                        Span::styled(": ", Style::default().fg(t.fg_faint)),
                        Span::styled(first_line(&proposal.instruction), body_style),
                    ])
                }
            }
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), area);
}

/// One line of what is often several paragraphs of instruction.
fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() > 60 {
        format!("{}…", line.chars().take(59).collect::<String>())
    } else {
        line.to_string()
    }
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
    } else if state.ui.selected_tasks_tab == TasksTab::Objectives {
        &[
            ("n", ":add "),
            ("e", ":edit "),
            ("d", ":del "),
            ("Space", ":state "),
            ("J/K", ":rank "),
            ("a", ":approve "),
            ("x", ":no "),
            ("h", ":help"),
        ]
    } else {
        &[
            ("n", ":add "),
            ("e", ":edit "),
            ("d", ":del "),
            ("p", ":pause "),
            ("J/K", ":move "),
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
        // Your item. The state glyph carries the queue's story: waiting,
        // with the agent now, or finished.
        TaskRow::Todo { session_id, todo } => {
            let Some(item) = tasks_view::todo_at(state, *session_id, *todo) else {
                return ListItem::new(Line::from(""));
            };
            let (icon, color) = match item.state {
                TodoState::Pending => ("○", t.fg_dim),
                TodoState::Running => (state.spinner_char(), t.active),
                TodoState::Done => ("✓", t.success),
            };
            let text_style = match item.state {
                TodoState::Done => Style::default().fg(t.fg_faint),
                TodoState::Running => Style::default().fg(t.active).add_modifier(Modifier::BOLD),
                TodoState::Pending => Style::default().fg(t.fg),
            };

            const INDENT: usize = 4;
            let lines: Vec<Line> = wrap(&single_line(&item.text), width.saturating_sub(INDENT), 3)
                .into_iter()
                .enumerate()
                .map(|(i, chunk)| {
                    if i == 0 {
                        Line::from(vec![
                            Span::styled(prefix, text_style),
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
            ListItem::new(lines)
        }

        // The agent's own step, under the item it belongs to. Dimmer than
        // your items throughout: this is progress, not work you can edit.
        TaskRow::Step {
            session_id,
            batch,
            task,
        } => {
            let Some(step) = tasks_view::task_at(state, *session_id, *batch, *task) else {
                return ListItem::new(Line::from(""));
            };
            let (icon, color) = match step.state {
                TaskState::Pending => ("○", t.fg_faint),
                TaskState::InProgress => ("◐", t.active),
                TaskState::Completed => ("✓", t.fg_faint),
            };
            let style = Style::default().fg(t.fg_dim);

            const INDENT: usize = 8;
            ListItem::new(
                wrap(&single_line(&step.subject), width.saturating_sub(INDENT), 2)
                    .into_iter()
                    .enumerate()
                    .map(|(i, chunk)| {
                        if i == 0 {
                            Line::from(vec![
                                Span::styled(prefix, style),
                                Span::raw("    "),
                                Span::styled(icon, Style::default().fg(color)),
                                Span::raw(" "),
                                Span::styled(chunk, style),
                            ])
                        } else {
                            Line::from(vec![
                                Span::raw(" ".repeat(INDENT)),
                                Span::styled(chunk, style),
                            ])
                        }
                    })
                    .collect::<Vec<_>>(),
            )
        }

        TaskRow::Note { text, .. } => ListItem::new(Line::from(vec![
            Span::styled(prefix, Style::default().fg(t.fg_faint)),
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

    /// The fixture's agent has one parsed task; queue two items and run the
    /// first so the pane shows both kinds of row.
    fn state_with_queue() -> (AppState, uuid::Uuid, tempfile::TempDir) {
        let (mut state, session_id, dir) = crate::app::tasks_view::tests::fixture();
        let queue = &mut state.get_session_mut(session_id).unwrap().todo_queue;
        let first = queue.add("fix the login redirect");
        queue.add("write the migration");
        queue.mark_running(first);
        (state, session_id, dir)
    }

    #[test]
    fn the_pane_shows_your_queue_with_the_agents_steps_under_the_running_item() {
        let (state, _session_id, _dir) = state_with_queue();
        let out = screen(&state, 46, 12);

        assert!(out.contains("TODO"), "{out}");
        assert!(out.contains("fix the login redirect"), "{out}");
        assert!(out.contains("write the migration"), "{out}");
        // The agent's own step, shown as progress for the running item.
        assert!(out.contains("Parse agent logs"), "{out}");
        assert!(out.contains("(2 left)"), "{out}");
    }

    #[test]
    fn your_items_sit_left_of_the_agents_steps() {
        let (state, _session_id, _dir) = state_with_queue();
        let out = screen(&state, 46, 12);

        let indent = |needle: &str| {
            let line = out
                .lines()
                .find(|l| l.contains(needle))
                .unwrap_or_else(|| panic!("{needle} missing from:\n{out}"))
                .trim_start_matches('│')
                .to_string();
            line.len() - line.trim_start().len()
        };

        assert!(
            indent("fix the login redirect") < indent("Parse agent logs"),
            "the agent's progress belongs under your item:\n{out}"
        );
    }

    #[test]
    fn an_empty_queue_says_how_to_fill_it() {
        let (state, _session_id, _dir) = crate::app::tasks_view::tests::fixture();
        let out = screen(&state, 46, 10);
        assert!(out.contains("Press n to add"), "{out}");
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
        assert_eq!(
            single_line("make it\n  show   tasks\n"),
            "make it show tasks"
        );
    }
}
