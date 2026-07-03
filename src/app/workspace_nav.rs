use crate::app::selection::transition_workspace;
use crate::app::session_start::start_workspace_sessions;
use crate::app::{Action, AppState, TextSelection};
use crate::models::WorkspaceStatus;
use crate::pty::PtyManager;
use tokio::sync::mpsc;
use uuid::Uuid;

fn is_in_area(x: u16, y: u16, area: (u16, u16, u16, u16)) -> bool {
    let (ax, ay, aw, ah) = area;
    x >= ax && x < ax + aw && y >= ay && y < ay + ah
}

pub(crate) fn move_workspace_selection(
    state: &mut AppState,
    move_prev: bool,
    pty_manager: &PtyManager,
    pty_tx: &mpsc::Sender<Action>,
) {
    let prev_idx = state.ui.selected_workspace_idx;
    let prev_ws_id = state.data.workspaces.get(prev_idx).map(|w| w.id);

    let active = state.active_session_id();
    if let Some(current_ws) = state.data.workspaces.get_mut(prev_idx) {
        current_ws.last_active_session_id = active;
    }

    if move_prev {
        state.select_prev_workspace();
    } else {
        state.select_next_workspace();
    }

    if state.ui.selected_workspace_idx != prev_idx {
        transition_workspace_after_index_change(state, prev_ws_id);
        start_workspace_sessions(state, pty_manager, pty_tx);
    }
}

pub(crate) fn set_selected_workspace(
    state: &mut AppState,
    workspace_idx: usize,
    pty_manager: &PtyManager,
    pty_tx: &mpsc::Sender<Action>,
) {
    if workspace_idx >= state.data.workspaces.len()
        || workspace_idx == state.ui.selected_workspace_idx
    {
        return;
    }

    let prev_ws_id = state
        .data
        .workspaces
        .get(state.ui.selected_workspace_idx)
        .map(|w| w.id);

    let active = state.active_session_id();
    if let Some(current_ws) = state
        .data
        .workspaces
        .get_mut(state.ui.selected_workspace_idx)
    {
        current_ws.last_active_session_id = active;
    }

    state.ui.selected_workspace_idx = workspace_idx;
    transition_workspace_after_index_change(state, prev_ws_id);
    start_workspace_sessions(state, pty_manager, pty_tx);
}

pub(crate) fn cycle_next_working_workspace(state: &mut AppState) {
    cycle_working_workspace(state, CycleDirection::Next);
}

pub(crate) fn cycle_prev_working_workspace(state: &mut AppState) {
    cycle_working_workspace(state, CycleDirection::Prev);
}

pub(crate) fn workspace_index_at_position(state: &AppState, x: u16, y: u16) -> Option<usize> {
    let (area_x, area_y, area_w, area_h) = state.ui.workspace_area?;
    if !is_in_area(x, y, (area_x, area_y, area_w, area_h)) || area_w <= 2 || area_h <= 3 {
        return None;
    }

    let inner_x = area_x.saturating_add(1);
    let inner_y = area_y.saturating_add(1);
    let inner_w = area_w.saturating_sub(2);
    let inner_h = area_h.saturating_sub(2);
    let list_h = inner_h.saturating_sub(1);

    if list_h == 0 || x < inner_x || x >= inner_x + inner_w || y < inner_y || y >= inner_y + list_h
    {
        return None;
    }

    let row = (y - inner_y) as usize;
    let working_indices = workspace_indices_with_status(state, WorkspaceStatus::Working);
    let paused_indices = workspace_indices_with_status(state, WorkspaceStatus::Paused);

    let mut visual_row = 0usize;

    if !working_indices.is_empty() || paused_indices.is_empty() {
        if row == visual_row {
            return None;
        }
        visual_row += 1;
    }

    for idx in working_indices {
        if row == visual_row {
            return Some(idx);
        }
        visual_row += 1;
    }

    if !paused_indices.is_empty() {
        if row == visual_row {
            return None;
        }
        visual_row += 1;
    }

    for idx in paused_indices {
        if row == visual_row {
            return Some(idx);
        }
        visual_row += 1;
    }

    None
}

enum CycleDirection {
    Next,
    Prev,
}

fn cycle_working_workspace(state: &mut AppState, direction: CycleDirection) {
    let working_indices = workspace_indices_with_status(state, WorkspaceStatus::Working);

    if working_indices.is_empty() {
        return;
    }

    let prev_ws_id = state
        .data
        .workspaces
        .get(state.ui.selected_workspace_idx)
        .map(|w| w.id);

    let active = state.active_session_id();
    if let Some(current_ws) = state
        .data
        .workspaces
        .get_mut(state.ui.selected_workspace_idx)
    {
        current_ws.last_active_session_id = active;
    }

    let current_pos = working_indices
        .iter()
        .position(|&idx| idx == state.ui.selected_workspace_idx);

    let next_idx = match (direction, current_pos) {
        (CycleDirection::Next, Some(pos)) => working_indices[(pos + 1) % working_indices.len()],
        (CycleDirection::Next, None) => working_indices[0],
        (CycleDirection::Prev, Some(pos)) => {
            working_indices[(pos + working_indices.len() - 1) % working_indices.len()]
        }
        (CycleDirection::Prev, None) => *working_indices.last().unwrap(),
    };

    if next_idx != state.ui.selected_workspace_idx {
        state.ui.selected_workspace_idx = next_idx;
        transition_workspace_after_index_change(state, prev_ws_id);
    }
}

