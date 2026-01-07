use crate::app::{AppState, FocusPanel, TextSelection};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

/// Render a specific pinned terminal pane at the given index
pub fn render_at(frame: &mut Frame, area: Rect, state: &mut AppState, pane_index: usize) {
    let is_focused = matches!(state.focus, FocusPanel::PinnedTerminalPane(idx) if idx == pane_index);
    let selection = &state.pinned_text_selections[pane_index];
    let has_selection = selection.start.is_some();

    let border_style = if has_selection {
        Style::default().fg(Color::Yellow)
    } else if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::Magenta)
    };

    // Store the pane area for mouse click detection
    state.pinned_pane_areas[pane_index] = Some((area.x, area.y, area.width, area.height));

    let title = if has_selection {
        " SELECT - y: copy, Esc: cancel ".to_string()
    } else {
        state
            .pinned_terminal_session_at(pane_index)
            .map(|s| format!(" {} [pinned {}] ", s.agent_type.display_name(), pane_index + 1))
            .unwrap_or_else(|| format!(" Pinned {} ", pane_index + 1))
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner_area = block.inner(area);

    // Convert vt100 parser output to ratatui Lines
    let lines: Vec<Line> = if let Some(parser) = state.pinned_terminal_output_at(pane_index) {
        let screen = parser.screen();
        if has_selection {
            convert_vt100_to_lines_with_selection(screen, selection)
        } else {
            convert_vt100_to_lines(screen)
        }
    } else {
        vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No terminal in this slot",
                Style::default().fg(Color::Gray),
            )),
        ]
    };

    let content_length = lines.len();
    let viewport_height = inner_area.height as usize;

    // Calculate scroll position - scroll_offset is offset from bottom
    // 0 = show bottom (latest), higher = scroll up to see older content
    let max_scroll = content_length.saturating_sub(viewport_height);
    let scroll_from_bottom = (state.pinned_scroll_offsets[pane_index] as usize).min(max_scroll);
    let scroll_offset = max_scroll.saturating_sub(scroll_from_bottom);

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

/// Legacy render function for backward compatibility (renders focused pane)
pub fn render(frame: &mut Frame, area: Rect, state: &mut AppState) {
    render_at(frame, area, state, state.focused_pinned_pane);
}

fn convert_vt100_to_lines(screen: &vt100::Screen) -> Vec<Line<'static>> {
    let mut all_lines = Vec::new();
    let (rows, cols) = screen.size();

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

    for row in 0..rows {
        let row_idx = row as usize;
        let mut spans = Vec::new();
        let mut current_text = String::new();
        let mut current_style = Style::default();
        let mut current_selected = false;

        for col in 0..cols {
            let col_idx = col as usize;

            // Check if this cell is within selection
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

    let fg = cell.fgcolor();
    if !matches!(fg, vt100::Color::Default) {
        style = style.fg(convert_vt100_color(fg));
    }

    let bg = cell.bgcolor();
    if !matches!(bg, vt100::Color::Default) {
        style = style.bg(convert_vt100_color(bg));
    }

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
