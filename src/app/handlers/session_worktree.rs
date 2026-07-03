use crate::app::{Action, AppState, FocusPanel, InputMode, Toast, ToastLevel, WorktreeMergeOutcome};
use crate::git;
use crate::models::{AgentType, Session, SessionStatus};
use crate::pty::{PtyManager, SessionSpawnConfig};
use std::path::Path;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::{report_background_error, save_state};

fn show_toast(state: &mut AppState, msg: impl Into<String>, level: ToastLevel) {
    let duration = match level {
        ToastLevel::Error => Duration::from_secs(5),
        _ => Duration::from_secs(3),
    };
    state
        .ui
        .toasts
        .push_back(Toast::new(msg.into(), level, duration));
    while state.ui.toasts.len() > 5 {
        state.ui.toasts.pop_front();
    }
}

// The merge flow shells out to git (status/commit/merge/worktree remove),
// which can block for seconds on large repos, so every git call runs on a
// blocking thread. The flow is: MergeSessionWorktree kicks off a status
// check → SessionWorktreeMergeChecked decides (confirm modal / blocked /
// merge) → SessionWorktreeMergeFinished reports the outcome.

pub(super) fn handle_merge_session_worktree(
    state: &mut AppState,
    action_tx: &mpsc::UnboundedSender<Action>,
    session_id: Uuid,
) {
    let Some((workspace_path, worktree_path, _branch_name)) =
        merge_info_for_session(state, session_id)
    else {
        return;
    };

    let tx = action_tx.clone();
    tokio::task::spawn_blocking(move || {
        let has_changes = git::worktree_has_changes(&worktree_path);
        let workspace_clean = git::is_clean(&workspace_path).unwrap_or(false);
        if let Err(err) = tx.send(Action::SessionWorktreeMergeChecked {
            session_id,
            has_changes,
            workspace_clean,
        }) {
            report_background_error("failed to report worktree merge check", err);
        }
    });
}

pub(super) fn handle_merge_checked(
    state: &mut AppState,
    action_tx: &mpsc::UnboundedSender<Action>,
    session_id: Uuid,
    has_changes: bool,
    workspace_clean: bool,
) {
    if has_changes {
        state.ui.merging_session_id = Some(session_id);
        state.ui.input_mode = InputMode::ConfirmMergeWorktree;
        return;
    }

    if !workspace_clean {
        show_toast(
            state,
            "Merge blocked: workspace has uncommitted changes",
            ToastLevel::Warning,
        );
        return;
    }

    start_worktree_merge(state, action_tx, session_id, false);
}

pub(super) fn handle_confirm_merge_with_commit(
    state: &mut AppState,
    action_tx: &mpsc::UnboundedSender<Action>,
) {
    if let Some(session_id) = state.ui.merging_session_id.take() {
        start_worktree_merge(state, action_tx, session_id, true);
    }
    state.ui.input_mode = InputMode::Normal;
}

fn start_worktree_merge(
    state: &mut AppState,
    action_tx: &mpsc::UnboundedSender<Action>,
    session_id: Uuid,
    commit_first: bool,
) {
    let Some((workspace_path, worktree_path, branch_name, agent_name)) =
        commit_merge_info_for_session(state, session_id)
    else {
        return;
    };

    show_toast(state, "Merging worktree…", ToastLevel::Info);

    let tx = action_tx.clone();
    tokio::task::spawn_blocking(move || {
        let outcome = run_worktree_merge(
            &workspace_path,
            &worktree_path,
            &branch_name,
            &agent_name,
            commit_first,
        );
        if let Err(err) = tx.send(Action::SessionWorktreeMergeFinished {
            session_id,
            committed: commit_first,
            outcome,
        }) {
            report_background_error("failed to report worktree merge result", err);
        }
    });
}

fn run_worktree_merge(
    workspace_path: &Path,
    worktree_path: &Path,
    branch_name: &str,
    agent_name: &str,
    commit_first: bool,
) -> WorktreeMergeOutcome {
    // Re-check cleanliness here: the earlier check ran before the user sat on
    // the confirm modal (or before this task was queued), so it can be stale.
    if !git::is_clean(workspace_path).unwrap_or(false) {
        return WorktreeMergeOutcome::WorkspaceDirty;
    }

    if commit_first {
        let commit_msg = format!("Agent {} work - auto-committed for merge", agent_name);
        if git::commit_all_changes(worktree_path, &commit_msg).is_err() {
            return WorktreeMergeOutcome::CommitFailed;
        }
    }

    if git::merge_branch(workspace_path, branch_name).is_err() {
        return WorktreeMergeOutcome::MergeFailed;
    }

    let worktree_removed = match git::remove_worktree(workspace_path, worktree_path, true) {
        Ok(()) => true,
        Err(err) => {
            report_background_error("failed to remove merged worktree", err);
            false
        }
    };
    WorktreeMergeOutcome::Merged { worktree_removed }
}

