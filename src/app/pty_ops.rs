use crate::app::AppState;
use std::collections::HashSet;
use uuid::Uuid;

/// Request a PTY-to-pane size sync. The actual resize runs in the main loop
/// AFTER the next draw: pane rects (`ui.output_pane_area` etc.) are computed
/// during render, so resizing inline from an action handler would use the
/// previous layout's dimensions and leave every PTY one resize behind — the
/// classic "view is garbled until I resize the window again" bug.
pub fn request_pty_resize(state: &mut AppState) {
    state.system.pty_resize_pending = true;
}

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
    // Geometry must come from the rects the last draw actually laid out
    // (`ui.output_pane_area`, `ui.pinned_pane_areas`). The ratio math used as
    // a fallback below rounds differently than ratatui's Layout, and a
    // one-column drift mis-wraps every line longer than the pane. Pinned
    // panes are also stacked vertically, so each has its own height — a
    // shared row count lies to the shell's line editor about the viewport.
    let output_size = (state.pane_rows(), state.output_pane_cols());

    // Copy pinned IDs since we need mutable state access below
    let pinned_ids: Vec<Uuid> = state.pinned_terminal_ids().to_vec();
    // A pinned pane with no rect isn't on screen (split view off, or no draw
    // yet) — leave its PTY at its last geometry rather than sizing it for a
    // pane that doesn't exist.
    let pinned_sizes: Vec<Option<(u16, u16)>> = (0..pinned_ids.len())
        .map(|idx| {
            state
                .ui
                .pinned_pane_areas
                .get(idx)
                .copied()
                .flatten()
                // Subtract borders, as output_pane_cols/pane_rows do.
                .map(|(_, _, w, h)| (h.saturating_sub(2), w.saturating_sub(2)))
        })
        .collect();
    let size_for = |session_id: &Uuid| -> Option<(u16, u16)> {
        match pinned_ids.iter().position(|id| id == session_id) {
            Some(idx) => pinned_sizes[idx],
            None => Some(output_size),
        }
    };

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
        let Some((rows, cols)) = size_for(session_id) else {
            continue;
        };

        // Resize the PTY - this updates TIOCGWINSZ which apps query for terminal size
        if let Err(err) = handle.resize(rows.max(1), cols.max(1)) {
            crate::logger::warn(format!("failed to resize PTY {session_id}: {err}"));
        }
    }

    // Resize vt100 parsers to match new column widths. For redraw-style agents,
    // rows also need to match the visible PTY height.
    for (session_id, parser) in state.system.output_buffers.iter_mut() {
        let Some((rows, cols)) = size_for(session_id) else {
            continue;
        };
        let cols = cols.max(1);

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

    // Drop replay caches whose column count no longer matches their pane —
    // rebuilding one replays the whole raw buffer, so a resize that left a
    // pane's width alone must not cost its sessions their caches. (Staleness
    // from new output is handled at use: build_terminal_view checks the
    // cache's generation and cols before trusting it.)
    state.system.replay_caches.retain(|session_id, cache| {
        size_for(session_id).is_none_or(|(_, cols)| cache.cols == cols.max(1))
    });
}

#[cfg(test)]
mod tests {
    use super::resize_ptys_to_panes;
    use crate::app::AppState;
    use crate::models::{AgentType, Session, Workspace};
    use uuid::Uuid;

    fn state_with_pinned_terminal() -> (AppState, Uuid, Uuid) {
        let mut state = AppState::default();
        let mut workspace = Workspace::new("w".into(), std::path::PathBuf::from("/tmp/w"));
        let workspace_id = workspace.id;
        let agent = Session::new(workspace_id, AgentType::Claude, false);
        let agent_id = agent.id;
        let terminal_type = AgentType::Terminal("t".into());
        let pinned = Session::new(workspace_id, terminal_type.clone(), false);
        let pinned_id = pinned.id;
        workspace.pinned_terminal_ids.push(pinned_id);
        state.data.workspaces.push(workspace);
        state.data.sessions.insert(workspace_id, vec![agent, pinned]);
        state
            .system
            .create_session_buffers(agent_id, 24, 80, &AgentType::Claude);
        state
            .system
            .create_session_buffers(pinned_id, 24, 80, &terminal_type);
        (state, agent_id, pinned_id)
    }

    /// The wrap bug this guards: pane widths used to come from ratio math
    /// that rounds differently than the Layout the panes were actually drawn
    /// with, so shells wrapped a column or two past (or short of) the visible
    /// edge. Sizing must follow the rendered rects.
    #[test]
    fn parsers_are_sized_from_rendered_rects_not_ratio_math() {
        let (mut state, agent_id, pinned_id) = state_with_pinned_terminal();

        // Ratio math over this terminal size would give different numbers
        // than the rects below — the rects must win.
        state.system.terminal_size = (200, 60);
        state.ui.output_pane_area = Some((30, 0, 100, 42));
        state.ui.pinned_pane_areas[0] = Some((130, 0, 47, 21));

        let pinned_rows_before = state.system.output_buffers[&pinned_id].screen().size().0;

        resize_ptys_to_panes(&mut state);

        // Redraw-style agent: rows and cols both track its pane (minus borders).
        assert_eq!(
            state.system.output_buffers[&agent_id].screen().size(),
            (40, 98)
        );
        // Append-style terminal: cols track its own pane — not the output
        // pane's, not the ratio estimate — and parser rows are preserved.
        assert_eq!(
            state.system.output_buffers[&pinned_id].screen().size(),
            (pinned_rows_before, 45)
        );
    }

    /// A pinned terminal with no rect isn't on screen (split view off): its
    /// geometry must be left alone, not squeezed into a zero-width pane.
    #[test]
    fn hidden_pinned_terminals_keep_their_geometry() {
        let (mut state, _, pinned_id) = state_with_pinned_terminal();

        state.system.terminal_size = (200, 60);
        state.ui.output_pane_area = Some((30, 0, 100, 42));
        state.ui.pinned_pane_areas[0] = None;

        let size_before = state.system.output_buffers[&pinned_id].screen().size();

        resize_ptys_to_panes(&mut state);

        assert_eq!(
            state.system.output_buffers[&pinned_id].screen().size(),
            size_before
        );
    }
}
