use crate::app::AppState;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, _state: &AppState) {
    let area = centered_rect(60, 70, frame.area());

    // Clear the background
    frame.render_widget(Clear, area);

    let help_text = vec![
        Line::from(Span::styled(
            "Workbench - AI Agent Workspace Manager",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Navigation",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled("  j/k, ↑/↓  ", Style::default().fg(Color::Cyan)),
            Span::raw("Move up/down in lists"),
        ]),
        Line::from(vec![
            Span::styled("  h/l, ←/→  ", Style::default().fg(Color::Cyan)),
            Span::raw("Switch between panels"),
        ]),
        Line::from(vec![
            Span::styled("  Tab       ", Style::default().fg(Color::Cyan)),
            Span::raw("Cycle focus between panels"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Workspaces",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled("  n         ", Style::default().fg(Color::Cyan)),
            Span::raw("Create new workspace"),
        ]),
        Line::from(vec![
            Span::styled("  Enter     ", Style::default().fg(Color::Cyan)),
            Span::raw("Select workspace / view sessions"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Sessions",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled("  1         ", Style::default().fg(Color::Cyan)),
            Span::raw("New Claude session"),
        ]),
        Line::from(vec![
            Span::styled("  2         ", Style::default().fg(Color::Cyan)),
            Span::raw("New Gemini session"),
        ]),
        Line::from(vec![
            Span::styled("  3         ", Style::default().fg(Color::Cyan)),
            Span::raw("New Codex session"),
        ]),
        Line::from(vec![
            Span::styled("  4         ", Style::default().fg(Color::Cyan)),
            Span::raw("New Grok session"),
        ]),
        Line::from(vec![
            Span::styled("  Enter     ", Style::default().fg(Color::Cyan)),
            Span::raw("Activate selected session"),
        ]),
        Line::from(vec![
            Span::styled("  s         ", Style::default().fg(Color::Cyan)),
            Span::raw("Stop session (graceful)"),
        ]),
        Line::from(vec![
            Span::styled("  x         ", Style::default().fg(Color::Cyan)),
            Span::raw("Kill session (force)"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Output Pane",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled("  (type)    ", Style::default().fg(Color::Cyan)),
            Span::raw("Send input to active session"),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+H    ", Style::default().fg(Color::Cyan)),
            Span::raw("Return to session list"),
        ]),
        Line::from(vec![
            Span::styled("  Esc       ", Style::default().fg(Color::Cyan)),
            Span::raw("Send escape to agent (interrupt)"),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+C    ", Style::default().fg(Color::Cyan)),
            Span::raw("Send interrupt signal"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "General",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled("  `         ", Style::default().fg(Color::Cyan)),
            Span::raw("Jump to next idle session"),
        ]),
        Line::from(vec![
            Span::styled("  ?         ", Style::default().fg(Color::Cyan)),
            Span::raw("Show this help"),
        ]),
        Line::from(vec![
            Span::styled("  q         ", Style::default().fg(Color::Cyan)),
            Span::raw("Quit workbench"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Press Esc or ? to close",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let block = Block::default()
        .title(" Help ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .style(Style::default().bg(Color::Black));

    let paragraph = Paragraph::new(help_text)
        .block(block)
        .alignment(Alignment::Left);

    frame.render_widget(paragraph, area);
}

use crate::app::PaneHelp;

pub fn render_pane_help(frame: &mut Frame, _state: &AppState, pane: PaneHelp) {
    let (title, help_lines) = match pane {
        PaneHelp::Workspaces => (
            " Workspaces Help ",
            vec![
                Line::from(Span::styled(
                    "Workspaces",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  j/k, ↑/↓  ", Style::default().fg(Color::Cyan)),
                    Span::raw("Move selection up/down"),
                ]),
                Line::from(vec![
                    Span::styled("  Enter     ", Style::default().fg(Color::Cyan)),
                    Span::raw("Select workspace"),
                ]),
                Line::from(vec![
                    Span::styled("  n         ", Style::default().fg(Color::Cyan)),
                    Span::raw("New workspace (create or open)"),
                ]),
                Line::from(vec![
                    Span::styled("  w         ", Style::default().fg(Color::Cyan)),
                    Span::raw("Toggle working/paused status"),
                ]),
                Line::from(vec![
                    Span::styled("  d         ", Style::default().fg(Color::Cyan)),
                    Span::raw("Delete workspace"),
                ]),
                Line::from(vec![
                    Span::styled("  Tab       ", Style::default().fg(Color::Cyan)),
                    Span::raw("Move to next pane"),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "Press h or Esc to close",
                    Style::default().fg(Color::DarkGray),
                )),
            ],
        ),
        PaneHelp::Sessions => (
            " Sessions Help ",
            vec![
                Line::from(Span::styled(
                    "Sessions",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Navigation",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(vec![
                    Span::styled("  j/k, ↑/↓  ", Style::default().fg(Color::Cyan)),
                    Span::raw("Move selection up/down"),
                ]),
                Line::from(vec![
                    Span::styled("  Enter     ", Style::default().fg(Color::Cyan)),
                    Span::raw("Activate session (view output)"),
                ]),
                Line::from(vec![
                    Span::styled("  Tab       ", Style::default().fg(Color::Cyan)),
                    Span::raw("Move to next pane"),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "Create Sessions",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(vec![
                    Span::styled("  1         ", Style::default().fg(Color::Cyan)),
                    Span::raw("New Claude session"),
                ]),
                Line::from(vec![
                    Span::styled("  2         ", Style::default().fg(Color::Cyan)),
                    Span::raw("New Gemini session"),
                ]),
                Line::from(vec![
                    Span::styled("  3         ", Style::default().fg(Color::Cyan)),
                    Span::raw("New Codex session"),
                ]),
                Line::from(vec![
                    Span::styled("  4         ", Style::default().fg(Color::Cyan)),
                    Span::raw("New Grok session"),
                ]),
                Line::from(vec![
                    Span::styled("  t         ", Style::default().fg(Color::Cyan)),
                    Span::raw("New terminal"),
                ]),
                Line::from(vec![
                    Span::styled("  P         ", Style::default().fg(Color::Cyan)),
                    Span::raw("Start parallel task"),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "Session Modifiers",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(vec![
                    Span::styled("  !/Shift+1 ", Style::default().fg(Color::Cyan)),
                    Span::raw("Claude (skip permissions)"),
                ]),
                Line::from(vec![
                    Span::styled("  @/#/$     ", Style::default().fg(Color::Cyan)),
                    Span::raw("Gemini/Codex/Grok (skip perms)"),
                ]),
                Line::from(vec![
                    Span::styled("  Alt+1-4   ", Style::default().fg(Color::Cyan)),
                    Span::raw("Create in isolated worktree"),
                ]),
                Line::from(vec![
                    Span::styled("  Alt+!/@/# ", Style::default().fg(Color::Cyan)),
                    Span::raw("Worktree + skip permissions"),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "Session Actions",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(vec![
                    Span::styled("  s         ", Style::default().fg(Color::Cyan)),
                    Span::raw("Stop session (graceful)"),
                ]),
                Line::from(vec![
                    Span::styled("  x         ", Style::default().fg(Color::Cyan)),
                    Span::raw("Kill session (force)"),
                ]),
                Line::from(vec![
                    Span::styled("  d         ", Style::default().fg(Color::Cyan)),
                    Span::raw("Delete session"),
                ]),
                Line::from(vec![
                    Span::styled("  p         ", Style::default().fg(Color::Cyan)),
                    Span::raw("Pin/unpin to side panel"),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "Worktree Actions",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(vec![
                    Span::styled("  w         ", Style::default().fg(Color::Cyan)),
                    Span::raw("Open terminal in worktree"),
                ]),
                Line::from(vec![
                    Span::styled("  m         ", Style::default().fg(Color::Cyan)),
                    Span::raw("Merge worktree into main"),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "Press h or Esc to close",
                    Style::default().fg(Color::DarkGray),
                )),
            ],
        ),
        PaneHelp::Todos => (
            " Todos Help ",
            vec![
                Line::from(Span::styled(
                    "Todos",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Navigation",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(vec![
                    Span::styled("  j/k, ↑/↓  ", Style::default().fg(Color::Cyan)),
                    Span::raw("Move selection up/down"),
                ]),
                Line::from(vec![
                    Span::styled("  Tab       ", Style::default().fg(Color::Cyan)),
                    Span::raw("Switch tabs (Active/Archived/Reports)"),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "Active Tab",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(vec![
                    Span::styled("  n         ", Style::default().fg(Color::Cyan)),
                    Span::raw("Create new todo"),
                ]),
                Line::from(vec![
                    Span::styled("  Enter     ", Style::default().fg(Color::Cyan)),
                    Span::raw("Run todo with agent"),
                ]),
                Line::from(vec![
                    Span::styled("  y         ", Style::default().fg(Color::Cyan)),
                    Span::raw("Accept suggested todo"),
                ]),
                Line::from(vec![
                    Span::styled("  Y         ", Style::default().fg(Color::Cyan)),
                    Span::raw("Accept all suggested todos"),
                ]),
                Line::from(vec![
                    Span::styled("  x         ", Style::default().fg(Color::Cyan)),
                    Span::raw("Mark as done"),
                ]),
                Line::from(vec![
                    Span::styled("  X         ", Style::default().fg(Color::Cyan)),
                    Span::raw("Archive todo"),
                ]),
                Line::from(vec![
                    Span::styled("  d         ", Style::default().fg(Color::Cyan)),
                    Span::raw("Delete todo"),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "Reports Tab",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(vec![
                    Span::styled("  v         ", Style::default().fg(Color::Cyan)),
                    Span::raw("View report details"),
                ]),
                Line::from(vec![
                    Span::styled("  m         ", Style::default().fg(Color::Cyan)),
                    Span::raw("Merge selected attempt"),
                ]),
                Line::from(vec![
                    Span::styled("  d         ", Style::default().fg(Color::Cyan)),
                    Span::raw("Discard attempt"),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "Press h or Esc to close",
                    Style::default().fg(Color::DarkGray),
                )),
            ],
        ),
        PaneHelp::Utilities => (
            " Utilities Help ",
            vec![
                Line::from(Span::styled(
                    "Utilities",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Navigation",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(vec![
                    Span::styled("  j/k, ↑/↓  ", Style::default().fg(Color::Cyan)),
                    Span::raw("Move selection up/down"),
                ]),
                Line::from(vec![
                    Span::styled("  Tab       ", Style::default().fg(Color::Cyan)),
                    Span::raw("Switch tabs (Util/Sounds/Cfg/Notes)"),
                ]),
                Line::from(vec![
                    Span::styled("  Enter     ", Style::default().fg(Color::Cyan)),
                    Span::raw("Toggle/activate selected item"),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "Util Tab",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(vec![
                    Span::styled("  Top Files ", Style::default().fg(Color::Gray)),
                    Span::raw("Show largest files by LOC"),
                ]),
                Line::from(vec![
                    Span::styled("  Calendar  ", Style::default().fg(Color::Gray)),
                    Span::raw("Display calendar"),
                ]),
                Line::from(vec![
                    Span::styled("  Git History", Style::default().fg(Color::Gray)),
                    Span::raw("Show recent commits"),
                ]),
                Line::from(vec![
                    Span::styled("  File Tree ", Style::default().fg(Color::Gray)),
                    Span::raw("Display project structure"),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "Sounds Tab",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(vec![
                    Span::styled("  Enter     ", Style::default().fg(Color::Cyan)),
                    Span::raw("Toggle sound on/off"),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "Notes Tab",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(vec![
                    Span::styled("  (type)    ", Style::default().fg(Color::Cyan)),
                    Span::raw("Edit notepad text"),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "Press h or Esc to close",
                    Style::default().fg(Color::DarkGray),
                )),
            ],
        ),
    };

    let area = centered_rect(50, 70, frame.area());

    // Clear the background
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .style(Style::default().bg(Color::Black));

    let paragraph = Paragraph::new(help_lines)
        .block(block)
        .alignment(Alignment::Left);

    frame.render_widget(paragraph, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
