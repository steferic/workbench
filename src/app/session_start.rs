use crate::app::{Action, AppState, PendingSessionStart};
use crate::models::{AgentType, SessionStatus, WorkspaceStatus};
use crate::persistence;
use crate::pty::{PtyManager, SessionSpawnConfig};
use tokio::sync::mpsc;
use uuid::Uuid;

/// Spawn a single session's PTY and update state.
/// Returns true if the session was started successfully.
fn spawn_single_session(
    state: &mut AppState,
    pty_manager: &PtyManager,
    pty_tx: &mpsc::Sender<Action>,
    session_id: Uuid,
    workspace_path: &std::path::Path,
    agent_type: AgentType,
    dangerously_skip_permissions: bool,
) -> bool {
    let pty_rows = state.pane_rows();
    let cols = state.output_pane_cols();

    state.system.create_session_buffers(session_id, cols);

    match pty_manager.spawn_session(SessionSpawnConfig {
        session_id,
        resume: agent_type.is_agent(),
        agent_type,
        working_dir: workspace_path,
        rows: pty_rows,
        cols,
        pty_tx: pty_tx.clone(),
        dangerously_skip_permissions,
    }) {
        Ok(handle) => {
            state.system.pty_handles.insert(session_id, handle);
            if let Some(session) = state.get_session_mut(session_id) {
                session.status = SessionStatus::Running;
            }
            true
        }
        Err(_e) => {
            // Don't use eprintln! in TUI - it corrupts the display
            state.system.remove_session_buffers(&session_id);
            if let Some(session) = state.get_session_mut(session_id) {
                session.mark_errored();
            }
            false
        }
    }
}

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

    // Start each stopped session
    for (session_id, agent_type, dangerously_skip_permissions) in stopped_sessions {
        spawn_single_session(
            state, pty_manager, pty_tx,
            session_id, &workspace_path, agent_type, dangerously_skip_permissions,
        );
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

    if spawn_single_session(
        state, pty_manager, pty_tx,
        pending.session_id, &pending.workspace_path,
        pending.agent_type.clone(), pending.dangerously_skip_permissions,
    ) {
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

    // Save state after each session start
    if state.system.startup_queue.is_empty() {
        let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
    }

    true
}
