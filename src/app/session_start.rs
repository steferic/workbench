use crate::app::{Action, AppState};
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

    // Calculate PTY size
    let pty_rows = state.pane_rows();
    let cols = state.output_pane_cols();
    let parser_rows = 500; // Large buffer for scrollback

    // Start each stopped session
    for (session_id, agent_type, dangerously_skip_permissions) in stopped_sessions {
        // Create vt100 parser
        let parser = vt100::Parser::new(parser_rows, cols, 10000);
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
            Err(e) => {
                eprintln!("Failed to start session: {}", e);
                state.system.output_buffers.remove(&session_id);
            }
        }
    }

    // Touch workspace and save
    if let Some(ws) = state.data.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
        ws.touch();
    }
    let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
}

/// Start all sessions in "Working" workspaces on startup
pub fn start_all_working_sessions(
    state: &mut AppState,
    pty_manager: &PtyManager,
    pty_tx: &mpsc::Sender<Action>,
    action_tx: &mpsc::UnboundedSender<Action>,
) {
    // Get all Working workspace IDs and their paths
    let working_workspaces: Vec<(Uuid, std::path::PathBuf)> = state.data.workspaces.iter()
        .filter(|ws| ws.status == WorkspaceStatus::Working)
        .map(|ws| (ws.id, ws.path.clone()))
        .collect();

    // For each working workspace, start all stopped sessions
    for (workspace_id, workspace_path) in working_workspaces {
        // Find all stopped sessions in this workspace (include start_command for terminals)
        let stopped_sessions: Vec<(Uuid, AgentType, Option<String>, bool)> = state.data.sessions
            .get(&workspace_id)
            .map(|sessions| {
                sessions.iter()
                    .filter(|s| matches!(s.status, SessionStatus::Stopped | SessionStatus::Errored))
                    .map(|s| (
                        s.id,
                        s.agent_type.clone(),
                        s.start_command.clone(),
                        s.dangerously_skip_permissions,
                    ))
                    .collect()
            })
            .unwrap_or_default();

        if stopped_sessions.is_empty() {
            continue;
        }

        // Calculate PTY size
        let pty_rows = state.pane_rows();
        let cols = state.output_pane_cols();
        let parser_rows = 500; // Large buffer for scrollback

        // Start each stopped session
        for (session_id, agent_type, start_command, dangerously_skip_permissions) in stopped_sessions {
            // Create vt100 parser
            let parser = vt100::Parser::new(parser_rows, cols, 10000);
            state.system.output_buffers.insert(session_id, parser);

            // Spawn PTY with resume flag for agents (not terminals)
            let resume: bool = agent_type.is_agent();
            match pty_manager.spawn_session_with_resume(
                session_id,
                agent_type.clone(),
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

                    // Send start command for terminals after a short delay
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
                }
                Err(e) => {
                    eprintln!("Failed to auto-start session: {}", e);
                    state.system.output_buffers.remove(&session_id);
                }
            }
        }

        // Touch workspace
        if let Some(ws) = state.data.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
            ws.touch();
        }
    }

    // Save state after starting sessions
    let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
}