fn workspace_indices_with_status(state: &AppState, status: WorkspaceStatus) -> Vec<usize> {
    state
        .data
        .workspaces
        .iter()
        .enumerate()
        .filter(|(_, ws)| ws.status == status)
        .map(|(idx, _)| idx)
        .collect()
}

fn transition_workspace_after_index_change(state: &mut AppState, prev_ws_id: Option<uuid::Uuid>) {
    transition_workspace(state, prev_ws_id);
    restore_workspace_session(state);
}

/// Validate/restore the active session for the currently selected workspace.
/// The workspace's own `ws_ui` already remembers its active session; this
/// re-resolves it against the live session list (it may have been deleted)
/// and falls back to the first agent session if it's gone.
fn restore_workspace_session(state: &mut AppState) {
    let next_idx = state.ui.selected_workspace_idx;
    let Some(ws) = state.data.workspaces.get(next_idx) else {
        return;
    };
    let ws_id = ws.id;
    // Prefer the workspace's preserved active session (seeded from the
    // persisted `last_active_session_id` on first access).
    let candidate = state.active_session_id().or(ws.last_active_session_id);

    // Resolve to owned values before the ws_ui writes below take &mut state.
    let sessions = state.data.sessions.get(&ws_id);
    let resolved: Option<(usize, Uuid)> = sessions.zip(candidate).and_then(|(sessions, id)| {
        sessions.iter().position(|s| s.id == id).map(|idx| (idx, id))
    });
    let fallback: Option<(usize, Uuid)> = sessions.and_then(|sessions| {
        sessions
            .iter()
            .enumerate()
            .find(|(_, s)| !s.agent_type.is_terminal())
            .or_else(|| sessions.iter().enumerate().next())
            .map(|(idx, s)| (idx, s.id))
    });

    if let Some((idx, session_id)) = resolved {
        state.set_selected_session_idx(idx);
        state.set_active_session_id(Some(session_id));
        return;
    }

    match fallback {
        Some((idx, session_id)) => {
            state.set_selected_session_idx(idx);
            state.set_active_session_id(Some(session_id));
        }
        None => {
            state.set_selected_session_idx(0);
            state.set_active_session_id(None);
        }
    }
    state.set_output_scroll_offset(0);
    state.set_text_selection(TextSelection::default());
}

#[cfg(test)]
mod tests {
    use super::{
        cycle_next_working_workspace, cycle_prev_working_workspace, workspace_index_at_position,
    };
    use crate::app::AppState;
    use crate::models::{Workspace, WorkspaceStatus};
    use std::path::PathBuf;

    fn workspace(name: &str, status: WorkspaceStatus) -> Workspace {
        let mut workspace = Workspace::new(name.to_string(), PathBuf::from(format!("/tmp/{name}")));
        workspace.status = status;
        workspace
    }

    #[test]
    fn workspace_hit_testing_skips_section_headers() {
        let mut state = AppState::default();
        state.data.workspaces = vec![
            workspace("alpha", WorkspaceStatus::Working),
            workspace("beta", WorkspaceStatus::Paused),
            workspace("gamma", WorkspaceStatus::Paused),
        ];
        state.ui.workspace_area = Some((0, 0, 20, 8));

        assert_eq!(workspace_index_at_position(&state, 2, 1), None);
        assert_eq!(workspace_index_at_position(&state, 2, 2), Some(0));
        assert_eq!(workspace_index_at_position(&state, 2, 3), None);
        assert_eq!(workspace_index_at_position(&state, 2, 4), Some(1));
        assert_eq!(workspace_index_at_position(&state, 2, 5), Some(2));
    }

    #[test]
    fn cycle_next_working_workspace_skips_paused() {
        let mut state = AppState::default();
        state.data.workspaces = vec![
            workspace("alpha", WorkspaceStatus::Working),
            workspace("beta", WorkspaceStatus::Paused),
            workspace("gamma", WorkspaceStatus::Working),
        ];

        cycle_next_working_workspace(&mut state);

        assert_eq!(state.ui.selected_workspace_idx, 2);
    }

    #[test]
    fn cycle_prev_working_workspace_wraps_to_last_working_workspace() {
        let mut state = AppState::default();
        state.data.workspaces = vec![
            workspace("alpha", WorkspaceStatus::Working),
            workspace("beta", WorkspaceStatus::Paused),
            workspace("gamma", WorkspaceStatus::Working),
        ];

        cycle_prev_working_workspace(&mut state);

        assert_eq!(state.ui.selected_workspace_idx, 2);
    }
}
