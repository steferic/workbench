use crate::app::{Action, AppState, PendingSessionStart, PARSER_BUFFER_ROWS, TERMINAL_SCROLLBACK_LIMIT};
use crate::models::{AgentType, SessionStatus, WorkspaceStatus};
use crate::persistence;
use crate::pty::PtyManager;
use tokio::sync::mpsc;
use uuid::Uuid;

/// Start all stopped sessions in the selected workspace
pub fn start_workspace_sessions(
    state: &mut AppState,
    pty_manager: &PtyManager,
    pty_tx: &mpsc::Sender<Action>,
) {
    // Get workspace info
    let workspace = match state.selected_workspace() {
        Some(ws) => ws,
        None => return,
    };
    let workspace_id = workspace.id;
    let workspace_path = workspace.path.clone();

    // Find all stopped sessions in this workspace
    let stopped_sessions: Vec<(Uuid, AgentType, bool)> = state.data.sessions
        .get(&workspace_id)
        .map(|sessions| {
            sessions.iter()
                .filter(|s| matches!(s.status, SessionStatus::Stopped | SessionStatus::Errored))
                .map(|s| (s.id, s.agent_type.clone(), s.dangerously_skip_permissions))
                .collect()
        })
        .unwrap_or_default();

    if stopped_sessions.is_empty() {
        return;
    }

    // Calculate PTY size (actual pane dimensions) and parser size (larger for history)
    let pty_rows = state.pane_rows();
    let cols = state.output_pane_cols();

    // Start each stopped session
    for (session_id, agent_type, dangerously_skip_permissions) in stopped_sessions {
        // Create vt100 parser with large buffer for scrollback history
        // (PTY uses pane size, but parser is larger to hold history)
        let parser = vt100::Parser::new(PARSER_BUFFER_ROWS, cols, TERMINAL_SCROLLBACK_LIMIT);
        state.system.output_buffers.insert(session_id, parser);

        // Spawn PTY with resume flag for agents (not terminals)
        let resume: bool = agent_type.is_agent();
        match pty_manager.spawn_session_with_resume(
            session_id,
            agent_type,
            &workspace_path,
            pty_rows,
            cols,
            pty_tx.clone(),
            resume,
            dangerously_skip_permissions,
        ) {
            Ok(handle) => {
                state.system.pty_handles.insert(session_id, handle);
                // Mark session as running
                if let Some(session) = state.get_session_mut(session_id) {
                    session.status = SessionStatus::Running;
                }
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

    // Touch workspace and save
    if let Some(ws) = state.data.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
        ws.touch();
    }
    let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
}

/// Queue all sessions in "Working" workspaces for staggered startup
/// Sessions will be started one at a time via process_startup_queue
pub fn start_all_working_sessions(
    state: &mut AppState,
    _pty_manager: &PtyManager,
    _pty_tx: &mpsc::Sender<Action>,
    _action_tx: &mpsc::UnboundedSender<Action>,
) {
    // Get all Working workspace IDs and their paths
    let working_workspaces: Vec<(Uuid, std::path::PathBuf)> = state.data.workspaces.iter()
        .filter(|ws| ws.status == WorkspaceStatus::Working)
        .map(|ws| (ws.id, ws.path.clone()))
        .collect();

    // Queue all stopped sessions for staggered startup
    for (workspace_id, workspace_path) in working_workspaces {
        // Find all stopped sessions in this workspace
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

        // Add to startup queue
        for pending in stopped_sessions {
            state.system.startup_queue.push_back(pending);
        }
    }
}

/// Process one session from the startup queue
/// Call this from the main loop (e.g., on tick) for staggered startup
/// Returns true if a session was started
pub fn process_startup_queue(
    state: &mut AppState,
    pty_manager: &PtyManager,
    pty_tx: &mpsc::Sender<Action>,
    action_tx: &mpsc::UnboundedSender<Action>,
) -> bool {
    let pending = match state.system.startup_queue.pop_front() {
        Some(p) => p,
        None => return false,
    };

    let pty_rows = state.pane_rows();
    let cols = state.output_pane_cols();

    // Create vt100 parser with large buffer for scrollback history
    let parser = vt100::Parser::new(PARSER_BUFFER_ROWS, cols, TERMINAL_SCROLLBACK_LIMIT);
    state.system.output_buffers.insert(pending.session_id, parser);

    // Spawn PTY with resume flag for agents (not terminals)
    let resume: bool = pending.agent_type.is_agent();
    match pty_manager.spawn_session_with_resume(
        pending.session_id,
        pending.agent_type.clone(),
        &pending.workspace_path,
        pty_rows,
        cols,
        pty_tx.clone(),
        resume,
        pending.dangerously_skip_permissions,
    ) {
        Ok(handle) => {
            state.system.pty_handles.insert(pending.session_id, handle);
            // Mark session as running
            if let Some(session) = state.get_session_mut(pending.session_id) {
                session.status = SessionStatus::Running;
            }

            // Send start command for terminals after a short delay
            if pending.agent_type.is_terminal() {
                if let Some(cmd) = pending.start_command {
                    if !cmd.is_empty() {
                        let tx = action_tx.clone();
                        let sid = pending.session_id;
                        tokio::spawn(async move {
                            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
                            let mut input = cmd.into_bytes();
                            input.push(b'\n');
                            let _ = tx.send(Action::SendInput(sid, input));
                        });
                    }
                }
            }

            // Touch workspace
            if let Some(ws) = state.data.workspaces.iter_mut().find(|ws| ws.id == pending.workspace_id) {
                ws.touch();
            }
        }
        Err(_e) => {
            // Mark session as errored so user sees it failed
            state.system.output_buffers.remove(&pending.session_id);
            if let Some(session) = state.get_session_mut(pending.session_id) {
                session.mark_errored();
            }
        }
    }

    // Save state after each session start
    if state.system.startup_queue.is_empty() {
        let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
    }

    true
}
