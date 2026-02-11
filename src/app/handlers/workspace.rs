use crate::app::{Action, AppState, InputMode, PendingDelete, PendingSessionStart, WorkspaceAction};
use crate::app::handlers::session::terminate_session_handle;
use crate::models::{SessionStatus, Workspace, WorkspaceStatus};
use crate::persistence;
use crate::pty::PtyManager;
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc;

pub fn handle_workspace_action(
    state: &mut AppState,
    action: Action,
    _pty_manager: &PtyManager,
    _action_tx: &mpsc::UnboundedSender<Action>,
    _pty_tx: &mpsc::Sender<Action>,
) -> Result<()> {
    match action {
        Action::ToggleWorkspaceStatus => {
            if let Some(ws) = state.data.workspaces.get(state.ui.selected_workspace_idx) {
                let workspace_id = ws.id;
                let workspace_path = ws.path.clone();
                let old_status = ws.status;

                // Toggle the status
                if let Some(ws) = state.data.workspaces.get_mut(state.ui.selected_workspace_idx) {
                    ws.toggle_status();
                }

                match old_status {
                    WorkspaceStatus::Working => {
                        // Pausing: tear down all PTYs, buffers, and activity tracking
                        let session_ids: Vec<_> = state.data.sessions
                            .get(&workspace_id)
                            .map(|sessions| {
                                sessions.iter()
                                    .filter(|s| s.status == SessionStatus::Running)
                                    .map(|s| (s.id, s.agent_type.is_terminal()))
                                    .collect()
                            })
                            .unwrap_or_default();

                        for (session_id, is_terminal) in &session_ids {
                            // Terminate PTY handle
                            if let Some(handle) = state.system.pty_handles.remove(session_id) {
                                terminate_session_handle(handle, *is_terminal);
                            }
                            // Drop output buffer (the big memory win)
                            state.system.output_buffers.remove(session_id);
                            // Remove activity tracking
                            state.data.last_activity.remove(session_id);
                            state.data.last_send_input.remove(session_id);
                        }

                        // Drain from idle queue
                        let ids: Vec<_> = session_ids.iter().map(|(id, _)| *id).collect();
                        state.data.idle_queue.retain(|id| !ids.contains(id));

                        // Mark running sessions as Stopped
                        if let Some(sessions) = state.data.sessions.get_mut(&workspace_id) {
                            for session in sessions.iter_mut() {
                                if session.status == SessionStatus::Running {
                                    session.status = SessionStatus::Stopped;
                                }
                            }
                        }

                        // Clear active session if it belonged to this workspace
                        if let Some(active_id) = state.ui.active_session_id {
                            if ids.contains(&active_id) {
                                state.ui.active_session_id = None;
                            }
                        }
                    }
                    WorkspaceStatus::Paused => {
                        // Resuming: queue stopped sessions for staggered startup
                        let stopped_sessions: Vec<PendingSessionStart> = state.data.sessions
                            .get(&workspace_id)
                            .map(|sessions| {
                                sessions.iter()
                                    .filter(|s| matches!(s.status, SessionStatus::Stopped | SessionStatus::Errored))
                                    .map(|s| PendingSessionStart {
                                        session_id: s.id,
                                        workspace_id,
                                        workspace_path: workspace_path.clone(),
                                        agent_type: s.agent_type.clone(),
                                        start_command: s.start_command.clone(),
                                        dangerously_skip_permissions: s.dangerously_skip_permissions,
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();

                        for pending in stopped_sessions {
                            state.system.startup_queue.push_back(pending);
                        }
                    }
                }

                let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
            }
        }
        Action::InitiateDeleteWorkspace(id, name) => {
            state.ui.pending_delete = Some(PendingDelete::Workspace(id, name));
        }
        Action::ConfirmDeleteWorkspace => {
            if let Some(PendingDelete::Workspace(id, _)) = state.ui.pending_delete.take() {
                // Remove all sessions and PTYs for this workspace
                if let Some(sessions) = state.data.sessions.remove(&id) {
                    for session in sessions {
                        if let Some(handle) = state.system.pty_handles.remove(&session.id) {
                            terminate_session_handle(handle, session.agent_type.is_terminal());
                        }
                        state.system.output_buffers.remove(&session.id);
                    }
                }
                // Remove the workspace
                if let Some(idx) = state.data.workspaces.iter().position(|w| w.id == id) {
                    state.data.workspaces.remove(idx);
                    if state.ui.selected_workspace_idx >= state.data.workspaces.len() && !state.data.workspaces.is_empty() {
                        state.ui.selected_workspace_idx = state.data.workspaces.len() - 1;
                    }
                    state.ui.selected_session_idx = 0;
                }
                let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
            }
        }
        Action::EnterWorkspaceActionMode => {
            state.ui.input_mode = InputMode::SelectWorkspaceAction;
            state.ui.selected_workspace_action = WorkspaceAction::default();
        }
        Action::NextWorkspaceChoice => {
            let actions = WorkspaceAction::all();
            let current_idx = actions.iter().position(|a| *a == state.ui.selected_workspace_action).unwrap_or(0);
            if current_idx < actions.len() - 1 {
                state.ui.selected_workspace_action = actions[current_idx + 1];
            }
        }
        Action::PrevWorkspaceChoice => {
            let actions = WorkspaceAction::all();
            let current_idx = actions.iter().position(|a| *a == state.ui.selected_workspace_action).unwrap_or(0);
            if current_idx > 0 {
                state.ui.selected_workspace_action = actions[current_idx - 1];
            }
        }
        Action::ConfirmWorkspaceChoice => {
            match state.ui.selected_workspace_action {
                WorkspaceAction::CreateNew => {
                    state.ui.workspace_create_mode = true;
                    state.ui.input_mode = InputMode::CreateWorkspace;
                    state.ui.input_buffer.clear();
                    state.ui.file_browser_query.clear();
                    state.ui.file_browser_path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                    state.refresh_file_browser();
                }
                WorkspaceAction::OpenExisting => {
                    state.ui.workspace_create_mode = false;
                    state.ui.input_mode = InputMode::CreateWorkspace;
                    state.ui.input_buffer.clear();
                    state.ui.file_browser_query.clear();
                    state.ui.file_browser_path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                    state.refresh_file_browser();
                }
            }
        }
        Action::EnterWorkspaceNameMode => {
            state.ui.input_mode = InputMode::EnterWorkspaceName;
            state.ui.input_buffer.clear();
        }
        Action::CreateNewWorkspace(name) => {
            let new_path = state.ui.file_browser_path.join(&name);
            if !new_path.exists() && std::fs::create_dir_all(&new_path).is_ok() {
                let workspace = Workspace::from_path(new_path);
                state.add_workspace(workspace);
                state.ui.input_mode = InputMode::Normal;
                let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
            }
        }
        _ => {}
    }
    Ok(())
}
