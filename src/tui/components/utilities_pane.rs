use crate::app::{AppState, ConfigItem, FocusPanel, UtilityItem, UtilitySection};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let is_focused = state.focus == FocusPanel::UtilitiesPane;
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // Create outer block
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    // Split inner area: tabs row + content + action bar (1 row)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)])
        .split(inner_area);

    let tabs_area = chunks[0];
    let content_area = chunks[1];
    let action_area = chunks[2];

    // Render horizontal tabs
    let utils_active = state.utility_section == UtilitySection::Utilities;
    let config_active = state.utility_section == UtilitySection::GlobalConfig;
    let notepad_active = state.utility_section == UtilitySection::Notepad;

    let tab_style = |active: bool| {
        if active && is_focused {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else if active {
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
        } else {
            Style::default().fg(Color::DarkGray)
        }
    };

    let tabs = Paragraph::new(Line::from(vec![
        Span::styled(" Util ", tab_style(utils_active)),
        Span::styled("|", Style::default().fg(Color::DarkGray)),
        Span::styled(" Cfg ", tab_style(config_active)),
        Span::styled("|", Style::default().fg(Color::DarkGray)),
        Span::styled(" Notes ", tab_style(notepad_active)),
    ]));
    frame.render_widget(tabs, tabs_area);

    // Render content based on active section
    match state.utility_section {
        UtilitySection::Utilities => {
            render_utilities_list(frame, content_area, state, is_focused);
        }
        UtilitySection::GlobalConfig => {
            render_config_list(frame, content_area, state, is_focused);
        }
        UtilitySection::Notepad => {
            render_notepad(frame, content_area, state, is_focused);
        }
    }

    // Render action bar (1 row, compact)
    let action_style = if is_focused {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::Rgb(60, 60, 60))
    };
    let key_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let action_bar = match state.utility_section {
        UtilitySection::Notepad => {
            Paragraph::new(Line::from(vec![
                Span::styled("tab", key_style),
                Span::styled(":switch ", action_style),
                Span::styled("type", key_style),
                Span::styled(":edit", action_style),
            ]))
        }
        _ => {
            Paragraph::new(Line::from(vec![
                Span::styled("tab", key_style),
                Span::styled(":switch ", action_style),
                Span::styled("⏎", key_style),
                Span::styled(":toggle", action_style),
            ]))
        }
    };

    frame.render_widget(action_bar, action_area);
}

fn render_utilities_list(frame: &mut Frame, area: Rect, state: &AppState, is_focused: bool) {
    let items: Vec<ListItem> = UtilityItem::all()
        .iter()
        .map(|item| {
            let is_selected = *item == state.selected_utility;

            let style = if is_selected && is_focused {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::Gray)
            };

            let prefix = if is_selected { "> " } else { "  " };

            // Show toggle indicator for toggle-able utilities
            let toggle_indicator = match item {
                UtilityItem::BrownNoise => {
                    if state.brown_noise_playing {
                        Span::styled(" [ON]", Style::default().fg(Color::Green))
                    } else {
                        Span::styled(" [OFF]", Style::default().fg(Color::Red))
                    }
                }
                _ => Span::raw(""),
            };

            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::raw(format!("{} ", item.icon())),
                Span::styled(item.name(), style),
                toggle_indicator,
            ]))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    let mut list_state = ListState::default();
    let selected_idx = UtilityItem::all()
        .iter()
        .position(|i| *i == state.selected_utility);
    list_state.select(selected_idx);

    frame.render_stateful_widget(list, area, &mut list_state);
}

fn render_config_list(frame: &mut Frame, area: Rect, state: &AppState, is_focused: bool) {
    let items: Vec<ListItem> = ConfigItem::all()
        .iter()
        .map(|item| {
            let is_selected = *item == state.selected_config;

            let style = if is_selected && is_focused {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::Gray)
            };

            let prefix = if is_selected { "> " } else { "  " };

            // Show toggle state
            let toggle_indicator = match item {
                ConfigItem::ToggleBanner => {
                    if state.banner_visible {
                        Span::styled(" [ON]", Style::default().fg(Color::Green))
                    } else {
                        Span::styled(" [OFF]", Style::default().fg(Color::Red))
                    }
                }
            };

            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::raw(format!("{} ", item.icon())),
                Span::styled(item.name(), style),
                toggle_indicator,
            ]))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    let mut list_state = ListState::default();
    let selected_idx = ConfigItem::all()
        .iter()
        .position(|i| *i == state.selected_config);
    list_state.select(selected_idx);

    frame.render_stateful_widget(list, area, &mut list_state);
}

fn render_notepad(frame: &mut Frame, area: Rect, state: &AppState, is_focused: bool) {
    let content = state.current_notepad();

    // Build the text with cursor
    let cursor_pos = state.notepad_cursor_pos.min(content.len());

    let text_style = if is_focused {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::Gray)
    };

    let cursor_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan);

    // Split content into lines for display
    let lines: Vec<Line> = if content.is_empty() {
        if is_focused {
            // Show cursor on empty content
            vec![Line::from(vec![
                Span::styled("█", cursor_style),
            ])]
        } else {
            vec![Line::from(vec![
                Span::styled("(empty - start typing)", Style::default().fg(Color::DarkGray)),
            ])]
        }
    } else {
        // Find which line and column the cursor is on
        let before_cursor = &content[..cursor_pos];

        let mut lines = Vec::new();
        let content_lines: Vec<&str> = content.split('\n').collect();

        // Calculate cursor line and column
        let cursor_line = before_cursor.matches('\n').count();
        let cursor_col = before_cursor.rfind('\n')
            .map(|pos| before_cursor.len() - pos - 1)
            .unwrap_or(before_cursor.len());

        for (line_idx, line_content) in content_lines.iter().enumerate() {
            if is_focused && line_idx == cursor_line {
                // This line has the cursor
                let mut spans = Vec::new();

                if cursor_col > 0 {
                    let before = &line_content[..cursor_col.min(line_content.len())];
                    spans.push(Span::styled(before.to_string(), text_style));
                }

                // Cursor character
                if cursor_col < line_content.len() {
                    let cursor_char = line_content.chars().nth(cursor_col).unwrap_or(' ');
                    spans.push(Span::styled(cursor_char.to_string(), cursor_style));

                    // After cursor
                    if cursor_col + cursor_char.len_utf8() < line_content.len() {
                        let after_start = cursor_col + cursor_char.len_utf8();
                        spans.push(Span::styled(line_content[after_start..].to_string(), text_style));
                    }
                } else {
                    // Cursor at end of line
                    spans.push(Span::styled("█", cursor_style));
                }

                lines.push(Line::from(spans));
            } else {
                lines.push(Line::from(Span::styled(line_content.to_string(), text_style)));
            }
        }

        // Handle cursor at very end (after last newline)
        if is_focused && content.ends_with('\n') && cursor_pos == content.len() {
            lines.push(Line::from(Span::styled("█", cursor_style)));
        }

        lines
    };

    // Apply scroll offset
    let visible_height = area.height as usize;

    // Auto-scroll to keep cursor visible
    let cursor_line = content[..cursor_pos].matches('\n').count();
    let scroll_offset = if cursor_line >= state.notepad_scroll_offset + visible_height {
        cursor_line.saturating_sub(visible_height - 1)
    } else if cursor_line < state.notepad_scroll_offset {
        cursor_line
    } else {
        state.notepad_scroll_offset
    };

    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(scroll_offset)
        .take(visible_height)
        .collect();

    let paragraph = Paragraph::new(visible_lines)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}
