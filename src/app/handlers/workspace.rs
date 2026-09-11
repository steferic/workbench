use crate::app::{Action, AppState, InputMode, PendingDelete, WorkspaceAction};
use crate::models::Workspace;
use crate::pty::PtyManager;
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc;

use super::save_state;

pub fn handle_workspace_action(
    state: &mut AppState,
    action: Action,
    _pty_manager: &PtyManager,
    action_tx: &mpsc::UnboundedSender<Action>,
    _pty_tx: &mpsc::Sender<Action>,
) -> Result<()> {
    match action {
        Action::InitiateDeleteWorkspace(id, name) => {
            let global = state.data.workspaces.iter().any(|w| w.id == id && w.global);
            if !global {
                state.ui.pending_delete = Some(PendingDelete::Workspace(id, name));
            }
        }
        Action::ConfirmDeleteWorkspace => {
            if let Some(PendingDelete::Workspace(id, _)) = state.ui.pending_delete.take() {
                let mut ids: Vec<_> = state
                    .data
                    .sessions
                    .get(&id)
                    .into_iter()
                    .flatten()
                    .map(|s| s.id)
                    .collect();
                if let Some(ws) = state.get_workspace(id) {
                    ids.extend(
                        ws.parallel_tasks
                            .iter()
                            .flat_map(|task| &task.attempts)
                            .map(|a| a.session_id),
                    );
                }
                crate::app::cleanup::remove_sessions(state, &ids, action_tx);
                state.data.sessions.remove(&id);
                crate::app::comms_tick::retire_workspace(state, id);
                // Drop the per-workspace UI state so we don't accumulate
                // entries for deleted workspaces over the process lifetime.
                state.ws_ui.remove(&id);
                state.data.notepads.remove(&id);
                // Remove the workspace
                if let Some(idx) = state.data.workspaces.iter().position(|w| w.id == id) {
                    state.data.workspaces.remove(idx);
                    if state.ui.selected_workspace_idx >= state.data.workspaces.len()
                        && !state.data.workspaces.is_empty()
                    {
                        state.ui.selected_workspace_idx = state.data.workspaces.len() - 1;
                    }
                    // Load the now-selected workspace's preserved UI state
                    // into live fields. Pass `None` for prev — the previous
                    // workspace was just deleted so there's nothing to snapshot.
                    crate::app::selection::transition_workspace(state, None);
                }
                save_state(state, "failed to save workspace deletion");
            }
        }
        Action::EnterWorkspaceActionMode => {
            state.ui.input_mode = InputMode::SelectWorkspaceAction;
            state.ui.selected_workspace_action = WorkspaceAction::default();
        }
        Action::NextWorkspaceChoice => {
            let actions = WorkspaceAction::all();
            let current_idx = actions
                .iter()
                .position(|a| *a == state.ui.selected_workspace_action)
                .unwrap_or(0);
            if current_idx < actions.len() - 1 {
                state.ui.selected_workspace_action = actions[current_idx + 1];
            }
        }
        Action::PrevWorkspaceChoice => {
            let actions = WorkspaceAction::all();
            let current_idx = actions
                .iter()
                .position(|a| *a == state.ui.selected_workspace_action)
                .unwrap_or(0);
            if current_idx > 0 {
                state.ui.selected_workspace_action = actions[current_idx - 1];
            }
        }
        Action::ConfirmWorkspaceChoice => match state.ui.selected_workspace_action {
            WorkspaceAction::CreateNew => {
                state.ui.workspace_create_mode = true;
                state.ui.input_mode = InputMode::CreateWorkspace;
                state.ui.input_buffer.clear();
                state.ui.file_browser.query.clear();
                state.ui.file_browser.path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                state.refresh_file_browser();
            }
            WorkspaceAction::OpenExisting => {
                state.ui.workspace_create_mode = false;
                state.ui.input_mode = InputMode::CreateWorkspace;
                state.ui.input_buffer.clear();
                state.ui.file_browser.query.clear();
                state.ui.file_browser.path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                state.refresh_file_browser();
            }
        },
        Action::EnterWorkspaceNameMode => {
            state.ui.input_mode = InputMode::EnterWorkspaceName;
            state.ui.input_buffer.clear();
        }
        Action::CreateNewWorkspace(name) => {
            let new_path = state.ui.file_browser.path.join(&name);
            if !new_path.exists() && std::fs::create_dir_all(&new_path).is_ok() {
                let workspace = Workspace::from_path(new_path);
                state.add_workspace(workspace);
                state.ui.input_mode = InputMode::Normal;
                save_state(state, "failed to save workspace creation");
            }
        }
        _ => {}
    }
    Ok(())
}
