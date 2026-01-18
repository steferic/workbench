use crate::app::{AppState, FocusPanel, InputMode, TextSelection};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

/// Render a specific visible session pane at the given index
pub fn render_at(frame: &mut Frame, area: Rect, state: &mut AppState, pane_index: usize) {
    let is_focused = matches!(state.focus, FocusPanel::SessionPane(idx) if idx == pane_index);

    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::Magenta)
    };

    // Store the pane area for mouse click detection
    state.session_pane_areas[pane_index] = Some((area.x, area.y, area.width, area.height));

    let title = state
        .visible_session_at(pane_index)
        .map(|s| format!(" {} [{}] ", s.agent_type.display_name(), pane_index + 1))
        .unwrap_or_else(|| format!(" Session {} ", pane_index + 1));

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner_area = block.inner(area);

    let mut cursor_state = None;
    // Convert vt100 parser output to ratatui Lines
    let lines: Vec<Line> = if let Some(parser) = state.visible_session_output_at(pane_index) {
        let screen = parser.screen();
        cursor_state = Some(cursor_info(screen));
        let default_selection = TextSelection::default();
        let selection = selection_bounds(
            state
                .session_text_selections
                .get(pane_index)
                .unwrap_or(&default_selection),
            screen.size(),
        );
        convert_vt100_to_lines(screen, selection)
    } else {
        state.session_content_lengths[pane_index] = 0;
        vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No session in this slot",
                Style::default().fg(Color::Gray),
            )),
        ]
    };

    let content_length = lines.len();
    state.session_content_lengths[pane_index] = content_length;
    let viewport_height = inner_area.height as usize;

    // Calculate scroll position - scroll_offset is offset from bottom
    // 0 = show bottom (latest), higher = scroll up to see older content
    let max_scroll = content_length.saturating_sub(viewport_height);
    let scroll_from_bottom = (state.session_scroll_offsets[pane_index] as usize).min(max_scroll);
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

    if is_focused && state.input_mode == InputMode::Normal && scroll_from_bottom == 0 {
        if let Some(info) = cursor_state {
            render_cursor(frame, inner_area, info, scroll_offset);
        }
    }
}

/// Legacy render function for backward compatibility (renders focused pane)
pub fn render(frame: &mut Frame, area: Rect, state: &mut AppState) {
    render_at(frame, area, state, state.focused_session_pane);
}

#[derive(Clone, Copy)]
struct SelectionBounds {
    start_row: usize,
    start_col: usize,
    end_row: usize,
    end_col: usize,
}

fn selection_bounds(
    selection: &TextSelection,
    (rows, cols): (u16, u16),
) -> Option<SelectionBounds> {
    let start = selection.start?;
    let end = selection.end?;
    if rows == 0 || cols == 0 {
        return None;
    }

    let (mut start_row, mut start_col) = start;
    let (mut end_row, mut end_col) = end;

    if start_row > end_row || (start_row == end_row && start_col > end_col) {
        std::mem::swap(&mut start_row, &mut end_row);
        std::mem::swap(&mut start_col, &mut end_col);
    }

    let max_row = rows.saturating_sub(1) as usize;
    let max_col = cols.saturating_sub(1) as usize;

    Some(SelectionBounds {
        start_row: start_row.min(max_row),
        start_col: start_col.min(max_col),
        end_row: end_row.min(max_row),
        end_col: end_col.min(max_col),
    })
}

fn cell_is_selected(row: usize, col: usize, bounds: SelectionBounds) -> bool {
    if row < bounds.start_row || row > bounds.end_row {
        return false;
    }

    if bounds.start_row == bounds.end_row {
        return col >= bounds.start_col && col <= bounds.end_col;
    }

    if row == bounds.start_row {
        return col >= bounds.start_col;
    }
    if row == bounds.end_row {
        return col <= bounds.end_col;
    }

    true
}

fn convert_vt100_to_lines(
    screen: &vt100::Screen,
    selection: Option<SelectionBounds>,
) -> Vec<Line<'static>> {
    let mut all_lines = Vec::new();
    let (rows, cols) = screen.size();

    for row in 0..rows {
        let mut spans = Vec::new();
        let mut current_text = String::new();
        let mut current_style = Style::default();
        let row_has_selection = selection
            .map(|bounds| (row as usize) >= bounds.start_row && (row as usize) <= bounds.end_row)
            .unwrap_or(false);

        for col in 0..cols {
            if let Some(cell) = screen.cell(row, col) {
                let char_str = cell.contents();
                let mut cell_style = convert_vt100_style(&cell);
                if let Some(bounds) = selection {
                    if cell_is_selected(row as usize, col as usize, bounds) {
                        cell_style = cell_style.add_modifier(Modifier::REVERSED);
                    }
                }

                if cell_style != current_style && !current_text.is_empty() {
                    spans.push(Span::styled(current_text.clone(), current_style));
                    current_text.clear();
                }
                current_style = cell_style;

                // Empty cells must be rendered as spaces to maintain column alignment
                if char_str.is_empty() {
                    current_text.push(' ');
                } else {
                    current_text.push_str(&char_str);
                }
            }
        }

        if !current_text.is_empty() {
            let text = if row_has_selection {
                current_text
            } else {
                current_text.trim_end().to_string()
            };
            if !text.is_empty() {
                spans.push(Span::styled(text, current_style));
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

#[derive(Clone, Copy)]
struct CursorInfo {
    row: u16,
    col: u16,
    hidden: bool,
}

fn cursor_info(screen: &vt100::Screen) -> CursorInfo {
    let (row, col) = screen.cursor_position();
    CursorInfo {
        row,
        col,
        hidden: screen.hide_cursor(),
    }
}

fn render_cursor(
    frame: &mut Frame,
    inner_area: Rect,
    cursor: CursorInfo,
    scroll_offset: usize,
) {
    if cursor.hidden || inner_area.width == 0 || inner_area.height == 0 {
        return;
    }

    let row = cursor.row as usize;
    if row < scroll_offset {
        return;
    }

    let row_in_view = row - scroll_offset;
    if row_in_view >= inner_area.height as usize {
        return;
    }

    let max_col = inner_area.width.saturating_sub(1) as usize;
    let x = inner_area.x + (cursor.col as usize).min(max_col) as u16;
    let y = inner_area.y + row_in_view as u16;
    frame.set_cursor_position((x, y));
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