pub(super) fn handle_merge_finished(
    state: &mut AppState,
    session_id: Uuid,
    committed: bool,
    outcome: WorktreeMergeOutcome,
) {
    match outcome {
        WorktreeMergeOutcome::Merged { worktree_removed } => {
            clear_worktree_info(state, session_id);
            save_state(state, "failed to save merged worktree");
            let msg = if committed {
                "Worktree committed and merged successfully"
            } else {
                "Worktree merged successfully"
            };
            show_toast(state, msg, ToastLevel::Success);
            if !worktree_removed {
                show_toast(state, "Merged, but failed to remove worktree", ToastLevel::Error);
            }
        }
        WorktreeMergeOutcome::WorkspaceDirty => {
            show_toast(
                state,
                "Merge blocked: workspace has uncommitted changes",
                ToastLevel::Warning,
            );
        }
        WorktreeMergeOutcome::CommitFailed => {
            show_toast(state, "Failed to commit worktree changes", ToastLevel::Error);
        }
        WorktreeMergeOutcome::MergeFailed => {
            show_toast(
                state,
                "Merge failed — resolve conflicts manually",
                ToastLevel::Error,
            );
        }
    }
}

pub(super) fn handle_switch_to_worktree(
    state: &mut AppState,
    pty_manager: &PtyManager,
    pty_tx: &mpsc::Sender<Action>,
    session_id_opt: Option<Uuid>,
) {
    let Some(session_id) = session_id_opt else {
        if let Some(workspace) = state.selected_workspace_mut() {
            workspace.active_worktree_session_id = None;
            save_state(state, "failed to save main workspace switch");
        }
        return;
    };

    let Some((workspace_id, worktree_path, agent_name)) =
        worktree_info_for_session(state, session_id)
    else {
        return;
    };

    if let Some(viewer_id) = existing_worktree_viewer(state, session_id) {
        state.set_active_session_id(Some(viewer_id));
        state.set_output_scroll_offset(0);
        state.ui.focus = FocusPanel::OutputPane;

        if let Some(workspace) = state.selected_workspace_mut() {
            workspace.active_worktree_session_id = Some(session_id);
        }
        save_state(state, "failed to save worktree switch");
        return;
    }

    let short_id = &session_id.to_string()[..8];
    let terminal_name = format!("⎇ {}-{}", agent_name, short_id);
    let session = Session::new_worktree_viewer(workspace_id, terminal_name, session_id);
    let new_session_id = session.id;

    let ws_idx = state.ui.selected_workspace_idx;
    if let Some(ws) = state.data.workspaces.get_mut(ws_idx) {
        ws.touch();
    }

    let pty_rows = state.pane_rows();
    let cols = state.output_pane_cols();
    let terminal_agent_type = AgentType::Terminal(format!("worktree-{}", short_id));
    state
        .system
        .create_session_buffers(new_session_id, pty_rows, cols, &terminal_agent_type);

    match pty_manager.spawn_session(SessionSpawnConfig {
        session_id: new_session_id,
        agent_type: terminal_agent_type,
        working_dir: &worktree_path,
        rows: pty_rows,
        cols,
        pty_tx: pty_tx.clone(),
        resume: false,
        dangerously_skip_permissions: false,
        use_alternate_screen: state.system.use_alternate_screen,
    }) {
        Ok(handle) => {
            state.system.pty_handles.insert(new_session_id, handle);
            state.add_session(session);
            state.set_active_session_id(Some(new_session_id));
            state.ui.focus = FocusPanel::OutputPane;

            if let Some(workspace) = state.selected_workspace_mut() {
                workspace.active_worktree_session_id = Some(session_id);
            }

            save_state(state, "failed to save worktree viewer");
        }
        Err(_e) => {
            show_toast(state, "Failed to open worktree terminal", ToastLevel::Error);
            state.system.remove_session_buffers(&new_session_id);
        }
    }
}

