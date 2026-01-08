use crate::app::{AppState, WorkspaceAction};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, state: &AppState) {
    let area = centered_rect(50, 30, frame.area());

    // Clear the background
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Add Workspace ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Split into: options list + help
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3), // Options list
            Constraint::Length(2), // Help
        ])
        .split(inner);

    let list_area = chunks[0];
    let help_area = chunks[1];

    // Render options
    let items: Vec<ListItem> = WorkspaceAction::all()
        .iter()
        .map(|action| {
            let is_selected = *action == state.ui.selected_workspace_action;

            let style = if is_selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let prefix = if is_selected { "> " } else { "  " };

            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(prefix, style),
                    Span::styled(action.icon(), Style::default().fg(Color::Yellow)),
                    Span::raw(" "),
                    Span::styled(action.name(), style),
                ]),
                Line::from(vec![
                    Span::raw("    "),
                    Span::styled(action.description(), Style::default().fg(Color::DarkGray)),
                ]),
            ])
        })
        .collect();

    let list = List::new(items).highlight_style(Style::default().add_modifier(Modifier::BOLD));

    frame.render_widget(list, list_area);

    // Render help
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[↑/k]", Style::default().fg(Color::Cyan)),
        Span::raw(" Up  "),
        Span::styled("[↓/j]", Style::default().fg(Color::Cyan)),
        Span::raw(" Down  "),
        Span::styled("[Enter]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw(" Select  "),
        Span::styled("[Esc]", Style::default().fg(Color::Yellow)),
        Span::raw(" Cancel"),
    ]));
    frame.render_widget(help, help_area);
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
