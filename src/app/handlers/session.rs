use crate::app::pty_ops::request_pty_resize;
use crate::app::{
    Action, AppState, FocusPanel, InputMode, PendingDelete, Toast, ToastLevel,
};
use crate::git;
use crate::models::{AgentType, AttemptStatus, Session};
use crate::pty::{PtyHandle, PtyManager, SessionSpawnConfig};
use anyhow::Result;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::session_worktree::{
    handle_confirm_merge_with_commit, handle_merge_checked, handle_merge_finished,
    handle_merge_session_worktree, handle_switch_to_worktree,
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
                action_tx,
                pty_tx,
            );
        }
        Action::CreateTerminal => {
            create_terminal(state, pty_manager, pty_tx);
        }
        Action::ActivateSession(session_id) => {
            if state.active_session_id() != Some(session_id) {
                crate::app::selection::clear_active_text_selection(state);
            }
            state.set_active_session_id(Some(session_id));
            state.set_output_scroll_offset(0);
            state.set_output_content_length(0);

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
            state.clear_active_session_everywhere(session_id);
            save_state(state, "failed to save killed session");
        }
        Action::InitiateDeleteSession(id, name) => {
            state.ui.pending_delete = Some(PendingDelete::Session(id, name));
        }
        Action::ConfirmDeleteSession => {
            confirm_delete_session(state, action_tx);
        }
        Action::CancelPendingDelete => {
            state.ui.pending_delete = None;
        }
        Action::MergeSessionWorktree(session_id) => {
            handle_merge_session_worktree(state, action_tx, session_id);
        }
        Action::SessionWorktreeMergeChecked {
            session_id,
            has_changes,
            workspace_clean,
        } => {
            handle_merge_checked(state, action_tx, session_id, has_changes, workspace_clean);
        }
        Action::SessionWorktreeMergeFinished {
            session_id,
            committed,
            outcome,
        } => {
            handle_merge_finished(state, session_id, committed, outcome);
        }
        Action::SessionWorktreeCreated {
            workspace_id,
            session_id,
            agent_type,
            dangerously_skip_permissions,
            worktree,
            failed,
        } => {
            finish_worktree_session_spawn(
                state,
                pty_manager,
                pty_tx,
                workspace_id,
                session_id,
                agent_type,
                dangerously_skip_permissions,
                worktree,
                failed,
            );
        }
        Action::ConfirmMergeWithCommit => {
            handle_confirm_merge_with_commit(state, action_tx);
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
                // Focus the new pane; its per-pane state starts zeroed because
                // pin_terminal_for_selected pushes a fresh PinnedPaneState.
                let new_idx = state.pinned_count().saturating_sub(1);
                state.set_focused_pinned_pane(new_idx);
                request_pty_resize(state);
                save_state(state, "failed to save pinned session");
            }
        }
        Action::UnpinSession(session_id) => {
            // unpin_terminal_anywhere removes the pane's state and clamps the
            // focused index in the owning workspace's ws_ui.
            state.unpin_terminal_anywhere(session_id);
            request_pty_resize(state);
            save_state(state, "failed to save unpinned session");
        }
        Action::UnpinFocusedSession => {
            let focused = state.focused_pinned_pane();
            if let Some(sid) = state.pinned_terminal_id_at(focused) {
                state.unpin_terminal_anywhere(sid);
                if state.pinned_count() == 0 {
                    state.ui.focus = FocusPanel::SessionList;
                }
                request_pty_resize(state);
                save_state(state, "failed to save focused unpin");
            }
        }
        Action::ToggleSplitView => {
            state.ui.layout.split_view_enabled = !state.ui.layout.split_view_enabled;
            request_pty_resize(state);
        }
        Action::SessionExited(session_id, exit_code) => {
            if let Some(mut handle) = state.system.pty_handles.remove(&session_id) {
                handle.mark_exited();
            }
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
            // Redraw-style agents (Claude, Codex) repaint a viewport rather than
            // emitting append-only output, so their scrollback is reconstructed
            // from screen snapshots via frame alignment. (Claude 2.1.185+ renders
            // in the alternate screen / full-repaint mode, so the older
            // scroll-based capture no longer applies.) Append-style sessions use
            // raw byte replay.
            let uses_transcript = state
                .data
                .sessions
                .values()
                .flatten()
                .find(|s| s.id == session_id)
                .map(|s| s.agent_type.is_redraw_style())
                .unwrap_or(false);

            let output_chunks = state.system.synchronized_output_chunks(session_id, &data);
            let has_processed_output = !output_chunks.is_empty();

            for chunk in output_chunks {
                if chunk.is_empty() {
                    continue;
                }

                // Process through live parser
                if let Some(parser) = state.system.output_buffers.get_mut(&session_id) {
                    parser.process(&chunk);
                }

                if uses_transcript {
                    state.system.update_transcript_from_screen(session_id);
                } else {
                    // Append raw bytes for append-style sessions; replay scrollback uses this for deep history.
                    if let Some(raw_buf) = state.system.raw_output_buffers.get_mut(&session_id) {
                        raw_buf.append(&chunk);
                    }
                }
            }

            // Invalidate replay cache only if one exists (user is scrolled back)
            if has_processed_output && state.system.replay_caches.contains_key(&session_id) {
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
) -> bool {
    let session_id = session.id;
    match spawn_result {
        Ok(handle) => {
            state.system.pty_handles.insert(session_id, handle);
            state.add_session(session);
            state.set_active_session_id(Some(session_id));
            state.ui.focus = FocusPanel::SessionList;
            let session_count = state.sessions_for_selected_workspace().len();
            if session_count > 0 {
                state.set_selected_session_idx(session_count - 1);
            }
            // Sync every PTY/parser to its pane so the new session starts
            // pixel-consistent with the layout (no-op when sizes already match).
            request_pty_resize(state);
            save_state(state, save_msg);
            true
        }
        Err(_e) => {
            show_toast(state, failure_toast, ToastLevel::Error);
            state.system.remove_session_buffers(&session_id);
            false
        }
    }
}

fn create_session(
    state: &mut AppState,
    agent_type: AgentType,
    dangerously_skip_permissions: bool,
    with_worktree: bool,
    pty_manager: &PtyManager,
    action_tx: &mpsc::UnboundedSender<Action>,
    pty_tx: &mpsc::Sender<Action>,
) -> Option<Uuid> {
    let Some(workspace) = state.selected_workspace() else {
        return None;
    };
    let workspace_id = workspace.id;
    let workspace_path = workspace.path.clone();
    let ws_idx = state.ui.selected_workspace_idx;

    if let Some(ws) = state.data.workspaces.get_mut(ws_idx) {
        ws.touch();
    }
    state.ui.input_mode = InputMode::Normal;

    // Create worktree only if requested (Alt key), is an agent, and workspace is
    // a git repo. `git worktree add` blocks for a noticeable moment on big
    // repos, so it runs on a blocking thread; the session spawn completes when
    // SessionWorktreeCreated arrives.
    if with_worktree && agent_type.is_agent() && git::is_git_repo(&workspace_path) {
        let session_id = uuid::Uuid::new_v4();
        let short_id = session_id.to_string()[..8].to_string();
        let branch_name = git::session_branch_name(&agent_type.display_name(), &short_id);
        let worktree_path = git::get_session_worktree_path(&workspace_path, &short_id);

        let tx = action_tx.clone();
        tokio::task::spawn_blocking(move || {
            let worktree = match git::create_worktree(&workspace_path, &branch_name, &worktree_path)
            {
                Ok(()) => Some((worktree_path, branch_name)),
                Err(err) => {
                    report_background_error("failed to create session worktree", err);
                    None
                }
            };
            let failed = worktree.is_none();
            if let Err(err) = tx.send(Action::SessionWorktreeCreated {
                workspace_id,
                session_id,
                agent_type,
                dangerously_skip_permissions,
                worktree,
                failed,
            }) {
                report_background_error("failed to report created session worktree", err);
            }
        });
        return Some(session_id);
    }

    // Default: run in workspace directly (no worktree isolation)
    let session = Session::new(workspace_id, agent_type.clone(), dangerously_skip_permissions);
    let session_id = session.id;

    let pty_rows = state.pane_rows();
    let cols = state.output_pane_cols();
    state
        .system
        .create_session_buffers(session_id, pty_rows, cols, &agent_type);

    let spawn_result = pty_manager.spawn_session(SessionSpawnConfig {
        session_id,
        agent_type,
        working_dir: &workspace_path,
        rows: pty_rows,
        cols,
        pty_tx: pty_tx.clone(),
        resume: false,
        dangerously_skip_permissions,
        use_alternate_screen: state.system.use_alternate_screen,
    });
    let started = finish_session_spawn(
        state,
        session,
        spawn_result,
        "Failed to spawn session",
        "failed to save created session",
    );
    started.then_some(session_id)
}

/// Completion of a worktree-backed session creation: the worktree was created
/// (or failed) on a blocking thread; build the session and spawn its PTY.
#[allow(clippy::too_many_arguments)]
fn finish_worktree_session_spawn(
    state: &mut AppState,
    pty_manager: &PtyManager,
    pty_tx: &mpsc::Sender<Action>,
    workspace_id: Uuid,
    session_id: Uuid,
    agent_type: AgentType,
    dangerously_skip_permissions: bool,
    worktree: Option<(std::path::PathBuf, String)>,
    failed: bool,
) {
    if failed {
        show_toast(
            state,
            "Worktree creation failed, using workspace directly",
            ToastLevel::Warning,
        );
    }

    // The workspace may have been deleted while the worktree was being created.
    let Some(workspace_path) = state
        .data
        .workspaces
        .iter()
        .find(|ws| ws.id == workspace_id)
        .map(|ws| ws.path.clone())
    else {
        return;
    };

    let (mut session, working_dir) = match worktree {
        Some((worktree_path, branch_name)) => {
            let session = Session::new_with_worktree(
                workspace_id,
                agent_type.clone(),
                dangerously_skip_permissions,
                worktree_path.clone(),
                branch_name,
            );
            (session, worktree_path)
        }
        None => (
            Session::new(workspace_id, agent_type.clone(), dangerously_skip_permissions),
            workspace_path,
        ),
    };
    // Keep the ID used for branch/worktree naming.
    session.id = session_id;

    let pty_rows = state.pane_rows();
    let cols = state.output_pane_cols();
    state
        .system
        .create_session_buffers(session_id, pty_rows, cols, &agent_type);

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
}

fn create_terminal(
    state: &mut AppState,
    pty_manager: &PtyManager,
    pty_tx: &mpsc::Sender<Action>,
) -> Option<Uuid> {
    let Some(workspace) = state.selected_workspace() else {
        return None;
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
    state
        .system
        .create_session_buffers(session_id, pty_rows, cols, &agent_type);

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
    let started = finish_session_spawn(
        state,
        session,
        spawn_result,
        "Failed to spawn terminal",
        "failed to save created terminal",
    );
    started.then_some(session_id)
}

/// Bootstrap a freshly created/opened workspace with the default session layout:
/// one Claude agent shown in the main pane plus two pinned terminals. Assumes the
/// target workspace is already selected.
pub fn start_default_workspace_sessions(
    state: &mut AppState,
    pty_manager: &PtyManager,
    action_tx: &mpsc::UnboundedSender<Action>,
    pty_tx: &mpsc::Sender<Action>,
) {
    // Claude agent (no worktree, permissions prompt on) in the main pane.
    let claude_id = create_session(
        state,
        AgentType::Claude,
        false,
        false,
        pty_manager,
        action_tx,
        pty_tx,
    );

    // Two terminals, pinned by default.
    let mut pinned_any = false;
    for _ in 0..2 {
        if let Some(terminal_id) = create_terminal(state, pty_manager, pty_tx) {
            if state.pin_terminal_for_selected(terminal_id) {
                pinned_any = true;
            }
        }
    }

    if pinned_any {
        state.ui.layout.split_view_enabled = true;
        let focused = state.pinned_count().saturating_sub(1);
        state.set_focused_pinned_pane(focused);
    }

    // Show the Claude agent in the main output pane.
    if let Some(claude_id) = claude_id {
        state.set_active_session_id(Some(claude_id));
        if let Some(idx) = state
            .sessions_for_selected_workspace()
            .iter()
            .position(|s| s.id == claude_id)
        {
            state.set_selected_session_idx(idx);
        }
    }

    // Re-layout PTYs/parsers for the new pinned-pane layout.
    request_pty_resize(state);
    save_state(state, "failed to save default workspace sessions");
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
    state
        .system
        .create_session_buffers(session_id, pty_rows, cols, &agent_type);

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
            state.set_active_session_id(Some(session_id));
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

/// Remove a worktree on a blocking thread (git can stall for seconds on big
/// repos); failures surface as a toast via the action channel.
fn remove_worktree_in_background(
    action_tx: &mpsc::UnboundedSender<Action>,
    workspace_path: std::path::PathBuf,
    worktree_path: std::path::PathBuf,
    context: &'static str,
) {
    let tx = action_tx.clone();
    tokio::task::spawn_blocking(move || {
        if let Err(err) = git::remove_worktree(&workspace_path, &worktree_path, true) {
            report_background_error(context, err);
            let _ = tx.send(Action::ShowToast(
                "Failed to remove worktree".to_string(),
                ToastLevel::Error,
            ));
        }
    });
}

fn confirm_delete_session(state: &mut AppState, action_tx: &mpsc::UnboundedSender<Action>) {
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
        remove_worktree_in_background(
            action_tx,
            workspace_path,
            worktree_path,
            "failed to remove parallel session worktree",
        );

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
        remove_worktree_in_background(
            action_tx,
            workspace_path,
            worktree_path,
            "failed to remove session worktree",
        );
    }

    // delete_session clears active_session_id in every workspace's ws_ui.
    state.delete_session(session_id);
    let session_count = state.sessions_for_selected_workspace().len();
    if state.selected_session_idx() >= session_count && session_count > 0 {
        state.set_selected_session_idx(session_count - 1);
    }
    save_state(state, "failed to save deleted session");
}
