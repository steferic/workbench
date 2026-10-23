//! The three main panes while another Mac is on show (see `app::remote_view`):
//! its projects, the selected project's agents, and that agent's terminal.

use crate::app::remote_view::{self, agents, projects, selected_agent, selected_project};
use crate::app::{AppState, FocusPanel};
use crate::remote::{peers, AgentView};
use crate::tui::utils::{convert_vt100_to_lines_visible, get_content_length};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

fn block(title: String, focused: bool) -> Block<'static> {
    let t = crate::theme::current();
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if focused { t.border_focused } else { t.border }))
}

/// The name Tab moves to next: another Mac, or "this Mac" after the last.
fn next_machine(current: &str) -> String {
    let names: Vec<String> = peers::get()
        .map(|p| p.machines().into_iter().map(|m| m.config.name).collect())
        .unwrap_or_default();
    names
        .iter()
        .position(|n| n == current)
        .and_then(|i| names.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "this Mac".into())
}

/// Why the Mac on show has nothing to list, if it has not answered.
fn machine_trouble(machine: &str) -> Option<String> {
    let found = peers::get()?
        .machines()
        .into_iter()
        .find(|m| m.config.name == machine)?;
    match (&found.error, &found.snapshot) {
        (Some(err), _) => Some(format!("Cannot reach {machine}: {err}")),
        (None, None) => Some(format!("Connecting to {machine}…")),
        _ => None,
    }
}

pub fn workspaces(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = crate::theme::current();
    let Some(machine) = state.ui.remote.machine.clone() else {
        return;
    };
    let focused = state.ui.focus == FocusPanel::WorkspaceList;
    let projects = projects(&machine);
    let title = format!(" {machine} · Workspaces ({}) ", projects.len());
    let outer = block(title, focused);
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    if let Some(trouble) = machine_trouble(&machine) {
        frame.render_widget(
            Paragraph::new(trouble)
                .style(Style::default().fg(t.fg_dim))
                .wrap(Wrap { trim: true }),
            chunks[0],
        );
    } else {
        let selected = selected_project(state).map(|p| p.id);
        let mut list_state = ListState::default();
        let items: Vec<ListItem> = projects
            .iter()
            .enumerate()
            .map(|(i, project)| {
                let is_selected = Some(&project.id) == selected.as_ref();
                if is_selected {
                    list_state.select(Some(i));
                }
                let here = agents(&machine, &project.id);
                let waiting = here
                    .iter()
                    .any(|a| a.prompt.is_some() || a.status == "blocked");
                let working = here.iter().any(|a| a.status == "working");
                let marker = if waiting {
                    Span::styled(
                        "! ",
                        Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
                    )
                } else if working {
                    Span::styled("● ", Style::default().fg(t.active))
                } else {
                    Span::raw("  ")
                };
                let style = if is_selected && focused {
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
                } else if is_selected {
                    Style::default().fg(t.fg)
                } else {
                    Style::default().fg(t.fg_dim)
                };
                ListItem::new(Line::from(vec![
                    Span::raw(if is_selected { "> " } else { "  " }),
                    marker,
                    Span::styled(project.name.clone(), style),
                    Span::styled(format!("  {}", here.len()), Style::default().fg(t.fg_faint)),
                ]))
            })
            .collect();
        let highlight = if focused {
            Style::default()
                .bg(t.selection_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        frame.render_stateful_widget(
            List::new(items).highlight_style(highlight),
            chunks[0],
            &mut list_state,
        );
    }

    let (key, text) = if focused {
        (
            Style::default().fg(t.accent),
            Style::default().fg(t.fg_faint),
        )
    } else {
        (
            Style::default().fg(t.fg_faint),
            Style::default().fg(t.inactive),
        )
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("tab", key),
            Span::styled(format!(":{}", next_machine(&machine)), text),
        ])),
        chunks[1],
    );
}

fn label(agent: &AgentView) -> String {
    agent
        .alias
        .clone()
        .unwrap_or_else(|| agent.provider.clone())
}

