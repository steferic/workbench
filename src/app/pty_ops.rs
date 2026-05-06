use crate::app::AppState;
use uuid::Uuid;

/// Resize all PTYs and vt100 parsers to match their respective pane sizes.
/// This accounts for which pane each session is displayed in (output vs pinned).
///
/// Both PTY and parser columns MUST stay in sync. The PTY tells the subprocess
/// its terminal width (TIOCGWINSZ), so the subprocess formats output for that
/// width. If the parser has a different column count, it interprets that output
/// incorrectly — lines wrap at the wrong boundary and fullscreen apps break.
///
/// We only resize parser columns, not rows. The parser's row count
/// (PARSER_BUFFER_ROWS) must be preserved. Deep scrollback uses raw byte replay.
pub fn resize_ptys_to_panes(state: &mut AppState) {
    let output_cols = state.output_pane_cols();
    let pinned_cols = state.pinned_pane_cols();
    let rows = state.pane_rows();

    // Copy pinned IDs since we need mutable state access below
    let pinned_ids: Vec<Uuid> = state.pinned_terminal_ids().to_vec();

    // Resize each PTY based on which pane it belongs to
    for (session_id, handle) in state.system.pty_handles.iter() {
        let cols = if pinned_ids.contains(session_id) {
            pinned_cols
        } else {
            output_cols
        };

        // Resize the PTY - this updates TIOCGWINSZ which apps query for terminal size
        if let Err(err) = handle.resize(rows, cols) {
            crate::logger::warn(format!("failed to resize PTY {session_id}: {err}"));
        }
    }

    // Resize vt100 parsers to match new column widths.
    // We keep the parser's existing row count (scrollback buffer) but update columns
    // so the parser interprets output at the same width the subprocess is targeting.
    for (session_id, parser) in state.system.output_buffers.iter_mut() {
        let cols = if pinned_ids.contains(session_id) {
            pinned_cols
        } else {
            output_cols
        };

        let (parser_rows, parser_cols) = parser.screen().size();
        if parser_cols != cols {
            parser.set_size(parser_rows, cols);
        }
    }

    // Invalidate all replay caches since column changes affect line wrapping
    state.system.replay_caches.clear();
}
