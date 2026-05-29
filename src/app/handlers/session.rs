use crate::app::pty_ops::resize_ptys_to_panes;
use crate::app::{Action, AppState, FocusPanel, InputMode, PendingDelete, Toast, ToastLevel};
use crate::git;
use crate::models::{AgentType, AttemptStatus, Session};
use crate::pty::{PtyHandle, PtyManager, SessionSpawnConfig};
use anyhow::Result;
use std::time::Duration;
use tokio::sync::mpsc;

use super::session_worktree::{
    handle_confirm_merge_with_commit, handle_merge_session_worktree, handle_switch_to_worktree,
};
use super::{report_background_error, report_runtime_error, save_state};

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

const SHELL_KILL_TIMEOUT: Duration = Duration::from_millis(500);
pub(crate) fn terminate_session_handle(mut handle: PtyHandle, is_terminal: bool) {
    if is_terminal {
        std::thread::spawn(move || {
            if let Err(err) = handle.interrupt_then_kill(SHELL_KILL_TIMEOUT) {
                report_background_error("failed to terminate terminal session", err);
            }
        });
    } else if let Err(err) = handle.kill() {
        report_background_error("failed to kill session", err);
    }
}

pub fn handle_session_action(
    state: &mut AppState,
    action: Action,
    pty_manager: &PtyManager,
    action_tx: &mpsc::UnboundedSender<Action>,
    pty_tx: &mpsc::Sender<Action>,
) -> Result<()> {
    match action {
        Action::CreateSession(agent_type, dangerously_skip_permissions, with_worktree) => {
            create_session(
                state,
                agent_type,
                dangerously_skip_permissions,
                with_worktree,
                pty_manager,
                pty_tx,
            );
        }
        Action::CreateTerminal => {
            create_terminal(state, pty_manager, pty_tx);
        }
        Action::ActivateSession(session_id) => {
            if state.ui.active_session_id != Some(session_id) {
                crate::app::selection::clear_active_text_selection(state);
            }
            state.ui.active_session_id = Some(session_id);
            state.ui.output_scroll_offset = 0;
            state.ui.output_content_length = 0;

            // Save as last active session for the workspace
            if let Some(ws) = state.selected_workspace_mut() {
                ws.last_active_session_id = Some(session_id);
            }
        }
        Action::RestartSession(session_id) => {
            restart_session(state, session_id, pty_manager, action_tx, pty_tx);
        }
        Action::StopSession(session_id) => {
            let send_error = state
                .system
                .pty_handles
                .get_mut(&session_id)
                .and_then(|handle| handle.send_input(&[0x03]).err());
            if let Some(err) = send_error {
                report_runtime_error(
                    state,
                    "failed to send stop signal to PTY",
                    err,
                    "Failed to stop session",
                );
            }
        }
        Action::KillSession(session_id) => {
            let is_terminal = state
                .data
                .sessions
                .values()
                .flatten()
                .find(|s| s.id == session_id)
                .map(|s| s.agent_type.is_terminal())
                .unwrap_or(false);

            if let Some(handle) = state.system.pty_handles.remove(&session_id) {
                terminate_session_handle(handle, is_terminal);
            }

            if let Some(session) = state.get_session_mut(session_id) {
                session.mark_stopped();
            }
            if state.ui.active_session_id == Some(session_id) {
                state.ui.active_session_id = None;
            }
            save_state(state, "failed to save killed session");
        }
        Action::InitiateDeleteSession(id, name) => {
            state.ui.pending_delete = Some(PendingDelete::Session(id, name));
        }
        Action::ConfirmDeleteSession => {
            confirm_delete_session(state);
        }
        Action::CancelPendingDelete => {
            state.ui.pending_delete = None;
        }
        Action::MergeSessionWorktree(session_id) => {
            handle_merge_session_worktree(state, session_id);
        }
        Action::ConfirmMergeWithCommit => {
            handle_confirm_merge_with_commit(state);
        }
        Action::CancelMerge => {
            state.ui.merging_session_id = None;
            state.ui.input_mode = InputMode::Normal;
        }
        Action::SwitchToWorktree(session_id_opt) => {
            handle_switch_to_worktree(state, pty_manager, pty_tx, session_id_opt);
        }
        Action::EnterCreateSessionMode => {
            if state.selected_workspace().is_some() {
                state.ui.input_mode = InputMode::CreateSession;
            }
        }
        Action::EnterSetStartCommandMode => {
            let session_info = state
                .selected_session()
                .filter(|s| s.agent_type.is_terminal())
                .map(|s| (s.id, s.start_command.clone()));

            if let Some((session_id, existing_cmd)) = session_info {
                state.ui.editing_session_id = Some(session_id);
                state.ui.input_buffer = existing_cmd.unwrap_or_default();
                state.ui.input_mode = InputMode::SetStartCommand;
            }
        }
        Action::SetStartCommand(session_id, command) => {
            if let Some(session) = state.get_session_mut(session_id) {
                session.start_command = if command.is_empty() {
                    None
                } else {
                    Some(command)
                };
            }
            state.ui.input_mode = InputMode::Normal;
            state.ui.input_buffer.clear();
            state.ui.editing_session_id = None;
            save_state(state, "failed to save start command");
        }
        Action::PinSession(session_id) => {
            if state.pin_terminal_for_selected(session_id) {
                state.ui.layout.split_view_enabled = true;
                let new_idx = state
                    .selected_workspace()
                    .map(|ws| ws.pinned_terminal_ids.len().saturating_sub(1))
                    .unwrap_or(0);
                if let Some(ws_ui) = state.ws_ui_mut() {
                    ws_ui.focused_pinned_pane = new_idx;
                }
                // Reset every live per-pane field at the new slot. Otherwise
                // a workspace-switch snapshot would lock in stale data from
                // a previously-unpinned terminal at the same slot.
                state.ui.focused_pinned_pane = new_idx;
                if new_idx < state.ui.pinned_scroll_offsets.len() {
                    state.ui.pinned_scroll_offsets[new_idx] = 0;
                    state.ui.pinned_text_selections[new_idx] = crate::app::TextSelection::default();
                    state.ui.pinned_on_replay[new_idx] = false;
                    state.ui.pinned_content_lengths[new_idx] = 0;
                }
                resize_ptys_to_panes(state);
                save_state(state, "failed to save pinned session");
            }
        }
        Action::UnpinSession(session_id) => {
            state.unpin_terminal_anywhere(session_id);
            // Mirror to legacy fields.
            let count = state
                .selected_workspace()
                .map(|ws| ws.pinned_terminal_ids.len())
                .unwrap_or(0);
            if state.ui.focused_pinned_pane >= count && count > 0 {
                state.ui.focused_pinned_pane = count - 1;
            }
            state.ui.pinned_content_lengths = [0; crate::models::MAX_PINNED_TERMINALS];
            resize_ptys_to_panes(state);
            save_state(state, "failed to save unpinned session");
        }
        Action::UnpinFocusedSession => {
            let focused = state
                .ws_ui()
                .map(|u| u.focused_pinned_pane)
                .unwrap_or(state.ui.focused_pinned_pane);
            if let Some(sid) = state.pinned_terminal_id_at(focused) {
                state.unpin_terminal_anywhere(sid);
                let count = state
                    .selected_workspace()
                    .map(|ws| ws.pinned_terminal_ids.len())
                    .unwrap_or(0);
                if state.ui.focused_pinned_pane >= count && count > 0 {
                    state.ui.focused_pinned_pane = count - 1;
                }
                if count == 0 {
                    state.ui.focus = FocusPanel::SessionList;
                }
                state.ui.pinned_content_lengths = [0; crate::models::MAX_PINNED_TERMINALS];
                resize_ptys_to_panes(state);
                save_state(state, "failed to save focused unpin");
            }
        }
        Action::ToggleSplitView => {
            state.ui.layout.split_view_enabled = !state.ui.layout.split_view_enabled;
            resize_ptys_to_panes(state);
        }
        Action::SessionExited(session_id, exit_code) => {
            state.system.pty_handles.remove(&session_id);
            if let Some(session) = state.get_session_mut(session_id) {
                if exit_code == 0 {
                    session.mark_stopped();
                } else {
                    session.mark_errored();
                }
            }
            save_state(state, "failed to save exited session");
        }
        Action::PtyOutput(session_id, data) => {
            let uses_transcript_scrollback = state
                .data
                .sessions
                .values()
                .flatten()
                .find(|s| s.id == session_id)
                .map(|s| s.agent_type.is_codex_like())
                .unwrap_or(false);

            // Process through live parser
            if let Some(parser) = state.system.output_buffers.get_mut(&session_id) {
                parser.process(&data);
            }

            if uses_transcript_scrollback {
                state.system.update_transcript_from_screen(session_id);
            } else {
                // Append raw bytes for append-style sessions; replay scrollback uses this for deep history.
                if let Some(raw_buf) = state.system.raw_output_buffers.get_mut(&session_id) {
                    raw_buf.append(&data);
                }
            }

            // Invalidate replay cache only if one exists (user is scrolled back)
            if state.system.replay_caches.contains_key(&session_id) {
                state.system.replay_caches.remove(&session_id);
            }
            // Only count as agent activity if this isn't an echo of recent user input.
            // Keystroke echoes arrive within ~50ms of SendInput; real agent output is autonomous.
            let is_echo = state
                .data
                .last_send_input
                .get(&session_id)
                .map(|t| t.elapsed().as_millis() < 500)
                .unwrap_or(false);
            if !is_echo {
                // Track when this work burst started (first output after being idle)
                if !state.is_session_working(session_id) {
                    state
                        .data
                        .work_started
                        .insert(session_id, std::time::Instant::now());
                }
                state
                    .data
                    .last_activity
                    .insert(session_id, std::time::Instant::now());
            }
        }
        Action::SendInput(session_id, data) => {
            state
                .data
                .last_send_input
                .insert(session_id, std::time::Instant::now());
            let send_error = state
                .system
                .pty_handles
                .get_mut(&session_id)
                .and_then(|handle| handle.send_input(&data).err());
            if let Some(err) = send_error {
                report_runtime_error(
                    state,
                    "failed to send input to PTY",
                    err,
                    "Failed to send input",
                );
            }
            if let Some(workspace_id) = state.workspace_id_for_session(session_id) {
                if let Some(ws) = state
                    .data
                    .workspaces
                    .iter_mut()
                    .find(|ws| ws.id == workspace_id)
                {
                    ws.touch();
                }
            }
        }
        _ => {} // This is a catch-all for any other Action variants not explicitly handled.
    }
    Ok(())
}

