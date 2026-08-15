use crate::app::selection::transition_workspace;
use crate::app::session_start::start_workspace_sessions;
use crate::app::{Action, AppState, TextSelection};
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

pub(crate) fn cycle_next_workspace(state: &mut AppState) {
    cycle_workspace(state, CycleDirection::Next);
}

pub(crate) fn cycle_prev_workspace(state: &mut AppState) {
    cycle_workspace(state, CycleDirection::Prev);
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
    state.data.workspaces.get(row).map(|_| row)
}

enum CycleDirection {
    Next,
    Prev,
}

fn cycle_workspace(state: &mut AppState, direction: CycleDirection) {
    let workspace_count = state.data.workspaces.len();
    if workspace_count == 0 {
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

    let current = state.ui.selected_workspace_idx;
    let next_idx = match (direction, current < workspace_count) {
        (CycleDirection::Next, true) => (current + 1) % workspace_count,
        (CycleDirection::Next, false) => 0,
        (CycleDirection::Prev, true) => (current + workspace_count - 1) % workspace_count,
        (CycleDirection::Prev, false) => workspace_count - 1,
    };

    if next_idx != state.ui.selected_workspace_idx {
        state.ui.selected_workspace_idx = next_idx;
        transition_workspace_after_index_change(state, prev_ws_id);
    }
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
    use super::{cycle_next_workspace, cycle_prev_workspace, workspace_index_at_position};
    use crate::app::AppState;
    use crate::models::Workspace;
    use std::path::PathBuf;

    fn workspace(name: &str) -> Workspace {
        Workspace::new(name.to_string(), PathBuf::from(format!("/tmp/{name}")))
    }

    #[test]
    fn workspace_hit_testing_maps_each_row_directly() {
        let mut state = AppState::default();
        state.data.workspaces = vec![
            workspace("alpha"),
            workspace("beta"),
            workspace("gamma"),
        ];
        state.ui.workspace_area = Some((0, 0, 20, 8));

        assert_eq!(workspace_index_at_position(&state, 2, 1), Some(0));
        assert_eq!(workspace_index_at_position(&state, 2, 2), Some(1));
        assert_eq!(workspace_index_at_position(&state, 2, 3), Some(2));
        assert_eq!(workspace_index_at_position(&state, 2, 4), None);
    }

    #[test]
    fn cycle_next_workspace_visits_every_project() {
        let mut state = AppState::default();
        state.data.workspaces = vec![workspace("alpha"), workspace("beta"), workspace("gamma")];

        cycle_next_workspace(&mut state);

        assert_eq!(state.ui.selected_workspace_idx, 1);
    }

    #[test]
    fn cycle_prev_workspace_wraps_to_last_project() {
        let mut state = AppState::default();
        state.data.workspaces = vec![workspace("alpha"), workspace("beta"), workspace("gamma")];

        cycle_prev_workspace(&mut state);

        assert_eq!(state.ui.selected_workspace_idx, 2);
    }
}
