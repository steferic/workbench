use crate::app::{AppState, InputMode};
use crate::tui::components::{
    banner, create_session_dialog, create_workspace_dialog, help_popup, output_pane,
    pinned_terminal_pane, session_list, status_bar, utilities_pane, workspace_list,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    Frame,
};

pub fn draw(frame: &mut Frame, state: &mut AppState) {
    let (banner_area, main_area, status_area) = if state.banner_visible {
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
    let left_pct = (state.left_panel_ratio * 100.0) as u16;
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
    let ws_pct = (state.workspace_ratio * 100.0) as u16;
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(ws_pct),
            Constraint::Percentage(100 - ws_pct),
        ])
        .split(left_panel);

    let workspace_area = left_chunks[0];
    let lower_left = left_chunks[1];

    // Split lower left: session list | utilities pane (using session_ratio)
    let session_pct = (state.session_ratio * 100.0) as u16;
    let lower_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(session_pct),
            Constraint::Percentage(100 - session_pct),
        ])
        .split(lower_left);

    let session_area = lower_chunks[0];
    let utilities_area = lower_chunks[1];

    // Render left components
    workspace_list::render(frame, workspace_area, state);
    session_list::render(frame, session_area, state);
    utilities_pane::render(frame, utilities_area, state);

    // Render right panel - split if pinned terminals exist and split view is enabled
    if state.should_show_split() {
        // Split right panel: active session | pinned terminals (using dynamic ratio)
        let output_pct = (state.output_split_ratio * 100.0) as u16;
        let right_split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(output_pct),
                Constraint::Percentage(100 - output_pct),
            ])
            .split(right_panel);

        output_pane::render(frame, right_split[0], state);

        // Render multiple pinned panes stacked vertically
        let pinned_count = state.pinned_count();
        if pinned_count > 0 {
            let pinned_areas = split_pinned_area(right_split[1], state);
            for (idx, area) in pinned_areas.iter().enumerate() {
                if idx < pinned_count {
                    pinned_terminal_pane::render_at(frame, *area, state, idx);
                }
            }
        }
    } else {
        // Single pane - full width
        output_pane::render(frame, right_panel, state);
    }

    if let Some(banner_rect) = banner_area {
        banner::render(frame, banner_rect, state);
    }
    status_bar::render(frame, status_area, state);

    // Render modal overlays
    match state.input_mode {
        InputMode::Help => {
            help_popup::render(frame, state);
        }
        InputMode::CreateWorkspace => {
            create_workspace_dialog::render(frame, state);
        }
        InputMode::CreateSession => {
            create_session_dialog::render(frame, state);
        }
        InputMode::CreateTerminal => {
            // Terminal name input is shown in the status bar
        }
        InputMode::SetStartCommand => {
            // Start command input is shown in the status bar
        }
        InputMode::Normal => {}
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
        .iter()
        .cloned()
        .collect()
}