/// Register a freshly spawned session: insert its PTY handle, add it to state,
/// focus it, and persist. On spawn failure show a toast and drop its buffers.
/// Shared by [`create_session`] and [`create_terminal`].
fn finish_session_spawn(
    state: &mut AppState,
    session: Session,
    spawn_result: Result<PtyHandle>,
    failure_toast: &str,
    save_msg: &str,
) {
    let session_id = session.id;
    match spawn_result {
        Ok(handle) => {
            state.system.pty_handles.insert(session_id, handle);
            state.add_session(session);
            state.ui.active_session_id = Some(session_id);
            state.ui.focus = FocusPanel::SessionList;
            let session_count = state.sessions_for_selected_workspace().len();
            if session_count > 0 {
                state.ui.selected_session_idx = session_count - 1;
            }
            save_state(state, save_msg);
        }
        Err(_e) => {
            show_toast(state, failure_toast, ToastLevel::Error);
            state.system.remove_session_buffers(&session_id);
        }
    }
}

fn create_session(
    state: &mut AppState,
    agent_type: AgentType,
    dangerously_skip_permissions: bool,
    with_worktree: bool,
    pty_manager: &PtyManager,
    pty_tx: &mpsc::Sender<Action>,
) {
    let Some(workspace) = state.selected_workspace() else {
        return;
    };
    let workspace_id = workspace.id;
    let workspace_path = workspace.path.clone();
    let ws_idx = state.ui.selected_workspace_idx;

    // Create worktree only if requested (Alt key), is an agent, and workspace is a git repo
    let (session, working_dir) =
        if with_worktree && agent_type.is_agent() && git::is_git_repo(&workspace_path) {
            // Create a temporary session to get the ID for branch naming
            let temp_id = uuid::Uuid::new_v4();
            let short_id = &temp_id.to_string()[..8];
            let branch_name = git::session_branch_name(&agent_type.display_name(), short_id);
            let worktree_path = git::get_session_worktree_path(&workspace_path, short_id);

            // Create the worktree
            match git::create_worktree(&workspace_path, &branch_name, &worktree_path) {
                Ok(()) => {
                    // Create session with worktree info
                    let mut session = Session::new_with_worktree(
                        workspace_id,
                        agent_type.clone(),
                        dangerously_skip_permissions,
                        worktree_path.clone(),
                        branch_name,
                    );
                    // Override the ID to match what we used for naming
                    session.id = temp_id;
                    (session, worktree_path)
                }
                Err(_e) => {
                    show_toast(
                        state,
                        "Worktree creation failed, using workspace directly",
                        ToastLevel::Warning,
                    );
                    (
                        Session::new(workspace_id, agent_type.clone(), dangerously_skip_permissions),
                        workspace_path.clone(),
                    )
                }
            }
        } else {
            // Default: run in workspace directly (no worktree isolation)
            (
                Session::new(workspace_id, agent_type.clone(), dangerously_skip_permissions),
                workspace_path.clone(),
            )
        };

    let session_id = session.id;

    if let Some(ws) = state.data.workspaces.get_mut(ws_idx) {
        ws.touch();
    }

    let pty_rows = state.pane_rows();
    let cols = state.output_pane_cols();
    state.system.create_session_buffers(session_id, cols);

    let spawn_result = pty_manager.spawn_session(SessionSpawnConfig {
        session_id,
        agent_type,
        working_dir: &working_dir,
        rows: pty_rows,
        cols,
        pty_tx: pty_tx.clone(),
        resume: false,
        dangerously_skip_permissions,
        use_alternate_screen: state.system.use_alternate_screen,
    });
    finish_session_spawn(
        state,
        session,
        spawn_result,
        "Failed to spawn session",
        "failed to save created session",
    );
    state.ui.input_mode = InputMode::Normal;
}

