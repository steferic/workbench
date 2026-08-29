//! Tasks pane — the work queued for the selected agent, and how it is going.
//!
//! It follows the Sessions pane above: whichever agent the session cursor is
//! on is the one whose queue appears here. Your items are the pane; under the
//! one currently running, the agent's own steps show as progress detail. Rows
//! come from `app::tasks_view` so navigation and rendering can never disagree
//! about what is on screen.

use crate::app::{AppState, FocusPanel, TasksTab};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
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

    let block = Block::default()
        .title(Line::from(vec![Span::raw(" MANAGER ")]))
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

    match state.ui.selected_tasks_tab {
        TasksTab::Managers => render_managers_tab(frame, list_area, state, is_focused),
        TasksTab::Objectives => render_objectives_tab(frame, list_area, state, is_focused),
    }
}

fn render_tab_bar(frame: &mut Frame, area: Rect, state: &AppState, is_focused: bool) {
    let t = crate::theme::current();
    let active = if is_focused {
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.fg_dim).add_modifier(Modifier::BOLD)
    };
    let dim = Style::default().fg(t.fg_faint);

    let objectives_count = state
        .selected_workspace()
        .map(|ws| ws.objectives.len())
        .unwrap_or(0);
    let manager_count = crate::app::managers_view::count(state);
    let (managers_style, objectives_style) = match state.ui.selected_tasks_tab {
        TasksTab::Managers => (active, dim),
        TasksTab::Objectives => (dim, active),
    };

    let names = [
        ("Managers", managers_style, format!("({manager_count})")),
        (
            "Objectives",
            objectives_style,
            format!("({objectives_count})"),
        ),
    ];
    let spans = tab_spans(&names, dim, area.width as usize);
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The widest tab bar that fits, in three steps: with counts, without them,
/// then without the padding too.
fn tab_spans<'a>(
    names: &'a [(&'a str, Style, String); 2],
    dim: Style,
    width: usize,
) -> Vec<Span<'a>> {
    for step in 0..3 {
        let mut spans: Vec<Span> = Vec::new();
        for (i, (name, style, count)) in names.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled("│", dim));
            }
            match step {
                0 if count.is_empty() => spans.push(Span::styled(format!(" {name} "), *style)),
                0 => spans.push(Span::styled(format!(" {name}{count} "), *style)),
                1 => spans.push(Span::styled(format!(" {name} "), *style)),
                _ => spans.push(Span::styled(*name, *style)),
            }
        }
        let used: usize = spans.iter().map(|span| span.content.chars().count()).sum();
        if used <= width || step == 2 {
            return spans;
        }
    }
    unreachable!("the loop returns on its last step")
}

/// This project's managers, and what each is waiting on you for.
///
/// Two lines apiece: who it is and whether it is mid-turn, then the one number
/// that asks something of you. A manager with nothing pending says so in
/// words — an empty second line reads as a rendering fault.
fn render_managers_tab(frame: &mut Frame, area: Rect, state: &AppState, is_focused: bool) {
    let t = crate::theme::current();

    if state.selected_workspace().is_none() {
        let msg = Paragraph::new(Line::from(Span::styled(
            "  Open a project first.",
            Style::default().fg(t.fg_faint),
        )));
        frame.render_widget(msg, area);
        return;
    }

    let rows = crate::app::managers_view::rows(state);
    if rows.is_empty() {
        // The keys come first and unbroken: this pane is four content rows in
        // a real layout, and a provider whose number scrolled off is a
        // provider you cannot start.
        let mut lines = vec![Line::from(Span::styled(
            "  No managers yet. Press a number:",
            Style::default().fg(t.fg_faint),
        ))];
        for line in provider_keys(state, area.width.saturating_sub(4) as usize) {
            lines.push(Line::from(Span::styled(
                format!("    {line}"),
                Style::default().fg(t.fg_dim),
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  It suggests work; nothing runs until you",
            Style::default().fg(t.fg_faint),
        )));
        lines.push(Line::from(Span::styled(
            "  approve it.",
            Style::default().fg(t.fg_faint),
        )));
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }

    let selected = state.ui.selected_manager.min(rows.len() - 1);
    let mut items: Vec<ListItem> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let on_cursor = i == selected && is_focused;
        let name_style = if on_cursor {
            Style::default().fg(t.fg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.fg)
        };

        let (icon, icon_color, word) = manager_activity(state, row.session_id);
        items.push(ListItem::new(Line::from(vec![
            Span::styled(format!("  [{}] ", row.badge), Style::default().fg(t.special)),
            Span::styled(row.name.clone(), name_style),
            Span::raw("  "),
            Span::styled(icon, Style::default().fg(icon_color)),
            Span::styled(format!(" {word}"), Style::default().fg(t.fg_faint)),
        ])));

        let detail = if row.pending == 1 {
            ("      1 proposal awaiting you".to_string(), t.active)
        } else if row.pending > 1 {
            (
                format!("      {} proposals awaiting you", row.pending),
                t.active,
            )
        } else {
            ("      nothing proposed yet".to_string(), t.fg_faint)
        };
        items.push(ListItem::new(Line::from(Span::styled(
            detail.0,
            Style::default().fg(detail.1),
        ))));
    }

    let highlight_style = if is_focused {
        Style::default().bg(t.selection_bg)
    } else {
        Style::default()
    };
    let list = List::new(items).highlight_style(highlight_style);
    let mut list_state = ListState::default();
    // Two lines per manager, and the name line is the one to light up.
    list_state.select(Some(selected * 2));
    frame.render_stateful_widget(list, area, &mut list_state);
}

