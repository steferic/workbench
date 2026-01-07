use crate::app::AppState;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, state: &AppState) {
    let area = centered_rect(70, 70, frame.area());

    // Clear the background
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Select Workspace Directory ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Split into: current path + workspace name preview, file list, help
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Current path + workspace name
            Constraint::Min(5),    // File list
            Constraint::Length(3), // Help
        ])
        .split(inner);

    let path_area = chunks[0];
    let list_area = chunks[1];
    let help_area = chunks[2];

    // Render current path and workspace name preview
    let path_display = state
        .file_browser_path
        .to_str()
        .map(|s| {
            if let Some(home) = dirs::home_dir() {
                if let Some(home_str) = home.to_str() {
                    if s.starts_with(home_str) {
                        return format!("~{}", &s[home_str.len()..]);
                    }
                }
            }
            s.to_string()
        })
        .unwrap_or_else(|| "?".to_string());

    // Show the highlighted entry's name (what will actually be selected)
    let workspace_name = state
        .file_browser_entries
        .get(state.file_browser_selected)
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or_else(|| {
            // Fallback to current directory name if no entry highlighted
            state.file_browser_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
        })
        .to_string();

    let path_widget = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(" Path: ", Style::default().fg(Color::Gray)),
            Span::styled(
                path_display,
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Name: ", Style::default().fg(Color::Gray)),
            Span::styled(
                workspace_name,
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" (will be created)", Style::default().fg(Color::DarkGray)),
        ]),
    ]);
    frame.render_widget(path_widget, path_area);

    // Render directory list
    let visible_height = list_area.height.saturating_sub(2) as usize;
    let total_entries = state.file_browser_entries.len();

    let items: Vec<ListItem> = state
        .file_browser_entries
        .iter()
        .enumerate()
        .skip(state.file_browser_scroll)
        .take(visible_height)
        .map(|(i, path)| {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?")
                .to_string();

            let is_selected = i == state.file_browser_selected;

            // Check if it looks like a code repo (has .git, package.json, Cargo.toml, etc.)
            let is_repo = path.join(".git").exists()
                || path.join("package.json").exists()
                || path.join("Cargo.toml").exists()
                || path.join("go.mod").exists()
                || path.join("mix.exs").exists()
                || path.join("pyproject.toml").exists()
                || path.join("Gemfile").exists();

            let icon = if is_repo { "📁 " } else { "📂 " };

            let style = if is_selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else if is_repo {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::White)
            };

            let prefix = if is_selected { "> " } else { "  " };

            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::raw(icon),
                Span::styled(name, style),
                if is_repo {
                    Span::styled(" (repo)", Style::default().fg(Color::DarkGray))
                } else {
                    Span::raw("")
                },
            ]))
        })
        .collect();

    let list_title = if total_entries == 0 {
        " (empty) ".to_string()
    } else {
        format!(" {} directories ", total_entries)
    };

    let list = List::new(items).block(
        Block::default()
            .title(list_title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    let mut list_state = ListState::default();
    if !state.file_browser_entries.is_empty() {
        list_state.select(Some(state.file_browser_selected - state.file_browser_scroll));
    }

    frame.render_stateful_widget(list, list_area, &mut list_state);

    // Render help
    let help = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("[↑/k]", Style::default().fg(Color::Cyan)),
            Span::raw(" Up  "),
            Span::styled("[↓/j]", Style::default().fg(Color::Cyan)),
            Span::raw(" Down  "),
            Span::styled("[←/h]", Style::default().fg(Color::Cyan)),
            Span::raw(" Parent  "),
            Span::styled("[→/Enter]", Style::default().fg(Color::Cyan)),
            Span::raw(" Open"),
        ]),
        Line::from(vec![
            Span::styled("[Space/Tab]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled(" Select current directory as workspace  ", Style::default().fg(Color::White)),
            Span::styled("[Esc]", Style::default().fg(Color::Yellow)),
            Span::raw(" Cancel"),
        ]),
    ]);
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