fn create_terminal(
    state: &mut AppState,
    pty_manager: &PtyManager,
    pty_tx: &mpsc::Sender<Action>,
) {
    let Some(workspace) = state.selected_workspace() else {
        return;
    };
    let terminal_count = state
        .sessions_for_selected_workspace()
        .iter()
        .filter(|s| s.agent_type.is_terminal())
        .count();
    let name = format!("{}", terminal_count + 1);

    let agent_type = AgentType::Terminal(name);
    let session = Session::new(workspace.id, agent_type.clone(), false);
    let session_id = session.id;
    let workspace_path = workspace.path.clone();
    let ws_idx = state.ui.selected_workspace_idx;

    if let Some(ws) = state.data.workspaces.get_mut(ws_idx) {
        ws.touch();
    }

    let pty_rows = state.pane_rows();
    let cols = state.output_pane_cols();
    state.system.create_session_buffers(session_id, cols);

    let spawn_result = pty_manager.spawn_session(SessionSpawnConfig {
        session_id,
        agent_type,
        working_dir: &workspace_path,
        rows: pty_rows,
        cols,
        pty_tx: pty_tx.clone(),
        resume: false,
        dangerously_skip_permissions: false,
        use_alternate_screen: state.system.use_alternate_screen,
    });
    finish_session_spawn(
        state,
        session,
        spawn_result,
        "Failed to spawn terminal",
        "failed to save created terminal",
    );
}

