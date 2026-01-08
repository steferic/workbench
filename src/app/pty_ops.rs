use crate::app::AppState;

/// Resize all PTYs and vt100 parsers to match their respective pane sizes
/// This accounts for which pane each session is displayed in (output vs pinned)
pub fn resize_ptys_to_panes(state: &mut AppState) {
    let output_cols = state.output_pane_cols();
    let pinned_cols = state.pinned_pane_cols();
    let rows = state.pane_rows();
    let parser_rows = 500u16; // Keep large scrollback

    // Get all pinned terminal IDs for the current workspace
    let pinned_ids = state.pinned_terminal_ids();

    // Resize each PTY and parser based on which pane it belongs to
    for (session_id, handle) in state.system.pty_handles.iter() {
        let cols = if pinned_ids.contains(session_id) {
            pinned_cols
        } else {
            output_cols
        };

        // Resize the PTY
        let _ = handle.resize(rows, cols);

        // Resize the vt100 parser
        if let Some(parser) = state.system.output_buffers.get_mut(session_id) {
            parser.set_size(parser_rows, cols);
        }
    }
}
