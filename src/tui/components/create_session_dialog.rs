use crate::app::AppState;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, state: &AppState) {
    let area = centered_rect(40, 30, frame.area());

    // Clear the background
    frame.render_widget(Clear, area);

    let workspace_name = state
        .selected_workspace()
        .map(|w| w.name.as_str())
        .unwrap_or("Unknown");

    let content = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  Workspace: {}", workspace_name),
            Style::default().fg(Color::Gray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Select an agent:",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [1] ", Style::default().fg(Color::Cyan)),
            Span::styled("[C] ", Style::default().fg(Color::Magenta)),
            Span::raw("Claude"),
        ]),
        Line::from(vec![
            Span::styled("  [2] ", Style::default().fg(Color::Cyan)),
            Span::styled("[G] ", Style::default().fg(Color::Magenta)),
            Span::raw("Gemini"),
        ]),
        Line::from(vec![
            Span::styled("  [3] ", Style::default().fg(Color::Cyan)),
            Span::styled("[X] ", Style::default().fg(Color::Magenta)),
            Span::raw("Codex"),
        ]),
        Line::from(vec![
            Span::styled("  [4] ", Style::default().fg(Color::Cyan)),
            Span::styled("[K] ", Style::default().fg(Color::Magenta)),
            Span::raw("Grok"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Press Esc to cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let block = Block::default()
        .title(" New Session ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta))
        .style(Style::default().bg(Color::Black));

    let paragraph = Paragraph::new(content).block(block);

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