fn restart_session(
    state: &mut AppState,
    session_id: uuid::Uuid,
    pty_manager: &PtyManager,
    action_tx: &mpsc::UnboundedSender<Action>,
    pty_tx: &mpsc::Sender<Action>,
) {
    let session_info = state
        .data
        .sessions
        .values()
        .flatten()
        .find(|s| s.id == session_id)
        .map(|s| {
            (
                s.agent_type.clone(),
                s.workspace_id,
                s.start_command.clone(),
                s.dangerously_skip_permissions,
                s.worktree_path.clone(),
            )
        });

    let Some((agent_type, workspace_id, start_command, dangerously_skip_permissions, worktree_path)) =
        session_info
    else {
        return;
    };

    let workspace_path = state
        .data
        .workspaces
        .iter()
        .find(|w| w.id == workspace_id)
        .map(|w| w.path.clone());

    let Some(workspace_path) = workspace_path else {
        return;
    };

    // Use worktree path if session has one, otherwise use workspace path
    let working_dir = worktree_path
        .as_ref()
        .filter(|p| p.exists())
        .cloned()
        .unwrap_or_else(|| workspace_path.clone());

    let pty_rows = state.pane_rows();
    let cols = state.output_pane_cols();
    state.system.create_session_buffers(session_id, cols);

    let resume = agent_type.is_agent();

    match pty_manager.spawn_session(SessionSpawnConfig {
        session_id,
        agent_type: agent_type.clone(),
        working_dir: &working_dir,
        rows: pty_rows,
        cols,
        pty_tx: pty_tx.clone(),
        resume,
        dangerously_skip_permissions,
        use_alternate_screen: state.system.use_alternate_screen,
    }) {
        Ok(handle) => {
            state.system.pty_handles.insert(session_id, handle);
            if let Some(session) = state.get_session_mut(session_id) {
                session.status = crate::models::SessionStatus::Running;
            }
            state.ui.active_session_id = Some(session_id);
            state.ui.focus = FocusPanel::OutputPane;

            if agent_type.is_terminal() {
                if let Some(cmd) = start_command {
                    if !cmd.is_empty() {
                        let tx = action_tx.clone();
                        let sid = session_id;
                        tokio::spawn(async move {
                            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
                            let mut input = cmd.into_bytes();
                            input.push(b'\n');
                            if let Err(err) = tx.send(Action::SendInput(sid, input)) {
                                report_background_error(
                                    "failed to queue terminal start command",
                                    err,
                                );
                            }
                        });
                    }
                }
            }
            save_state(state, "failed to save restarted session");
        }
        Err(_e) => {
            show_toast(state, "Failed to restart session", ToastLevel::Error);
            state.system.remove_session_buffers(&session_id);
            if let Some(session) = state.get_session_mut(session_id) {
                session.mark_errored();
            }
            save_state(state, "failed to save errored session");
        }
    }
}

