use crate::app::{AppState, InputMode};
use crate::tui::components::{
    banner, command_palette, config_window, create_session_dialog, create_workspace_dialog,
    debug_overlay, decision_detail, desk_pane, jobs_window, merge_confirm_modal, output_pane,
    parallel_merge_confirm_modal, parallel_task_modal, pinned_terminal_pane, session_list,
    status_bar, utilities_pane, workspace_action_dialog, workspace_list, workspace_name_dialog,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    Frame,
};

#[cfg(test)]
mod layout_tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn the_left_column_has_no_objectives_pane_or_empty_gap() {
        for (width, height) in [(120, 40), (80, 24)] {
            let mut state = AppState::default();
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &mut state)).unwrap();
            let workspace = state.ui.workspace_area.unwrap();
            let sessions = state.ui.session_area.unwrap();
            let utilities = state.ui.utilities_area.unwrap();
            assert_eq!(workspace.1 + workspace.3, sessions.1);
            assert_eq!(sessions.1 + sessions.3, utilities.1);
            assert_eq!(utilities.1 + utilities.3, height - 1);
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(!text.contains("OBJECTIVES"));
        }
    }
}

pub fn draw(frame: &mut Frame, state: &mut AppState) {
    state.ui.link_hits.clear();
    state.ui.click_hits.clear();
    // Activate the chosen theme for this frame and fill the background so light
    // mode doesn't show through to the terminal's (dark) default.
    crate::theme::set_current(state.ui.theme_mode);
    let theme = crate::theme::current();
    let full_area = frame.area();
    frame.buffer_mut().set_style(
        full_area,
        ratatui::style::Style::default().bg(theme.bg).fg(theme.fg),
    );

    let (banner_area, main_area, status_area) = if state.ui.banner_visible {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Banner
                Constraint::Min(3),    // Main content
                Constraint::Length(1), // Status bar
            ])
            .split(frame.area());
        (Some(chunks[0]), chunks[1], chunks[2])
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),    // Main content
                Constraint::Length(1), // Status bar
            ])
            .split(frame.area());
        (None, chunks[0], chunks[1])
    };

    // Split main area: left panel | right panel (using dynamic ratios)
    let left_pct = (state.ui.layout.left_panel_ratio * 100.0) as u16;
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(left_pct),
            Constraint::Percentage(100 - left_pct),
        ])
        .split(main_area);

    let left_panel = horizontal[0];
    let right_panel = horizontal[1];

    // Split left panel: workspace list | sessions + utilities (using workspace_ratio)
    let ws_pct = (state.ui.layout.workspace_ratio * 100.0) as u16;
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(ws_pct),
            Constraint::Percentage(100 - ws_pct),
        ])
        .split(left_panel);

    let workspace_area = left_chunks[0];
    let lower_left = left_chunks[1];

    // Sessions receive the space formerly occupied by Objectives.
    let sessions_pct = (state.ui.layout.sessions_ratio * 100.0) as u16;
    let lower_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(sessions_pct),
            Constraint::Percentage(100 - sessions_pct),
        ])
        .split(lower_left);
    let session_area = lower_chunks[0];
    let utilities_area = lower_chunks[1];

    // Store areas in state for mouse interaction
    state.ui.workspace_area = Some((
        workspace_area.x,
        workspace_area.y,
        workspace_area.width,
        workspace_area.height,
    ));
    state.ui.session_area = Some((
        session_area.x,
        session_area.y,
        session_area.width,
        session_area.height,
    ));
    state.ui.utilities_area = Some((
        utilities_area.x,
        utilities_area.y,
        utilities_area.width,
        utilities_area.height,
    ));

    // Render left components
    workspace_list::render(frame, workspace_area, state);
    session_list::render(frame, session_area, state);
    utilities_pane::render(frame, utilities_area, state);

    // Render right panel: the desk when it is open, otherwise the active
    // session (split with pinned terminals when there are any).
    // Compute the terminal's layout even under the Desk: newly started agents
    // and window resizes must use the geometry they will actually be shown in.
    let right_split = state.should_show_split().then(|| {
        let output_pct = (state.ui.layout.output_split_ratio * 100.0) as u16;
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(output_pct),
                Constraint::Percentage(100 - output_pct),
            ])
            .split(right_panel)
    });
    let output_area = right_split.as_ref().map_or(right_panel, |split| split[0]);
    state.ui.terminal_output_area = Some((
        output_area.x,
        output_area.y,
        output_area.width,
        output_area.height,
    ));
    if state.ui.desk_open {
        // Mouse interaction belongs to the whole Desk. Terminal sizing uses
        // terminal_output_area above; hidden pinned terminals retain their size.
        state.ui.output_pane_area = Some((
            right_panel.x,
            right_panel.y,
            right_panel.width,
            right_panel.height,
        ));
        for area in state.ui.pinned_pane_areas.iter_mut() {
            *area = None;
        }
        desk_pane::render(frame, right_panel, state);
    } else if let Some(right_split) = right_split {
        state.ui.output_pane_area = Some((
            output_area.x,
            output_area.y,
            output_area.width,
            output_area.height,
        ));
        output_pane::render(frame, output_area, state);

        // Render multiple pinned panes stacked vertically
        let pinned_count = state.pinned_count();
        if pinned_count > 0 {
            let pinned_areas = split_pinned_area(right_split[1], state);
            for (idx, area) in pinned_areas.iter().enumerate() {
                if idx < pinned_count {
                    state.ui.pinned_pane_areas[idx] =
                        Some((area.x, area.y, area.width, area.height));
                    pinned_terminal_pane::render_at(frame, *area, state, idx);
                }
            }
        }
    } else {
        // Single pane - full width
        state.ui.output_pane_area = Some((
            right_panel.x,
            right_panel.y,
            right_panel.width,
            right_panel.height,
        ));
        // Clear pinned areas if not in split view
        for area in state.ui.pinned_pane_areas.iter_mut() {
            *area = None;
        }
        output_pane::render(frame, right_panel, state);
    }

    if let Some(banner_rect) = banner_area {
        banner::render(frame, banner_rect, state);
    }
    status_bar::render(frame, status_area, state);

    // Render modal overlays
    match state.ui.input_mode {
        InputMode::SelectWorkspaceAction => {
            workspace_action_dialog::render(frame, state);
        }
        InputMode::CreateWorkspace => {
            create_workspace_dialog::render(frame, state);
        }
        InputMode::EnterWorkspaceName => {
            workspace_name_dialog::render(frame, state);
        }
        _ if state.ui.detail.is_some() => {
            decision_detail::render_detail(frame, state);
        }
        InputMode::CreateSession | InputMode::CreateManager | InputMode::AssignAgent => {
            create_session_dialog::render(frame, state);
        }
        InputMode::SetStartCommand => {
            // Start command input is shown in the status bar
        }
        InputMode::ComposeTaskMessage => {
            // The composed message is shown in the status bar
        }
        InputMode::CreateParallelTask => {
            // Will render parallel task modal
            parallel_task_modal::render(frame, state);
        }
        InputMode::ConfirmMergeWorktree => {
            merge_confirm_modal::render(frame, state);
        }
        InputMode::ConfirmParallelMerge => {
            parallel_merge_confirm_modal::render(frame, state);
        }
        InputMode::CommandPalette => {
            command_palette::render(frame, state);
        }
        InputMode::ConfigWindow => {
            config_window::render(frame, state);
        }
        InputMode::JobsWindow => {
            // Drawn below, after the frame's hits are cleared: its own rows
            // and tabs are the only things clickable while it is open.
        }
        InputMode::Normal => {}
    }

    if state.ui.input_mode == InputMode::Normal
        && state.ui.detail.is_none()
        && !state.ui.pending_quit
        && state.ui.pending_delete.is_none()
        && state.ui.servers.dialog.is_none()
    {
        crate::media::render(frame, state);
    }
    if state.ui.media_preview.is_some()
        || state.ui.input_mode != InputMode::Normal
        || state.ui.detail.is_some()
        || state.ui.pending_quit
        || state.ui.pending_delete.is_some()
        || state.ui.servers.dialog.is_some()
    {
        state.ui.link_hits.clear();
        state.ui.click_hits.clear();
    }
    if state.ui.input_mode == InputMode::JobsWindow
        && !state.ui.pending_quit
        && state.ui.pending_delete.is_none()
    {
        jobs_window::render(frame, state);
    }
    if state.ui.servers.dialog.is_some()
        && !state.ui.pending_quit
        && state.ui.pending_delete.is_none()
    {
        crate::tui::components::servers_pane::dialog(frame, state);
    }

    // Toast notifications are intentionally suppressed — the in-app toast
    // surface didn't earn its keep. State machinery and helpers remain so
    // callers don't need to change; they're just not rendered.

    // Debug overlay (F11)
    if state.ui.show_debug_overlay {
        debug_overlay::render(frame, state);
    }
}

