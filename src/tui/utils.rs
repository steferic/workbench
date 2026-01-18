use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    Frame,
};
use crate::app::TextSelection;

#[derive(Clone, Copy)]
pub struct SelectionBounds {
    pub start_row: usize,
    pub start_col: usize,
    pub end_row: usize,
    pub end_col: usize,
}

#[derive(Clone, Copy)]
pub struct CursorInfo {
    pub row: u16,
    pub col: u16,
    pub hidden: bool,
}

pub fn get_cursor_info(screen: &vt100::Screen) -> CursorInfo {
    let (row, col) = screen.cursor_position();
    CursorInfo {
        row,
        col,
        hidden: screen.hide_cursor(),
    }
}

pub fn render_cursor(
    frame: &mut Frame,
    inner_area: Rect,
    cursor: CursorInfo,
    scroll_offset: usize,
    force_visible: bool,
) {
    // Skip if area is empty, but ignore cursor.hidden when force_visible is true
    // (agents often send hide cursor sequences, but we still want to show it for user input)
    if inner_area.width == 0 || inner_area.height == 0 {
        return;
    }
    if cursor.hidden && !force_visible {
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

pub fn get_selection_bounds(
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

pub fn cell_is_selected(row: usize, col: usize, bounds: SelectionBounds) -> bool {
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

pub fn convert_vt100_to_lines(
    screen: &vt100::Screen,
    selection: Option<SelectionBounds>,
    cursor_row: u16,
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
                let mut cell_style = convert_vt100_cell_style(cell);
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
                // This is critical for fullscreen apps like nvim that use cursor positioning
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

    // Remove trailing empty lines, but preserve lines up to the cursor
    while all_lines.len() > (cursor_row as usize + 1)
        && all_lines.last().map(|l| l.spans.is_empty()).unwrap_or(false)
    {
        all_lines.pop();
    }

    all_lines
}

pub fn convert_vt100_cell_style(cell: &vt100::Cell) -> Style {
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
    // Inverse video - used by many CLI apps to draw their visual cursor
    if cell.inverse() {
        style = style.add_modifier(Modifier::REVERSED);
    }

    style
}

pub fn convert_vt100_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}
