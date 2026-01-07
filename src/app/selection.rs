use crate::app::{AppState, TextSelection};

/// Extract selected text from vt100 screen based on selection start and end positions
pub fn extract_selected_text(
    screen: &vt100::Screen,
    start: (usize, usize),
    end: (usize, usize),
) -> String {
    let (rows, cols) = screen.size();

    // Order the selection (start should be before end)
    let (start_row, start_col, end_row, end_col) = if start.0 < end.0
        || (start.0 == end.0 && start.1 <= end.1)
    {
        (start.0, start.1, end.0, end.1)
    } else {
        (end.0, end.1, start.0, start.1)
    };

    let mut result = String::new();

    for row in start_row..=end_row.min(rows as usize - 1) {
        let row_start = if row == start_row { start_col } else { 0 };
        let row_end = if row == end_row {
            end_col.min(cols as usize)
        } else {
            cols as usize
        };

        let mut line = String::new();
        for col in row_start..=row_end {
            if let Some(cell) = screen.cell(row as u16, col as u16) {
                line.push_str(&cell.contents());
            }
        }

        // Trim trailing whitespace from each line
        let trimmed = line.trim_end();
        result.push_str(trimmed);

        // Add newline between rows (but not at the very end)
        if row < end_row && row < rows as usize - 1 {
            result.push('\n');
        }
    }

    result
}

/// Clear all pinned text selections
pub fn clear_all_pinned_selections(state: &mut AppState) {
    for sel in state.pinned_text_selections.iter_mut() {
        *sel = TextSelection::default();
    }
}

/// Copy text to clipboard using pbcopy on macOS
pub fn copy_to_clipboard(text: &str) {
    if text.is_empty() {
        return;
    }
    if let Ok(mut child) = std::process::Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        if let Some(stdin) = child.stdin.as_mut() {
            use std::io::Write;
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}