/// Split the pinned terminal area into multiple vertically stacked panes
fn split_pinned_area(area: Rect, state: &AppState) -> Vec<Rect> {
    let count = state.pinned_count();
    if count == 0 {
        return vec![];
    }

    // Get normalized ratios for the current pane count
    let ratios = state.normalized_pinned_ratios();

    // Build constraints based on ratios
    let constraints: Vec<Constraint> = ratios
        .iter()
        .map(|r| Constraint::Percentage((r * 100.0) as u16))
        .collect();

    Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area)
        .to_vec()
}

#[cfg(test)]
mod hyperlink_layout_tests {
    use super::*;
    use crate::models::{AgentType, Session, Workspace};
    use ratatui::{backend::TestBackend, Terminal};
    #[test]
    fn link_hits_follow_rendered_cells_and_clear_on_session_or_modal_changes() {
        let mut state = AppState::default();
        state.ui.banner_visible = false;
        let workspace = Workspace::new("test".into(), "/tmp".into());
        let a = Session::new(workspace.id, AgentType::Claude, false);
        let b = Session::new(workspace.id, AgentType::Claude, false);
        let (a_id, b_id) = (a.id, b.id);
        state.add_workspace(workspace);
        state.add_session(a);
        state.add_session(b);
        let mut parser = vt100::Parser::new(20, 80, 0);
        parser.process(b"\x1b]8;;https://example.com\x07reference\x1b]8;;\x07");
        state.system.output_buffers.insert(a_id, parser);
        state
            .system
            .output_buffers
            .insert(b_id, vt100::Parser::new(20, 80, 0));
        state.set_active_session_id(Some(a_id));
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|f| draw(f, &mut state)).unwrap();
        let hit = state.ui.link_hits.first().unwrap();
        assert_eq!(hit.target, "https://example.com/");
        assert_eq!(
            terminal.backend().buffer()[(hit.area.x, hit.area.y)].symbol(),
            "r"
        );
        state.set_active_session_id(Some(b_id));
        terminal.draw(|f| draw(f, &mut state)).unwrap();
        assert!(state.ui.link_hits.is_empty());
        state.set_active_session_id(Some(a_id));
        state.ui.input_mode = InputMode::CommandPalette;
        terminal.draw(|f| draw(f, &mut state)).unwrap();
        assert!(state.ui.link_hits.is_empty());
    }
}