/// The provider hotkeys, as "1 Claude  2 Gemini  ...", wrapped to the pane.
///
/// Read from the same config the keys themselves are read from, so a disabled
/// or renamed provider cannot leave the hint advertising a key that does
/// nothing.
fn provider_keys(state: &AppState, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for agent in state.system.user_config.agents.iter().filter(|a| a.enabled) {
        let entry = format!("{} {}", agent.hotkey, agent.display_name);
        let candidate = if current.is_empty() {
            entry.clone()
        } else {
            format!("{current}  {entry}")
        };
        if candidate.chars().count() > width && !current.is_empty() {
            lines.push(std::mem::replace(&mut current, entry));
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// How a manager is doing right now, in the same vocabulary the Sessions pane
/// uses — a manager stopped on a permission prompt is stuck in exactly the way
/// an agent is, and should not read as merely idle.
fn manager_activity(
    state: &AppState,
    session_id: uuid::Uuid,
) -> (&'static str, ratatui::style::Color, &'static str) {
    use crate::agent_status::Activity;
    use crate::models::SessionStatus;

    let t = crate::theme::current();
    let stopped = state
        .get_session(session_id)
        .map(|session| session.status != SessionStatus::Running)
        .unwrap_or(true);
    if stopped {
        return ("○", t.fg_dim, "stopped");
    }
    match state
        .system
        .agent_status
        .get(&session_id)
        .map(|status| status.activity)
    {
        Some(Activity::NeedsAttention(_)) => ("!", t.warning, "needs you"),
        Some(Activity::Working) => ("●", t.active, "working"),
        Some(Activity::Exited) => ("○", t.fg_dim, "exited"),
        _ => ("◆", t.fg_faint, "idle"),
    }
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

    // Wrapped, not clipped: an objective is a standing priority in the
    // user's own words, and words that fall off the pane edge read as a
    // different priority than the one written. Rows keep their identity —
    // the cursor walks rows, and a row is merely taller when it wraps.
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
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

    let hints: &[(&str, &str)] = if state.ui.selected_tasks_tab == TasksTab::Managers {
        &[
            ("1-9", ":new "),
            ("Enter", ":open "),
            ("d", ":del "),
            ("h", ":help"),
        ]
    } else {
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

    /// A project with one agent in it, on whichever tab the test is about.
    fn state_on(tab: TasksTab) -> (AppState, uuid::Uuid, tempfile::TempDir) {
        let (mut state, session_id, dir) = crate::app::tasks_view::tests::fixture();
        state.ui.selected_tasks_tab = tab;
        (state, session_id, dir)
    }

    #[test]
    fn the_pane_opens_on_the_roster_and_an_empty_one_explains_itself() {
        let (state, _session_id, _dir) = state_on(TasksTab::Managers);
        let out = screen(&state, 72, 12);
        assert!(out.contains("MANAGER"), "{out}");
        assert!(out.contains("Managers(0)"), "{out}");
        assert!(out.contains("Press a number"), "{out}");
        assert!(out.contains("1 Claude"), "the real hotkeys:\n{out}");
        assert!(out.contains("nothing runs until you"), "{out}");
    }

    /// A manager is listed with what it is waiting on you for. The count is
    /// the whole reason to look at this tab.
    #[test]
    fn a_manager_is_listed_with_what_it_is_waiting_on_you_for() {
        let (mut state, _session_id, _dir) = state_on(TasksTab::Managers);
        let workspace_id = state.data.workspaces[0].id;
        let manager = crate::models::Session::new(
            workspace_id,
            crate::models::AgentType::Claude.as_manager(),
            false,
        );
        let short = manager.short_id();
        state
            .data
            .sessions
            .get_mut(&workspace_id)
            .unwrap()
            .push(manager);
        state.data.workspaces[0]
            .proposals
            .push(crate::models::Proposal::new(short, "split the auth module"));

        let out = screen(&state, 72, 12);
        assert!(out.contains("Managers(1)"), "{out}");
        assert!(out.contains("[M]"), "{out}");
        assert!(out.contains("1 proposal awaiting you"), "{out}");
    }

    /// A pane too narrow for the counts must still show every tab name —
    /// otherwise the later tabs look like they do not exist.
    #[test]
    fn a_narrow_pane_drops_the_counts_rather_than_the_tabs() {
        let (state, _session_id, _dir) = state_on(TasksTab::Managers);
        let out = screen(&state, 26, 10);
        assert!(out.contains("Managers"), "{out}");
        assert!(out.contains("Objectives"), "{out}");
        assert!(!out.contains("Managers("), "counts should be gone:\n{out}");
    }





}
