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
/// Every parser uses its actual PTY dimensions. Append-style normal buffers
/// reflow stored cells on resize; redraw-style agents retain viewport semantics.
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

    // Keep the parser and PTY geometry identical for cursor-addressed output.
    for (session_id, parser) in state.system.output_buffers.iter_mut() {
        let Some((rows, cols)) = size_for(session_id) else {
            continue;
        };
        let cols = cols.max(1);

        let (parser_rows, parser_cols) = parser.screen().size();
        let target_rows = rows.max(1);
        if parser_cols != cols || parser_rows != target_rows {
            if redraw_session_ids.contains(session_id) {
                parser.set_size(target_rows, cols);
            } else {
                parser.resize_reflow(target_rows, cols);
                state.system.native_history_dirty.insert(*session_id);
            }
        }
    }
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
        state
            .data
            .sessions
            .insert(workspace_id, vec![agent, pinned]);
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

        resize_ptys_to_panes(&mut state);

        // Redraw-style agent: rows and cols both track its pane (minus borders).
        assert_eq!(
            state.system.output_buffers[&agent_id].screen().size(),
            (40, 98)
        );
        // The terminal uses its own real pane dimensions too.
        assert_eq!(
            state.system.output_buffers[&pinned_id].screen().size(),
            (19, 45)
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

    #[test]
    fn desk_and_window_resizes_keep_agents_sized_for_their_actual_panes() {
        use ratatui::{backend::TestBackend, Terminal};
        let (mut state, agent_id, pinned_id) = state_with_pinned_terminal();
        state.ui.layout.left_panel_ratio = 0.27;
        state.ui.layout.output_split_ratio = 0.63;
        state.set_active_session_id(Some(agent_id));
        let (tx, _) = tokio::sync::mpsc::unbounded_channel();

        for (width, height) in [(121, 39), (81, 25), (203, 61)] {
            state.ui.desk_open = false;
            state.system.terminal_size = (width, height);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| crate::tui::ui::draw(frame, &mut state))
                .unwrap();
            let expected = state.ui.output_pane_area.unwrap();
            let size = (expected.3 - 2, expected.2 - 2);
            resize_ptys_to_panes(&mut state);
            assert_eq!(state.system.output_buffers[&agent_id].screen().size(), size);

            state.ui.desk_open = true;
            terminal
                .draw(|frame| crate::tui::ui::draw(frame, &mut state))
                .unwrap();
            assert!(state.ui.output_pane_area.unwrap().2 > expected.2);
            assert_eq!((state.pane_rows(), state.output_pane_cols()), size);
            // This is the spawn path's size source while the Desk is open.
            let new_agent = Uuid::new_v4();
            state.system.create_session_buffers(
                new_agent,
                state.pane_rows(),
                state.output_pane_cols(),
                &AgentType::Claude,
            );
            assert_eq!(
                state.system.output_buffers[&new_agent].screen().size(),
                size
            );
            state.system.remove_session_buffers(&new_agent);

            terminal.backend_mut().resize(width + 7, height + 3);
            state.system.terminal_size = (width + 7, height + 3);
            terminal
                .draw(|frame| crate::tui::ui::draw(frame, &mut state))
                .unwrap();
            resize_ptys_to_panes(&mut state);
            let hidden_agent_size = state.system.output_buffers[&agent_id].screen().size();

            state.system.pty_resize_pending = false;
            crate::app::handlers::tasks::handle_task_action(
                &mut state,
                crate::app::Action::ToggleDesk,
                &tx,
            )
            .unwrap();
            assert!(state.system.pty_resize_pending);
            terminal
                .draw(|frame| crate::tui::ui::draw(frame, &mut state))
                .unwrap();
            resize_ptys_to_panes(&mut state);
            let shown = state.ui.output_pane_area.unwrap();
            assert_eq!(hidden_agent_size, (shown.3 - 2, shown.2 - 2));
            let pinned = state.ui.pinned_pane_areas[0].unwrap();
            assert_eq!(
                state.system.output_buffers[&pinned_id].screen().size().1,
                pinned.2 - 2
            );
        }
    }
}
