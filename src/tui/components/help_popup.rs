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
            Span::styled("  Shift+1-4 ", Style::default().fg(Color::Cyan)),
            Span::raw("New session (dangerous permissions)"),
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
            Span::styled("  Esc       ", Style::default().fg(Color::Cyan)),
            Span::raw("Return to session list"),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+C    ", Style::default().fg(Color::Cyan)),
            Span::raw("Send interrupt to agent"),
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
