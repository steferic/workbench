use crate::app::{AppState, InputMode};
use crate::tui::components::{
    banner, create_session_dialog, create_workspace_dialog, help_popup, output_pane,
    parallel_task_modal, pinned_terminal_pane, session_list, status_bar, todos_pane,
    utilities_pane, workspace_action_dialog, workspace_list, workspace_name_dialog,
};
use crate::tui::effects::{EffectsManager, StartupAreas};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    Frame,
};

pub fn draw(frame: &mut Frame, state: &mut AppState, effects: &mut EffectsManager) {
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
    let left_pct = (state.ui.left_panel_ratio * 100.0) as u16;
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
    let ws_pct = (state.ui.workspace_ratio * 100.0) as u16;
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(ws_pct),
            Constraint::Percentage(100 - ws_pct),
        ])
        .split(left_panel);

    let workspace_area = left_chunks[0];
    let lower_left = left_chunks[1];

    // Split lower left into: sessions | todos | utilities (using dynamic ratios)
    // sessions_ratio controls how much of lower_left goes to sessions
    // todos_ratio controls how the remainder is split between todos and utilities
    let sessions_pct = (state.ui.sessions_ratio * 100.0) as u16;
    let remaining_pct = 100 - sessions_pct;
    let todos_pct = ((state.ui.todos_ratio * remaining_pct as f32) / 100.0 * 100.0) as u16;
    let utilities_pct = remaining_pct - todos_pct;

    let lower_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(sessions_pct),
            Constraint::Percentage(todos_pct),
            Constraint::Percentage(utilities_pct),
        ])
        .split(lower_left);

    let session_area = lower_chunks[0];
    let todos_area = lower_chunks[1];
    let utilities_area = lower_chunks[2];

    // Store areas in state for mouse interaction
    state.ui.workspace_area = Some((workspace_area.x, workspace_area.y, workspace_area.width, workspace_area.height));
    state.ui.session_area = Some((session_area.x, session_area.y, session_area.width, session_area.height));
    state.ui.todos_area = Some((todos_area.x, todos_area.y, todos_area.width, todos_area.height));
    state.ui.utilities_area = Some((utilities_area.x, utilities_area.y, utilities_area.width, utilities_area.height));

    // Render left components
    workspace_list::render(frame, workspace_area, state);
    session_list::render(frame, session_area, state);
    todos_pane::render(frame, todos_area, state);
    utilities_pane::render(frame, utilities_area, state);

    // Render right panel - split if pinned terminals exist and split view is enabled
    if state.should_show_split() {
        // Split right panel: active session | pinned terminals (using dynamic ratio)
        let output_pct = (state.ui.output_split_ratio * 100.0) as u16;
        let right_split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(output_pct),
                Constraint::Percentage(100 - output_pct),
            ])
            .split(right_panel);

        let output_area = right_split[0];
        state.ui.output_pane_area = Some((output_area.x, output_area.y, output_area.width, output_area.height));
        output_pane::render(frame, output_area, state);

        // Render multiple pinned panes stacked vertically
        let pinned_count = state.pinned_count();
        if pinned_count > 0 {
            let pinned_areas = split_pinned_area(right_split[1], state);
            for (idx, area) in pinned_areas.iter().enumerate() {
                if idx < pinned_count {
                    state.ui.pinned_pane_areas[idx] = Some((area.x, area.y, area.width, area.height));
                    pinned_terminal_pane::render_at(frame, *area, state, idx);
                }
            }
        }
    } else {
        // Single pane - full width
        state.ui.output_pane_area = Some((right_panel.x, right_panel.y, right_panel.width, right_panel.height));
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
        InputMode::Help => {
            help_popup::render(frame, state);
        }
        InputMode::SelectWorkspaceAction => {
            workspace_action_dialog::render(frame, state);
        }
        InputMode::CreateWorkspace => {
            create_workspace_dialog::render(frame, state);
        }
        InputMode::EnterWorkspaceName => {
            workspace_name_dialog::render(frame, state);
        }
        InputMode::CreateSession => {
            create_session_dialog::render(frame, state);
        }
        InputMode::SetStartCommand => {
            // Start command input is shown in the status bar
        }
        InputMode::CreateTodo => {
            // Todo input is shown in the status bar
        }
        InputMode::CreateParallelTask => {
            // Will render parallel task modal
            parallel_task_modal::render(frame, state);
        }
        InputMode::Normal => {}
    }

    // Trigger startup animation on first draw
    if !effects.startup_complete() && !effects.has_active_effects() {
        // Collect all pane areas for startup animation
        let pinned_areas = if state.should_show_split() {
            let output_pct = (state.ui.output_split_ratio * 100.0) as u16;
            let right_split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(output_pct),
                    Constraint::Percentage(100 - output_pct),
                ])
                .split(right_panel);
            split_pinned_area(right_split[1], state)
        } else {
            vec![]
        };

        let areas = StartupAreas {
            workspace: workspace_area,
            session: session_area,
            todos: todos_area,
            utilities: utilities_area,
            output: if state.should_show_split() {
                let output_pct = (state.ui.output_split_ratio * 100.0) as u16;
                Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Percentage(output_pct),
                        Constraint::Percentage(100 - output_pct),
                    ])
                    .split(right_panel)[0]
            } else {
                right_panel
            },
            pinned: pinned_areas,
        };
        effects.trigger_startup(&areas);
    }

    // Process active effects
    effects.process(frame);
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
        .iter()
        .cloned()
        .collect()
}
