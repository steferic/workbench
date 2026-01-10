use crate::app::{Action, AppState, InputMode, PendingDelete, WorkspaceAction};
use crate::models::Workspace;
use crate::persistence;
use anyhow::Result;
use std::path::PathBuf;

pub fn handle_workspace_action(state: &mut AppState, action: Action) -> Result<()> {
    match action {
        Action::ToggleWorkspaceStatus => {
            if let Some(ws) = state.data.workspaces.get_mut(state.ui.selected_workspace_idx) {
                ws.toggle_status();
                // Auto-save
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
                        if let Some(mut handle) = state.system.pty_handles.remove(&session.id) {
                            let _ = handle.kill();
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
        Action::SelectNextWorkspaceAction => {
            let actions = WorkspaceAction::all();
            let current_idx = actions.iter().position(|a| *a == state.ui.selected_workspace_action).unwrap_or(0);
            if current_idx < actions.len() - 1 {
                state.ui.selected_workspace_action = actions[current_idx + 1];
            }
        }
        Action::SelectPrevWorkspaceAction => {
            let actions = WorkspaceAction::all();
            let current_idx = actions.iter().position(|a| *a == state.ui.selected_workspace_action).unwrap_or(0);
            if current_idx > 0 {
                state.ui.selected_workspace_action = actions[current_idx - 1];
            }
        }
        Action::ConfirmWorkspaceAction => {
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
            if !new_path.exists() {
                if let Ok(_) = std::fs::create_dir_all(&new_path) {
                    let workspace = Workspace::from_path(new_path);
                    state.add_workspace(workspace);
                    state.ui.input_mode = InputMode::Normal;
                    let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
                }
            }
        }
        _ => {}
    }
    Ok(())
}
