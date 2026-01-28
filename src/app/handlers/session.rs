use crate::app::{Action, AppState, FocusPanel, InputMode, PendingDelete, PARSER_BUFFER_ROWS, TERMINAL_SCROLLBACK_LIMIT};
use crate::git;
use crate::models::{AgentType, AttemptStatus, Session};
use crate::persistence;
use crate::pty::{PtyHandle, PtyManager};
use crate::app::pty_ops::resize_ptys_to_panes;
use anyhow::Result;
use std::time::Duration;
use tokio::sync::mpsc;

const SHELL_KILL_TIMEOUT: Duration = Duration::from_millis(500);

pub(crate) fn terminate_session_handle(mut handle: PtyHandle, is_terminal: bool) {
    if is_terminal {
        std::thread::spawn(move || {
            let _ = handle.interrupt_then_kill(SHELL_KILL_TIMEOUT);
        });
    } else {
        let _ = handle.kill();
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
            if let Some(workspace) = state.selected_workspace() {
                let workspace_id = workspace.id;
                let workspace_path = workspace.path.clone();
                let ws_idx = state.ui.selected_workspace_idx;

                // Create worktree only if requested (Alt key), is an agent, and workspace is a git repo
                let (session, working_dir) = if with_worktree && agent_type.is_agent() && git::is_git_repo(&workspace_path) {
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
                            // Don't use eprintln! in TUI - it corrupts the display
                            // Fallback to regular session in workspace
                            (Session::new(workspace_id, agent_type.clone(), dangerously_skip_permissions), workspace_path.clone())
                        }
                    }
                } else {
                    // Default: run in workspace directly (no worktree isolation)
                    (Session::new(workspace_id, agent_type.clone(), dangerously_skip_permissions), workspace_path.clone())
                };

                let session_id = session.id;

                if let Some(ws) = state.data.workspaces.get_mut(ws_idx) {
                    ws.touch();
                }

                let pty_rows = state.pane_rows();
                let cols = state.output_pane_cols();
                let parser = vt100::Parser::new(PARSER_BUFFER_ROWS, cols, TERMINAL_SCROLLBACK_LIMIT);
                state.system.output_buffers.insert(session_id, parser);

                match pty_manager.spawn_session(
                    session_id,
                    agent_type,
                    &working_dir,
                    pty_rows,
                    cols,
                    pty_tx.clone(),
                    dangerously_skip_permissions,
                ) {
                    Ok(handle) => {
                        state.system.pty_handles.insert(session_id, handle);
                        state.add_session(session);
                        state.ui.active_session_id = Some(session_id);
                        state.ui.focus = FocusPanel::SessionList;
                        let session_count = state.sessions_for_selected_workspace().len();
                        if session_count > 0 {
                            state.ui.selected_session_idx = session_count - 1;
                        }
                        let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
                    }
                    Err(_e) => {
                        // Don't use eprintln! in TUI - it corrupts the display
                        // Session was not added to state, just clean up the buffer
                        state.system.output_buffers.remove(&session_id);
                    }
                }
                state.ui.input_mode = InputMode::Normal;
            }
        }
        Action::CreateTerminal => {
            if let Some(workspace) = state.selected_workspace() {
                let terminal_count = state.sessions_for_selected_workspace()
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
                let parser = vt100::Parser::new(PARSER_BUFFER_ROWS, cols, TERMINAL_SCROLLBACK_LIMIT);
                state.system.output_buffers.insert(session_id, parser);

                match pty_manager.spawn_session(
                    session_id,
                    agent_type,
                    &workspace_path,
                    pty_rows,
                    cols,
                    pty_tx.clone(),
                    false,
                ) {
                    Ok(handle) => {
                        state.system.pty_handles.insert(session_id, handle);
                                                state.add_session(session);
                        state.ui.active_session_id = Some(session_id);
                        state.ui.focus = FocusPanel::SessionList;
                        let session_count = state.sessions_for_selected_workspace().len();
                        if session_count > 0 {
                            state.ui.selected_session_idx = session_count - 1;
                        }
                        let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
                    }
                    Err(_e) => {
                        // Don't use eprintln! in TUI - it corrupts the display
                        // Session was not added to state, just clean up the buffer
                        state.system.output_buffers.remove(&session_id);
                    }
                }
            }
        }
        Action::ActivateSession(session_id) => {
            state.ui.active_session_id = Some(session_id);
            state.ui.output_scroll_offset = 0;
            state.ui.output_content_length = 0;

            // Save as last active session for the workspace
            if let Some(ws) = state.selected_workspace_mut() {
                ws.last_active_session_id = Some(session_id);
            }
        }
        Action::RestartSession(session_id) => {
            let session_info = state.data.sessions.values().flatten()
                .find(|s| s.id == session_id)
                .map(|s| (
                    s.agent_type.clone(),
                    s.workspace_id,
                    s.start_command.clone(),
                    s.dangerously_skip_permissions,
                    s.worktree_path.clone(),
                ));

            if let Some((agent_type, workspace_id, start_command, dangerously_skip_permissions, worktree_path)) = session_info {
                let workspace_path = state.data.workspaces.iter()
                    .find(|w| w.id == workspace_id)
                    .map(|w| w.path.clone());

                if let Some(workspace_path) = workspace_path {
                    // Use worktree path if session has one, otherwise use workspace path
                    let working_dir = worktree_path.as_ref()
                        .filter(|p| p.exists())
                        .cloned()
                        .unwrap_or_else(|| workspace_path.clone());

                    let pty_rows = state.pane_rows();
                    let cols = state.output_pane_cols();
                    let parser = vt100::Parser::new(PARSER_BUFFER_ROWS, cols, TERMINAL_SCROLLBACK_LIMIT);
                    state.system.output_buffers.insert(session_id, parser);

                    let resume = agent_type.is_agent();

                    match pty_manager.spawn_session_with_resume(
                        session_id,
                        agent_type.clone(),
                        &working_dir,
                        pty_rows,
                        cols,
                        pty_tx.clone(),
                        resume,
                        dangerously_skip_permissions,
                    ) {
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
                                            let _ = tx.send(Action::SendInput(sid, input));
                                        });
                                    }
                                }
                            }
                            let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
                        }
                        Err(_e) => {
                            // Don't use eprintln! in TUI - it corrupts the display
                            // Mark session as errored so user sees it failed
                            state.system.output_buffers.remove(&session_id);
                            if let Some(session) = state.get_session_mut(session_id) {
                                session.mark_errored();
                            }
                        }
                    }
                }
            }
        }
        Action::StopSession(session_id) => {
            if let Some(handle) = state.system.pty_handles.get_mut(&session_id) {
                let _ = handle.send_input(&[0x03]); // Ctrl+C
            }
        }
        Action::KillSession(session_id) => {
            let is_terminal = state.data.sessions
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
            let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
        }
        Action::InitiateDeleteSession(id, name) => {
            state.ui.pending_delete = Some(PendingDelete::Session(id, name));
        }
        Action::ConfirmDeleteSession => {
            if let Some(PendingDelete::Session(session_id, _)) = state.ui.pending_delete.take() {
                // Get session info before deleting
                let session_info: Option<(bool, Option<std::path::PathBuf>, Option<uuid::Uuid>)> = state.data.sessions
                    .values()
                    .flatten()
                    .find(|s| s.id == session_id)
                    .map(|s| (s.agent_type.is_terminal(), s.worktree_path.clone(), s.parallel_attempt_id));

                let (is_terminal, session_worktree_path, parallel_attempt_id) = session_info.unwrap_or((false, None, None));

                // Check if this session is part of a parallel task and get cleanup info
                let parallel_cleanup_info: Option<(std::path::PathBuf, std::path::PathBuf, uuid::Uuid)> = {
                    let workspace = state.selected_workspace();
                    if let Some(ws) = workspace {
                        if let Some(attempt_id) = parallel_attempt_id {
                            // Find the parallel task and attempt
                            ws.parallel_tasks.iter()
                                .find_map(|task| {
                                    task.attempts.iter()
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
                state.system.output_buffers.remove(&session_id);

                // Clean up worktree - either from parallel task or regular session
                if let Some((workspace_path, worktree_path, task_id)) = parallel_cleanup_info {
                    // Remove the parallel task worktree
                    let _ = git::remove_worktree(&workspace_path, &worktree_path, true);

                    // Mark the attempt as failed and potentially clean up the task
                    if let Some(ws) = state.selected_workspace_mut() {
                        if let Some(task) = ws.get_parallel_task_mut(task_id) {
                            // Find and mark the attempt as failed
                            if let Some(attempt) = task.attempts.iter_mut()
                                .find(|a| a.session_id == session_id)
                            {
                                attempt.status = AttemptStatus::Failed;
                            }

                            // If all attempts are now finished, mark task as awaiting selection
                            if task.all_attempts_finished() {
                                task.mark_awaiting_selection();
                            }

                            // If all attempts failed or were deleted, cancel the whole task
                            let all_failed = task.attempts.iter()
                                .all(|a| a.status == AttemptStatus::Failed);
                            if all_failed && !task.attempts.is_empty() {
                                task.mark_cancelled();
                            }
                        }
                    }
                } else if let (Some(worktree_path), Some(workspace_path)) = (session_worktree_path, workspace_path) {
                    // Clean up regular session worktree
                    let _ = git::remove_worktree(&workspace_path, &worktree_path, true);
                }

                state.delete_session(session_id);
                if state.ui.active_session_id == Some(session_id) {
                    state.ui.active_session_id = None;
                }
                let session_count = state.sessions_for_selected_workspace().len();
                if state.ui.selected_session_idx >= session_count && session_count > 0 {
                    state.ui.selected_session_idx = session_count - 1;
                }
                let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
            }
        }
        Action::CancelPendingDelete => {
            state.ui.pending_delete = None;
        }
        Action::MergeSessionWorktree(session_id) => {
            // Find session info
            let merge_info: Option<(std::path::PathBuf, std::path::PathBuf, String)> = {
                let session = state.data.sessions.values().flatten()
                    .find(|s| s.id == session_id);

                if let Some(s) = session {
                    if let (Some(worktree_path), Some(branch)) = (&s.worktree_path, &s.worktree_branch) {
                        // Get workspace path
                        let workspace_path = state.data.workspaces.iter()
                            .find(|w| w.id == s.workspace_id)
                            .map(|w| w.path.clone());

                        workspace_path.map(|wp| (wp, worktree_path.clone(), branch.clone()))
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            if let Some((workspace_path, worktree_path, branch_name)) = merge_info {
                // Check if worktree has uncommitted changes
                if git::worktree_has_changes(&worktree_path) {
                    // Show confirmation modal instead of erroring
                    state.ui.merging_session_id = Some(session_id);
                    state.ui.input_mode = InputMode::ConfirmMergeWorktree;
                    return Ok(());
                }

                // Check if main workspace is clean before merging
                if !git::is_clean(&workspace_path).unwrap_or(false) {
                    // Don't use eprintln! in TUI - it corrupts the display
                    // TODO: Add proper notification system for user feedback
                    return Ok(());
                }

                // No uncommitted changes - proceed with merge directly
                let _main_branch = git::get_current_branch(&workspace_path)
                    .unwrap_or_else(|_| "main".to_string());

                // Perform the merge
                match git::merge_branch(&workspace_path, &branch_name) {
                    Ok(()) => {
                        // Merge successful - clean up the worktree
                        let _ = git::remove_worktree(&workspace_path, &worktree_path, true);

                        // Clear worktree info from session
                        if let Some(session) = state.get_session_mut(session_id) {
                            session.worktree_path = None;
                            session.worktree_branch = None;
                        }

                        let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
                        // Don't use eprintln! in TUI - it corrupts the display
                        // Merge successful
                    }
                    Err(_e) => {
                        // Don't use eprintln! in TUI - it corrupts the display
                        // TODO: Add proper notification system for user feedback
                    }
                }
            }
        }
        Action::ConfirmMergeWithCommit => {
            if let Some(session_id) = state.ui.merging_session_id.take() {
                // Find session info
                let merge_info: Option<(std::path::PathBuf, std::path::PathBuf, String, String)> = {
                    let session = state.data.sessions.values().flatten()
                        .find(|s| s.id == session_id);

                    if let Some(s) = session {
                        if let (Some(worktree_path), Some(branch)) = (&s.worktree_path, &s.worktree_branch) {
                            let workspace_path = state.data.workspaces.iter()
                                .find(|w| w.id == s.workspace_id)
                                .map(|w| w.path.clone());

                            workspace_path.map(|wp| {
                                let agent_name = s.agent_type.display_name().to_string();
                                (wp, worktree_path.clone(), branch.clone(), agent_name)
                            })
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                if let Some((workspace_path, worktree_path, branch_name, agent_name)) = merge_info {
                    // Check if main workspace is clean before merging
                    if !git::is_clean(&workspace_path).unwrap_or(false) {
                        // Don't use eprintln! in TUI - it corrupts the display
                        // TODO: Add proper notification system for user feedback
                        state.ui.input_mode = InputMode::Normal;
                        return Ok(());
                    }

                    // Commit all changes first
                    let commit_msg = format!("Agent {} work - auto-committed for merge", agent_name);
                    if let Err(_e) = git::commit_all_changes(&worktree_path, &commit_msg) {
                        // Don't use eprintln! in TUI - it corrupts the display
                        // TODO: Add proper notification system for user feedback
                        state.ui.input_mode = InputMode::Normal;
                        return Ok(());
                    }

                    // Get the main branch name
                    let _main_branch = git::get_current_branch(&workspace_path)
                        .unwrap_or_else(|_| "main".to_string());

                    // Perform the merge
                    match git::merge_branch(&workspace_path, &branch_name) {
                        Ok(()) => {
                            // Merge successful - clean up the worktree
                            let _ = git::remove_worktree(&workspace_path, &worktree_path, true);

                            // Clear worktree info from session
                            if let Some(session) = state.get_session_mut(session_id) {
                                session.worktree_path = None;
                                session.worktree_branch = None;
                            }

                            let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
                            // Don't use eprintln! in TUI - it corrupts the display
                            // Merge successful
                        }
                        Err(_e) => {
                            // Don't use eprintln! in TUI - it corrupts the display
                            // TODO: Add proper notification system for user feedback
                        }
                    }
                }
            }
            state.ui.input_mode = InputMode::Normal;
        }
        Action::CancelMerge => {
            state.ui.merging_session_id = None;
            state.ui.input_mode = InputMode::Normal;
        }
        Action::SwitchToWorktree(session_id_opt) => {
            // Switch the workspace's active worktree view
            if let Some(session_id) = session_id_opt {
                // Switching TO a worktree - need to create or reuse a viewer terminal

                // First, get info about the agent session's worktree
                let worktree_info: Option<(uuid::Uuid, std::path::PathBuf, String)> = state.data.sessions
                    .values()
                    .flatten()
                    .find(|s| s.id == session_id)
                    .and_then(|s| {
                        s.worktree_path.as_ref().map(|path| {
                            (s.workspace_id, path.clone(), s.agent_type.display_name().to_string())
                        })
                    });

                if let Some((workspace_id, worktree_path, agent_name)) = worktree_info {
                    // Check if a viewer terminal already exists for this session
                    let existing_viewer_id: Option<uuid::Uuid> = state.data.sessions
                        .values()
                        .flatten()
                        .find(|s| s.worktree_viewer_for == Some(session_id) && s.status == crate::models::SessionStatus::Running)
                        .map(|s| s.id);

                    if let Some(viewer_id) = existing_viewer_id {
                        // Reuse existing viewer terminal
                        state.ui.active_session_id = Some(viewer_id);
                        state.ui.output_scroll_offset = 0;
                        state.ui.focus = FocusPanel::OutputPane;

                        // Update workspace active worktree
                        if let Some(workspace) = state.selected_workspace_mut() {
                            workspace.active_worktree_session_id = Some(session_id);
                        }
                        let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
                    } else {
                        // Create a new viewer terminal
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
                        let parser = vt100::Parser::new(PARSER_BUFFER_ROWS, cols, TERMINAL_SCROLLBACK_LIMIT);
                        state.system.output_buffers.insert(new_session_id, parser);

                        match pty_manager.spawn_session(
                            new_session_id,
                            AgentType::Terminal(format!("worktree-{}", short_id)),
                            &worktree_path,
                            pty_rows,
                            cols,
                            pty_tx.clone(),
                            false,
                        ) {
                            Ok(handle) => {
                                state.system.pty_handles.insert(new_session_id, handle);
                                state.add_session(session);
                                state.ui.active_session_id = Some(new_session_id);
                                state.ui.focus = FocusPanel::OutputPane;

                                // Update workspace active worktree
                                if let Some(workspace) = state.selected_workspace_mut() {
                                    workspace.active_worktree_session_id = Some(session_id);
                                }

                                let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
                            }
                            Err(_e) => {
                                // Don't use eprintln! in TUI - it corrupts the display
                                // Session was not added to state, just clean up the buffer
                                state.system.output_buffers.remove(&new_session_id);
                            }
                        }
                    }
                }
            } else {
                // Switching back to main - just clear the active worktree
                if let Some(workspace) = state.selected_workspace_mut() {
                    workspace.active_worktree_session_id = None;
                    let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
                }
            }
        }
        Action::EnterCreateSessionMode => {
            if state.selected_workspace().is_some() {
                state.ui.input_mode = InputMode::CreateSession;
            }
        }
        Action::EnterSetStartCommandMode => {
            let session_info = state.selected_session()
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
            let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
        }
        Action::PinSession(session_id) => {
            let ws_idx = state.ui.selected_workspace_idx;
            if ws_idx < state.data.workspaces.len() {
                let pinned = state.data.workspaces[ws_idx].pin_terminal(session_id);
                if pinned {
                    state.ui.split_view_enabled = true;
                    let new_idx = state.data.workspaces[ws_idx].pinned_terminal_ids.len().saturating_sub(1);
                    state.ui.focused_pinned_pane = new_idx;
                    state.ui.pinned_content_lengths[new_idx] = 0; // Reset length for stabilization
                    resize_ptys_to_panes(state);
                    let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
                }
            }
        }
        Action::UnpinSession(session_id) => {
            if let Some(ws) = state.data.workspaces.get_mut(state.ui.selected_workspace_idx) {
                ws.unpin_terminal(session_id);
                let count = ws.pinned_terminal_ids.len();
                if state.ui.focused_pinned_pane >= count && count > 0 {
                    state.ui.focused_pinned_pane = count - 1;
                }
                state.ui.pinned_content_lengths = [0; crate::models::MAX_PINNED_TERMINALS]; // Reset all lengths on shift
                resize_ptys_to_panes(state);
                let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
            }
        }
        Action::UnpinFocusedSession => {
            let session_id = state.pinned_terminal_id_at(state.ui.focused_pinned_pane);
            if let (Some(ws), Some(sid)) = (state.data.workspaces.get_mut(state.ui.selected_workspace_idx), session_id) {
                ws.unpin_terminal(sid);
                let count = ws.pinned_terminal_ids.len();
                if state.ui.focused_pinned_pane >= count && count > 0 {
                    state.ui.focused_pinned_pane = count - 1;
                }
                if count == 0 {
                    state.ui.focus = FocusPanel::SessionList;
                }
                state.ui.pinned_content_lengths = [0; crate::models::MAX_PINNED_TERMINALS]; // Reset all lengths on shift
                resize_ptys_to_panes(state);
                let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
            }
        }
        Action::ToggleSplitView => {
            state.ui.split_view_enabled = !state.ui.split_view_enabled;
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
            let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
        }
        Action::PtyOutput(session_id, data) => {
            if let Some(parser) = state.system.output_buffers.get_mut(&session_id) {
                parser.process(&data);
            }
            state.data.last_activity.insert(session_id, std::time::Instant::now());
        }
        Action::SendInput(session_id, data) => {
            if let Some(handle) = state.system.pty_handles.get_mut(&session_id) {
                let _ = handle.send_input(&data);
            }
            if let Some(workspace_id) = state.workspace_id_for_session(session_id) {
                if let Some(ws) = state.data.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    ws.touch();
                }
            }
        }
        _ => {} // This is a catch-all for any other Action variants not explicitly handled.
    }
    Ok(())
}
