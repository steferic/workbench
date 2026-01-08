use crate::app::{Action, AppState, FocusPanel, InputMode, PendingDelete};
use crate::models::{AgentType, Session};
use crate::persistence;
use crate::pty::PtyManager;
use crate::app::pty_ops::resize_ptys_to_panes;
use anyhow::Result;
use tokio::sync::mpsc;

pub fn handle_session_action(
    state: &mut AppState,
    action: Action,
    pty_manager: &PtyManager,
    action_tx: &mpsc::UnboundedSender<Action>,
) -> Result<()> {
    match action {
        Action::CreateSession(agent_type, dangerously_skip_permissions) => {
            if let Some(workspace) = state.selected_workspace() {
                let session = Session::new(
                    workspace.id,
                    agent_type.clone(),
                    dangerously_skip_permissions,
                );
                let session_id = session.id;
                let workspace_path = workspace.path.clone();
                let ws_idx = state.ui.selected_workspace_idx;

                if let Some(ws) = state.data.workspaces.get_mut(ws_idx) {
                    ws.touch();
                }

                let pty_rows = state.pane_rows();
                let cols = state.output_pane_cols();
                let parser_rows = 500;
                let parser = vt100::Parser::new(parser_rows, cols, 10000);
                state.system.output_buffers.insert(session_id, parser);

                match pty_manager.spawn_session(
                    session_id,
                    agent_type,
                    &workspace_path,
                    pty_rows,
                    cols,
                    action_tx.clone(),
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
                    Err(e) => {
                        eprintln!("Failed to spawn session: {}", e);
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
                let parser_rows = 500;
                let parser = vt100::Parser::new(parser_rows, cols, 10000);
                state.system.output_buffers.insert(session_id, parser);

                match pty_manager.spawn_session(
                    session_id,
                    agent_type,
                    &workspace_path,
                    pty_rows,
                    cols,
                    action_tx.clone(),
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
                    Err(e) => {
                        eprintln!("Failed to spawn terminal: {}", e);
                        state.system.output_buffers.remove(&session_id);
                    }
                }
            }
        }
        Action::ActivateSession(session_id) => {
            state.ui.active_session_id = Some(session_id);
            state.ui.output_scroll_offset = 0;
            state.ui.output_content_length = 0;
        }
        Action::RestartSession(session_id) => {
            let session_info = state.data.sessions.values().flatten()
                .find(|s| s.id == session_id)
                .map(|s| (
                    s.agent_type.clone(),
                    s.workspace_id,
                    s.start_command.clone(),
                    s.dangerously_skip_permissions,
                ));

            if let Some((agent_type, workspace_id, start_command, dangerously_skip_permissions)) = session_info {
                let workspace_path = state.data.workspaces.iter()
                    .find(|w| w.id == workspace_id)
                    .map(|w| w.path.clone());

                if let Some(workspace_path) = workspace_path {
                    let pty_rows = state.pane_rows();
                    let cols = state.output_pane_cols();
                    let parser_rows = 500;
                    let parser = vt100::Parser::new(parser_rows, cols, 10000);
                    state.system.output_buffers.insert(session_id, parser);

                    let resume = agent_type.is_agent();

                    match pty_manager.spawn_session_with_resume(
                        session_id,
                        agent_type.clone(),
                        &workspace_path,
                        pty_rows,
                        cols,
                        action_tx.clone(),
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
                        Err(e) => {
                            eprintln!("Failed to restart session: {}", e);
                            state.system.output_buffers.remove(&session_id);
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
            if let Some(mut handle) = state.system.pty_handles.remove(&session_id) {
                let _ = handle.kill();
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
                if let Some(mut handle) = state.system.pty_handles.remove(&session_id) {
                    let _ = handle.kill();
                }
                state.system.output_buffers.remove(&session_id);
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
        Action::SessionExited(session_id, _exit_code) => {
            state.system.pty_handles.remove(&session_id);
            if let Some(session) = state.get_session_mut(session_id) {
                session.mark_stopped();
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
