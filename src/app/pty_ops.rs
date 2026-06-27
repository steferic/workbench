use crate::app::AppState;
use std::collections::HashSet;
use uuid::Uuid;

/// Resize all PTYs and vt100 parsers to match their respective pane sizes.
/// This accounts for which pane each session is displayed in (output vs pinned).
///
/// Both PTY and parser columns MUST stay in sync. The PTY tells the subprocess
/// its terminal width (TIOCGWINSZ), so the subprocess formats output for that
/// width. If the parser has a different column count, it interprets that output
/// incorrectly — lines wrap at the wrong boundary and fullscreen apps break.
///
/// For append-style sessions, we only resize parser columns; their parser row
/// count is preserved and deep scrollback uses raw byte replay. Redraw-style
/// agents need parser rows to match the PTY rows because their cursor moves and
/// clears are relative to the visible terminal grid.
pub fn resize_ptys_to_panes(state: &mut AppState) {
    let output_cols = state.output_pane_cols();
    let pinned_cols = state.pinned_pane_cols();
    let rows = state.pane_rows();

    // Copy pinned IDs since we need mutable state access below
    let pinned_ids: Vec<Uuid> = state.pinned_terminal_ids().to_vec();
    let redraw_session_ids: HashSet<Uuid> = state
        .data
        .sessions
        .values()
        .flatten()
        .filter(|session| session.agent_type.is_redraw_style())
        .map(|session| session.id)
        .collect();

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

    // Resize vt100 parsers to match new column widths. For redraw-style agents,
    // rows also need to match the visible PTY height.
    for (session_id, parser) in state.system.output_buffers.iter_mut() {
        let cols = if pinned_ids.contains(session_id) {
            pinned_cols
        } else {
            output_cols
        };

        let (parser_rows, parser_cols) = parser.screen().size();
        let target_rows = if redraw_session_ids.contains(session_id) {
            rows.max(1)
        } else {
            parser_rows
        };
        if parser_cols != cols || parser_rows != target_rows {
            parser.set_size(target_rows, cols);
        }
    }

    // Invalidate all replay caches since column changes affect line wrapping
    state.system.replay_caches.clear();
}
