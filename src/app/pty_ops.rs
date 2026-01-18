use crate::app::AppState;

/// Resize all PTYs to match their respective pane sizes
/// This accounts for which pane each session is displayed in (output vs pinned)
///
/// Note: We only resize PTYs, not vt100 parsers. The parsers are kept large (500 rows)
/// to maintain scrollback history. For alternate screen apps like nvim, the rendering
/// code uses the pane height to display the correct portion of the screen.
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

        // Note: We intentionally do NOT resize the vt100 parser here.
        // The parser maintains a large buffer (500 rows) for scrollback.
        // For fullscreen apps (alternate screen), we only render pane_height rows.
        // For normal terminals, we render all content up to cursor position.
    }
}