fn status_span(agent: &AgentView) -> Span<'static> {
    let t = crate::theme::current();
    let (text, color) = match agent.status.as_str() {
        _ if agent.prompt.is_some() => ("needs you", t.warning),
        "blocked" => ("needs you", t.warning),
        "working" => ("working", t.active),
        "idle" => ("idle", t.fg_dim),
        "stopped" => ("stopped", t.fg_faint),
        _ => ("?", t.fg_faint),
    };
    Span::styled(text.to_string(), Style::default().fg(color))
}

pub fn sessions(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = crate::theme::current();
    let Some(machine) = state.ui.remote.machine.clone() else {
        return;
    };
    let focused = state.ui.focus == FocusPanel::SessionList;
    let project = selected_project(state);
    let rows = project
        .as_ref()
        .map(|p| agents(&machine, &p.id))
        .unwrap_or_default();
    let outer = block(format!(" Agents ({}) ", rows.len()), focused);
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new("No agents in this project.").style(Style::default().fg(t.fg_dim)),
            inner,
        );
        return;
    }
    let selected = selected_agent(state).map(|a| a.id);
    let mut list_state = ListState::default();
    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(i, agent)| {
            let is_selected = Some(&agent.id) == selected.as_ref();
            if is_selected {
                list_state.select(Some(i));
            }
            let style = if is_selected && focused {
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default().fg(t.fg)
            } else {
                Style::default().fg(t.fg_dim)
            };
            ListItem::new(Line::from(vec![
                Span::raw(if is_selected { "> " } else { "  " }),
                Span::styled(format!("{} ", label(agent)), style),
                Span::styled(format!("{} ", agent.id), Style::default().fg(t.fg_faint)),
                status_span(agent),
            ]))
        })
        .collect();
    let highlight = if focused {
        Style::default()
            .bg(t.selection_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    frame.render_stateful_widget(
        List::new(items).highlight_style(highlight),
        inner,
        &mut list_state,
    );
}

pub fn output(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let t = crate::theme::current();
    let Some(machine) = state.ui.remote.machine.clone() else {
        return;
    };
    let focused = state.ui.focus == FocusPanel::OutputPane;
    let Some(agent) = selected_agent(state) else {
        let outer = block(format!(" {machine} "), focused);
        let inner = outer.inner(area);
        frame.render_widget(outer, area);
        let text =
            machine_trouble(&machine).unwrap_or_else(|| "Select an agent on the left.".into());
        frame.render_widget(
            Paragraph::new(text).style(Style::default().fg(t.fg_dim)),
            inner,
        );
        return;
    };
    let title = format!(
        " {} - {} - {} @ {machine} ",
        label(&agent),
        agent.id,
        agent.project
    );
    let outer = block(title, focused);
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    if agent.status == "stopped" {
        frame.render_widget(
            Paragraph::new(format!(
                "This agent is stopped on {machine}. Start it there, or send it a message from the Machines window (F10): a message starts it."
            ))
            .style(Style::default().fg(t.fg_dim))
            .wrap(Wrap { trim: true }),
            inner,
        );
        return;
    }
    let Some(parser) = remote_view::mirror(state) else {
        frame.render_widget(
            Paragraph::new(format!("Reading the terminal from {machine}…"))
                .style(Style::default().fg(t.fg_dim)),
            inner,
        );
        return;
    };
    let screen = parser.screen();
    let height = inner.height as usize;
    let (cursor_row, _) = screen.cursor_position();
    // The bottom of what is written, as the live view of a local agent
    // shows it. A screen taller than this pane loses its top rows.
    let content = if screen.alternate_screen() {
        screen.size().0 as usize
    } else {
        get_content_length(screen, cursor_row)
    };
    let start = content.saturating_sub(height);
    let lines = convert_vt100_to_lines_visible(
        screen,
        None,
        cursor_row,
        Some(screen.size().0),
        Some(start),
        Some(height),
    );
    frame.render_widget(Paragraph::new(lines), inner);
}
