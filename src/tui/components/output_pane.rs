use crate::app::{AppState, FocusPanel, TextSelection};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let is_focused = state.focus == FocusPanel::OutputPane;
    let has_selection = state.text_selection.start.is_some();

    let border_style = if has_selection {
        Style::default().fg(Color::Yellow)
    } else if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let title = if has_selection {
        " SELECT - y: copy, Esc: cancel ".to_string()
    } else if let Some(session) = state.active_session() {
        format!(
            " {} - {} - {} ",
            session.agent_type.display_name(),
            session.short_id(),
            session.duration_string()
        )
    } else if !state.utility_content.is_empty() {
        format!(" {} ", state.selected_utility.name())
    } else {
        " No Active Session ".to_string()
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner_area = block.inner(area);

    // Store the output pane area for mouse coordinate conversion
    state.output_pane_area = Some((area.x, area.y, area.width, area.height));

    // Convert vt100 parser output to ratatui Lines
    let lines: Vec<Line> = if let Some(parser) = state.active_output() {
        let screen = parser.screen();
        if has_selection {
            convert_vt100_to_lines_with_selection(
                screen,
                &state.text_selection,
                state.output_scroll_offset as usize,
            )
        } else {
            convert_vt100_to_lines(screen)
        }
    } else if !state.utility_content.is_empty() {
        // Show utility content when no active session
        state.utility_content.iter()
            .map(|line| Line::from(Span::styled(line.clone(), Style::default().fg(Color::Gray))))
            .collect()
    } else {
        let hint = if state.workspaces.is_empty() {
            vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  Welcome to Workbench!",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "  Press 'n' to create a new workspace",
                    Style::default().fg(Color::Gray),
                )),
                Line::from(Span::styled(
                    "  Press '?' for help",
                    Style::default().fg(Color::Gray),
                )),
            ]
        } else if state.sessions_for_selected_workspace().is_empty() {
            vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  No sessions in this workspace",
                    Style::default().fg(Color::Gray),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "  Press 1-4 to start a new session:",
                    Style::default().fg(Color::Gray),
                )),
                Line::from(Span::styled(
                    "    1 = Claude, 2 = Gemini, 3 = Codex, 4 = Grok",
                    Style::default().fg(Color::DarkGray),
                )),
            ]
        } else {
            vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  Select a session and press Enter to view output",
                    Style::default().fg(Color::Gray),
                )),
            ]
        };
        hint
    };

    let content_length = lines.len();
    let viewport_height = inner_area.height as usize;

    // Calculate max scroll offset (can't scroll past content)
    let max_scroll = content_length.saturating_sub(viewport_height);
    let scroll_offset = (state.output_scroll_offset as usize).min(max_scroll);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((scroll_offset as u16, 0));

    frame.render_widget(paragraph, area);

    // Render scrollbar if content exceeds viewport
    if content_length > viewport_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        let mut scrollbar_state = ScrollbarState::new(max_scroll).position(scroll_offset);
        frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
    }
}

fn convert_vt100_to_lines(screen: &vt100::Screen) -> Vec<Line<'static>> {
    let mut all_lines = Vec::new();
    let (rows, cols) = screen.size();

    // Get visible screen lines
    for row in 0..rows {
        let mut spans = Vec::new();
        let mut current_text = String::new();
        let mut current_style = Style::default();

        for col in 0..cols {
            if let Some(cell) = screen.cell(row, col) {
                let char_str = cell.contents();
                let cell_style = convert_vt100_style(&cell);

                if cell_style != current_style && !current_text.is_empty() {
                    spans.push(Span::styled(current_text.clone(), current_style));
                    current_text.clear();
                }
                current_style = cell_style;
                current_text.push_str(&char_str);
            }
        }

        if !current_text.is_empty() {
            // Trim trailing spaces
            let trimmed = current_text.trim_end();
            if !trimmed.is_empty() {
                spans.push(Span::styled(trimmed.to_string(), current_style));
            }
        }

        all_lines.push(Line::from(spans));
    }

    // Remove trailing empty lines
    while all_lines.last().map(|l| l.spans.is_empty()).unwrap_or(false) {
        all_lines.pop();
    }

    all_lines
}

fn convert_vt100_to_lines_with_selection(
    screen: &vt100::Screen,
    selection: &TextSelection,
    _scroll_offset: usize,
) -> Vec<Line<'static>> {
    let mut all_lines = Vec::new();
    let (rows, cols) = screen.size();

    // Determine selection range if any
    let selection_range = match (selection.start, selection.end) {
        (Some(start), Some(end)) => {
            // Order the selection (start should be before end)
            if start.0 < end.0 || (start.0 == end.0 && start.1 <= end.1) {
                Some((start.0, start.1, end.0, end.1))
            } else {
                Some((end.0, end.1, start.0, start.1))
            }
        }
        _ => None,
    };

    // Get visible screen lines
    for row in 0..rows {
        let row_idx = row as usize;
        let mut spans = Vec::new();
        let mut current_text = String::new();
        let mut current_style = Style::default();
        let mut current_selected = false;

        for col in 0..cols {
            let col_idx = col as usize;

            // Check if this cell is within selection
            // Selection coordinates already include scroll offset (absolute row in buffer)
            // row_idx is the absolute row in the vt100 buffer, so compare directly
            let is_selected =
                if let Some((start_row, start_col, end_row, end_col)) = selection_range {
                    if row_idx > start_row && row_idx < end_row {
                        true
                    } else if row_idx == start_row && row_idx == end_row {
                        col_idx >= start_col && col_idx <= end_col
                    } else if row_idx == start_row {
                        col_idx >= start_col
                    } else if row_idx == end_row {
                        col_idx <= end_col
                    } else {
                        false
                    }
                } else {
                    false
                };

            if let Some(cell) = screen.cell(row, col) {
                let char_str = cell.contents();
                let mut cell_style = convert_vt100_style(&cell);

                // Apply selection highlighting
                if is_selected {
                    cell_style = cell_style.bg(Color::LightBlue).fg(Color::Black);
                }

                let selected_changed = is_selected != current_selected;

                if (cell_style != current_style || selected_changed) && !current_text.is_empty() {
                    spans.push(Span::styled(current_text.clone(), current_style));
                    current_text.clear();
                }

                current_style = cell_style;
                current_selected = is_selected;
                current_text.push_str(&char_str);
            }
        }

        if !current_text.is_empty() {
            // Trim trailing spaces unless within selection
            if current_selected {
                spans.push(Span::styled(current_text, current_style));
            } else {
                let trimmed = current_text.trim_end();
                if !trimmed.is_empty() {
                    spans.push(Span::styled(trimmed.to_string(), current_style));
                }
            }
        }

        all_lines.push(Line::from(spans));
    }

    // Remove trailing empty lines
    while all_lines.last().map(|l| l.spans.is_empty()).unwrap_or(false) {
        all_lines.pop();
    }

    all_lines
}

fn convert_vt100_style(cell: &vt100::Cell) -> Style {
    let mut style = Style::default();

    // Foreground color
    let fg = cell.fgcolor();
    if !matches!(fg, vt100::Color::Default) {
        style = style.fg(convert_vt100_color(fg));
    }

    // Background color
    let bg = cell.bgcolor();
    if !matches!(bg, vt100::Color::Default) {
        style = style.bg(convert_vt100_color(bg));
    }

    // Modifiers
    if cell.bold() {
        style = style.add_modifier(Modifier::BOLD);
    }
    if cell.italic() {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if cell.underline() {
        style = style.add_modifier(Modifier::UNDERLINED);
    }

    style
}

fn convert_vt100_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}
