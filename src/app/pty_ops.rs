use crate::app::AppState;

/// Resize all PTYs to match their respective pane sizes
/// This accounts for which pane each session is displayed in (output vs pinned)
///
/// NOTE: We only resize PTYs, NOT vt100 parsers. The vt100 docs say that
/// set_size() with smaller dimensions LOSES data instead of moving it to
/// scrollback. So we keep parsers at their original size to preserve history.
/// The rendering code handles displaying the correct viewport.
pub fn resize_ptys_to_panes(state: &mut AppState) {
    let output_cols = state.output_pane_cols();
    let pinned_cols = state.pinned_pane_cols();
    let rows = state.pane_rows();

    // Get all pinned terminal IDs for the current workspace
    let pinned_ids = state.pinned_terminal_ids();

    // Resize each PTY based on which pane it belongs to
    for (session_id, handle) in state.system.pty_handles.iter() {
        let cols = if pinned_ids.contains(session_id) {
            pinned_cols
        } else {
            output_cols
        };

        // Resize the PTY - this updates TIOCGWINSZ which apps query for terminal size
        let _ = handle.resize(rows, cols);
    }

    // NOTE: We intentionally do NOT resize vt100 parsers here.
    // Resizing the parser to smaller dimensions loses content!
    // The parser keeps a large buffer, and rendering clips to viewport.
}
