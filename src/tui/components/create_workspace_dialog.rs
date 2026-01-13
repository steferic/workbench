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

    let title = if state.ui.workspace_create_mode {
        " Select Parent Directory (Create New) "
    } else {
        " Select Workspace Directory (Open Existing) "
    };

    let border_color = if state.ui.workspace_create_mode {
        Color::Yellow
    } else {
        Color::Green
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Split into: current path + info, optional search, file list, help
    let constraints: Vec<Constraint> = if state.ui.workspace_create_mode {
        vec![
            Constraint::Length(3), // Current path + info
            Constraint::Min(5),    // File list
            Constraint::Length(3), // Help
        ]
    } else {
        vec![
            Constraint::Length(3), // Current path + info
            Constraint::Length(3), // Search input
            Constraint::Min(5),    // File list
            Constraint::Length(3), // Help
        ]
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let (path_area, search_area, list_area, help_area) = if state.ui.workspace_create_mode {
        (chunks[0], None, chunks[1], chunks[2])
    } else {
        (chunks[0], Some(chunks[1]), chunks[2], chunks[3])
    };

    // Render current path and workspace name preview
    let path_display = shorten_home_path(&state.ui.file_browser_path);

    // Show the highlighted entry's name (what will actually be selected)
    let workspace_name = state
        .ui.file_browser_entries
        .get(state.ui.file_browser_selected)
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or_else(|| {
            // Fallback to current directory name if no entry highlighted
            state.ui.file_browser_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
        })
        .to_string();

    let path_widget = if state.ui.workspace_create_mode {
        // Create new mode: show parent path info
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(" Parent: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    path_display,
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from(vec![
                Span::styled(" New project will be created in this folder", Style::default().fg(Color::DarkGray)),
            ]),
        ])
    } else {
        // Open existing mode: show what will be selected
        Paragraph::new(vec![
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
                Span::styled(" (will be added)", Style::default().fg(Color::DarkGray)),
            ]),
        ])
    };
    frame.render_widget(path_widget, path_area);

    if let Some(search_area) = search_area {
        let search_block = Block::default()
            .title(" Find ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));
        let search_inner = search_block.inner(search_area);
        frame.render_widget(search_block, search_area);

        let query = state.ui.file_browser_query.as_str();
        let placeholder = if query.is_empty() {
            "Type path or filter"
        } else {
            ""
        };

        let search_line = Line::from(vec![
            Span::styled(" Find: ", Style::default().fg(Color::Gray)),
            Span::styled(query, Style::default().fg(Color::White)),
            Span::styled(placeholder, Style::default().fg(Color::DarkGray)),
        ]);

        frame.render_widget(Paragraph::new(search_line), search_inner);

        let cursor_x = search_inner.x.saturating_add(" Find: ".len() as u16)
            .saturating_add(query.len() as u16);
        let max_x = search_inner.x.saturating_add(search_inner.width.saturating_sub(1));
        let x = cursor_x.min(max_x);
        frame.set_cursor_position((x, search_inner.y));
    }

    // Render directory list
    let visible_height = list_area.height.saturating_sub(2) as usize;
    let total_entries = state.ui.file_browser_entries.len();

    let max_width = list_area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = if state.ui.file_browser_entries.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            if state.ui.file_browser_query.is_empty() {
                "  No directories found"
            } else {
                "  No matches"
            },
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        state
            .ui
            .file_browser_entries
            .iter()
            .enumerate()
            .skip(state.ui.file_browser_scroll)
            .take(visible_height)
            .map(|(i, path)| {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
                    .to_string();

                let is_selected = i == state.ui.file_browser_selected;

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
                let mut spans = vec![
                    Span::styled(prefix, style),
                    Span::raw(icon),
                    Span::styled(name.clone(), style),
                ];

                if is_repo {
                    spans.push(Span::styled(" (repo)", Style::default().fg(Color::DarkGray)));
                }

                let base_len = prefix.chars().count()
                    + icon.chars().count()
                    + name.chars().count()
                    + if is_repo { " (repo)".chars().count() } else { 0 };

                let available = max_width
                    .saturating_sub(base_len)
                    .saturating_sub(" - ".len());

                if available > 0 {
                    let display_path = shorten_home_path(path);
                    let truncated = truncate_path(&display_path, available);
                    if !truncated.is_empty() {
                        spans.push(Span::styled(
                            format!(" - {}", truncated),
                            Style::default().fg(Color::DarkGray),
                        ));
                    }
                }

                ListItem::new(Line::from(spans))
            })
            .collect()
    };

    let list_title = if total_entries == 0 {
        " (empty) ".to_string()
    } else if state.ui.file_browser_query.is_empty() {
        format!(" {} directories ", total_entries)
    } else {
        format!(
            " {} matches ({} total) ",
            total_entries,
            state.ui.file_browser_all_entries.len()
        )
    };

    let list = List::new(items).block(
        Block::default()
            .title(list_title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    let mut list_state = ListState::default();
    if !state.ui.file_browser_entries.is_empty() {
        list_state.select(Some(state.ui.file_browser_selected - state.ui.file_browser_scroll));
    }

    frame.render_stateful_widget(list, list_area, &mut list_state);

    // Render help based on mode
    let help = if state.ui.workspace_create_mode {
        Paragraph::new(vec![
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
                Span::styled("[Space/Tab]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled(" Create here → Enter name  ", Style::default().fg(Color::White)),
                Span::styled("[Esc]", Style::default().fg(Color::Yellow)),
                Span::raw(" Cancel"),
            ]),
        ])
    } else {
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("[↑]", Style::default().fg(Color::Cyan)),
                Span::raw(" Up  "),
                Span::styled("[↓]", Style::default().fg(Color::Cyan)),
                Span::raw(" Down  "),
                Span::styled("[←]", Style::default().fg(Color::Cyan)),
                Span::raw(" Parent  "),
                Span::styled("[→/Enter]", Style::default().fg(Color::Cyan)),
                Span::raw(" Open"),
            ]),
            Line::from(vec![
                Span::styled("[Type]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::styled(" Path or filter  ", Style::default().fg(Color::White)),
                Span::styled("[Space/Tab]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::styled(" Select as workspace  ", Style::default().fg(Color::White)),
                Span::styled("[Esc]", Style::default().fg(Color::Yellow)),
                Span::raw(" Cancel"),
            ]),
        ])
    };
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

fn shorten_home_path(path: &std::path::Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let (Some(home_str), Some(path_str)) = (home.to_str(), path.to_str()) {
            if let Some(stripped) = path_str.strip_prefix(home_str) {
                return format!("~{}", stripped);
            }
        }
    }
    path.to_string_lossy().to_string()
}

fn truncate_path(path: &str, max_len: usize) -> String {
    let path_len = path.chars().count();
    if path_len <= max_len {
        return path.to_string();
    }
    if max_len <= 3 {
        return "...".chars().take(max_len).collect();
    }
    let tail_len = max_len - 3;
    let tail: String = path
        .chars()
        .rev()
        .take(tail_len)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("...{}", tail)
}