fn merge_info_for_session(
    state: &AppState,
    session_id: Uuid,
) -> Option<(std::path::PathBuf, std::path::PathBuf, String)> {
    let session = state
        .data
        .sessions
        .values()
        .flatten()
        .find(|s| s.id == session_id)?;

    let (Some(worktree_path), Some(branch)) = (&session.worktree_path, &session.worktree_branch)
    else {
        return None;
    };

    let workspace_path = state
        .data
        .workspaces
        .iter()
        .find(|w| w.id == session.workspace_id)
        .map(|w| w.path.clone())?;

    Some((workspace_path, worktree_path.clone(), branch.clone()))
}

fn commit_merge_info_for_session(
    state: &AppState,
    session_id: Uuid,
) -> Option<(std::path::PathBuf, std::path::PathBuf, String, String)> {
    let session = state
        .data
        .sessions
        .values()
        .flatten()
        .find(|s| s.id == session_id)?;

    let (Some(worktree_path), Some(branch)) = (&session.worktree_path, &session.worktree_branch)
    else {
        return None;
    };

    let workspace_path = state
        .data
        .workspaces
        .iter()
        .find(|w| w.id == session.workspace_id)
        .map(|w| w.path.clone())?;
    let agent_name = session.agent_type.display_name().to_string();

    Some((
        workspace_path,
        worktree_path.clone(),
        branch.clone(),
        agent_name,
    ))
}

fn worktree_info_for_session(
    state: &AppState,
    session_id: Uuid,
) -> Option<(Uuid, std::path::PathBuf, String)> {
    state
        .data
        .sessions
        .values()
        .flatten()
        .find(|s| s.id == session_id)
        .and_then(|s| {
            s.worktree_path.as_ref().map(|path| {
                (
                    s.workspace_id,
                    path.clone(),
                    s.agent_type.display_name().to_string(),
                )
            })
        })
}

fn existing_worktree_viewer(state: &AppState, session_id: Uuid) -> Option<Uuid> {
    state
        .data
        .sessions
        .values()
        .flatten()
        .find(|s| s.worktree_viewer_for == Some(session_id) && s.status == SessionStatus::Running)
        .map(|s| s.id)
}

fn clear_worktree_info(state: &mut AppState, session_id: Uuid) {
    if let Some(session) = state.get_session_mut(session_id) {
        session.worktree_path = None;
        session.worktree_branch = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{existing_worktree_viewer, merge_info_for_session};
    use crate::app::AppState;
    use crate::models::{AgentType, Session, SessionStatus, Workspace};
    use std::path::PathBuf;

    #[test]
    fn merge_info_for_session_returns_workspace_worktree_and_branch() {
        let mut state = AppState::default();
        let workspace = Workspace::new("repo".to_string(), PathBuf::from("/tmp/repo"));
        let workspace_id = workspace.id;
        let session = Session::new_with_worktree(
            workspace_id,
            AgentType::Codex,
            false,
            PathBuf::from("/tmp/repo-worktree"),
            "worktree-branch".to_string(),
        );
        let session_id = session.id;
        state.data.workspaces.push(workspace);
        state.data.sessions.insert(workspace_id, vec![session]);

        let (workspace_path, worktree_path, branch) =
            merge_info_for_session(&state, session_id).unwrap();

        assert_eq!(workspace_path, PathBuf::from("/tmp/repo"));
        assert_eq!(worktree_path, PathBuf::from("/tmp/repo-worktree"));
        assert_eq!(branch, "worktree-branch");
    }

    #[test]
    fn existing_worktree_viewer_ignores_stopped_viewers() {
        let mut state = AppState::default();
        let workspace = Workspace::new("repo".to_string(), PathBuf::from("/tmp/repo"));
        let workspace_id = workspace.id;
        let agent = Session::new(workspace_id, AgentType::Codex, false);
        let agent_id = agent.id;
        let mut stopped_viewer =
            Session::new_worktree_viewer(workspace_id, "viewer".to_string(), agent_id);
        stopped_viewer.status = SessionStatus::Stopped;
        let running_viewer =
            Session::new_worktree_viewer(workspace_id, "viewer-2".to_string(), agent_id);
        let running_viewer_id = running_viewer.id;

        state.data.workspaces.push(workspace);
        state
            .data
            .sessions
            .insert(workspace_id, vec![agent, stopped_viewer, running_viewer]);

        assert_eq!(
            existing_worktree_viewer(&state, agent_id),
            Some(running_viewer_id)
        );
    }
}
