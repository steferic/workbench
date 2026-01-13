use crate::app::AppState;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, state: &AppState) {
    let area = centered_rect(60, 30, frame.area());

    // Clear the background
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Create New Project ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Split into: parent path, input field, preview, help
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Parent path
            Constraint::Length(3), // Input field
            Constraint::Length(2), // Preview
            Constraint::Length(2), // Help
        ])
        .split(inner);

    let path_area = chunks[0];
    let input_area = chunks[1];
    let preview_area = chunks[2];
    let help_area = chunks[3];

    // Show parent path
    let path_display = state
        .ui.file_browser_path
        .to_str()
        .map(|s| {
            if let Some(home) = dirs::home_dir() {
                if let Some(home_str) = home.to_str() {
                    if let Some(stripped) = s.strip_prefix(home_str) {
                        return format!("~{}", stripped);
                    }
                }
            }
            s.to_string()
        })
        .unwrap_or_else(|| "?".to_string());

    let path_widget = Paragraph::new(Line::from(vec![
        Span::styled(" Parent: ", Style::default().fg(Color::Gray)),
        Span::styled(path_display, Style::default().fg(Color::Cyan)),
    ]));
    frame.render_widget(path_widget, path_area);

    // Render input field
    let input_block = Block::default()
        .title(" Project Name ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let input_inner = input_block.inner(input_area);
    frame.render_widget(input_block, input_area);

    let cursor_char = if state.ui.input_buffer.is_empty() {
        "_"
    } else {
        ""
    };

    let input_text = Paragraph::new(Line::from(vec![
        Span::styled(
            &state.ui.input_buffer,
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
        Span::styled(cursor_char, Style::default().fg(Color::Yellow).add_modifier(Modifier::SLOW_BLINK)),
    ]));
    frame.render_widget(input_text, input_inner);

    // Show preview of full path
    let preview_path = if state.ui.input_buffer.is_empty() {
        "<enter project name>".to_string()
    } else {
        state.ui.file_browser_path.join(&state.ui.input_buffer)
            .to_str()
            .map(|s| {
                if let Some(home) = dirs::home_dir() {
                    if let Some(home_str) = home.to_str() {
                        if let Some(stripped) = s.strip_prefix(home_str) {
                            return format!("~{}", stripped);
                        }
                    }
                }
                s.to_string()
            })
            .unwrap_or_else(|| "?".to_string())
    };

    let preview_style = if state.ui.input_buffer.is_empty() {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::Green)
    };

    let preview_widget = Paragraph::new(Line::from(vec![
        Span::styled(" Creates: ", Style::default().fg(Color::Gray)),
        Span::styled(preview_path, preview_style),
    ]));
    frame.render_widget(preview_widget, preview_area);

    // Render help
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[Enter]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw(" Create  "),
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
