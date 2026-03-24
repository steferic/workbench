use crate::app::AppState;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, state: &AppState) {
    let agents = &state.system.user_config.agents;
    let enabled_count = agents.iter().filter(|a| a.enabled).count();
    // Calculate height: header (5 lines) + agents + footer (3 lines)
    let needed_lines = 8 + enabled_count;
    let height_pct =
        ((needed_lines * 100) / frame.area().height.max(1) as usize).clamp(25, 60) as u16;
    let area = centered_rect(40, height_pct, frame.area());
    frame.render_widget(Clear, area);

    let workspace_name = state
        .selected_workspace()
        .map(|w| w.name.as_str())
        .unwrap_or("Unknown");

    let mut content = vec![
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
    ];

    for agent in agents {
        if !agent.enabled {
            continue;
        }
        content.push(Line::from(vec![
            Span::styled(
                format!("  [{}] ", agent.hotkey),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(
                format!("[{}] ", agent.badge),
                Style::default().fg(Color::Magenta),
            ),
            Span::raw(agent.display_name.clone()),
        ]));
    }

    content.push(Line::from(""));
    content.push(Line::from(vec![
        Span::styled("  [t] ", Style::default().fg(Color::Cyan)),
        Span::styled("[T] ", Style::default().fg(Color::Magenta)),
        Span::raw("Terminal"),
    ]));
    content.push(Line::from(""));
    content.push(Line::from(Span::styled(
        "  Press Esc to cancel",
        Style::default().fg(Color::DarkGray),
    )));

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