fn confirm_delete_session(state: &mut AppState) {
    let Some(PendingDelete::Session(session_id, _)) = state.ui.pending_delete.take() else {
        return;
    };

    // Get session info before deleting
    let session_info: Option<(bool, Option<std::path::PathBuf>, Option<uuid::Uuid>)> = state
        .data
        .sessions
        .values()
        .flatten()
        .find(|s| s.id == session_id)
        .map(|s| {
            (
                s.agent_type.is_terminal(),
                s.worktree_path.clone(),
                s.parallel_attempt_id,
            )
        });

    let (is_terminal, session_worktree_path, parallel_attempt_id) =
        session_info.unwrap_or((false, None, None));

    // Check if this session is part of a parallel task and get cleanup info
    let parallel_cleanup_info: Option<(std::path::PathBuf, std::path::PathBuf, uuid::Uuid)> = {
        let workspace = state.selected_workspace();
        if let Some(ws) = workspace {
            if let Some(attempt_id) = parallel_attempt_id {
                // Find the parallel task and attempt
                ws.parallel_tasks.iter().find_map(|task| {
                    task.attempts
                        .iter()
                        .find(|a| a.id == attempt_id)
                        .map(|attempt| (ws.path.clone(), attempt.worktree_path.clone(), task.id))
                })
            } else {
                None
            }
        } else {
            None
        }
    };

    // Get workspace path for regular session worktree cleanup
    let workspace_path = state.selected_workspace().map(|ws| ws.path.clone());

    // Kill PTY handle
    if let Some(handle) = state.system.pty_handles.remove(&session_id) {
        terminate_session_handle(handle, is_terminal);
    }
    state.system.remove_session_buffers(&session_id);

    // Clean up worktree - either from parallel task or regular session
    if let Some((workspace_path, worktree_path, task_id)) = parallel_cleanup_info {
        // Remove the parallel task worktree
        if let Err(err) = git::remove_worktree(&workspace_path, &worktree_path, true) {
            report_runtime_error(
                state,
                "failed to remove parallel session worktree",
                err,
                "Failed to remove worktree",
            );
        }

        // Mark the attempt as failed and potentially clean up the task
        if let Some(ws) = state.selected_workspace_mut() {
            if let Some(task) = ws.get_parallel_task_mut(task_id) {
                // Find and mark the attempt as failed
                if let Some(attempt) = task
                    .attempts
                    .iter_mut()
                    .find(|a| a.session_id == session_id)
                {
                    attempt.status = AttemptStatus::Failed;
                }

                // If all attempts are now finished, mark task as awaiting selection
                if task.all_attempts_finished() {
                    task.mark_awaiting_selection();
                }

                // If all attempts failed or were deleted, cancel the whole task
                let all_failed = task
                    .attempts
                    .iter()
                    .all(|a| a.status == AttemptStatus::Failed);
                if all_failed && !task.attempts.is_empty() {
                    task.mark_cancelled();
                }
            }
        }
    } else if let (Some(worktree_path), Some(workspace_path)) =
        (session_worktree_path, workspace_path)
    {
        // Clean up regular session worktree
        if let Err(err) = git::remove_worktree(&workspace_path, &worktree_path, true) {
            report_runtime_error(
                state,
                "failed to remove session worktree",
                err,
                "Failed to remove worktree",
            );
        }
    }

    state.delete_session(session_id);
    if state.ui.active_session_id == Some(session_id) {
        state.ui.active_session_id = None;
    }
    let session_count = state.sessions_for_selected_workspace().len();
    if state.ui.selected_session_idx >= session_count && session_count > 0 {
        state.ui.selected_session_idx = session_count - 1;
    }
    save_state(state, "failed to save deleted session");
}
