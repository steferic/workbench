use crate::app::handlers::{report_background_error, save_state};
use crate::app::{Action, AppState, PendingSessionStart, Toast, ToastLevel};
use crate::models::{AgentType, SessionStatus};
use crate::pty::{PtyManager, Resume, SessionSpawnConfig};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use uuid::Uuid;

struct SessionStartRequest<'a> {
    session_id: Uuid,
    workspace_id: Uuid,
    workspace_path: &'a Path,
    agent_type: AgentType,
    dangerously_skip_permissions: bool,
    worktree_path: Option<&'a Path>,
    /// The agent conversation this session owns, if we have learned it.
    provider_session_id: Option<String>,
}

impl<'a> SessionStartRequest<'a> {
    fn effective_dir(&self) -> &'a Path {
        self.worktree_path
            .filter(|path| path.exists())
            .unwrap_or(self.workspace_path)
    }

    /// Restore this session's own history. Falling back to `MostRecent` is
    /// only right when we never learned its conversation id — it resumes
    /// whatever the *directory* touched last, so two agents in one project
    /// would both land on the same conversation.
    fn resume_target(&self) -> Resume {
        if !self.agent_type.is_agent() {
            return Resume::No;
        }
        match &self.provider_session_id {
            Some(id) => Resume::Conversation(id.clone()),
            None => Resume::MostRecent,
        }
    }
}

/// Spawn a single session's PTY and update state.
/// Returns true if the session was started successfully.
fn spawn_single_session(
    state: &mut AppState,
    pty_manager: &PtyManager,
    pty_tx: &mpsc::Sender<Action>,
    request: SessionStartRequest<'_>,
) -> bool {
    let pty_rows = state.pane_rows();
    let cols = state.output_pane_cols();

    state
        .system
        .create_session_buffers(request.session_id, pty_rows, cols, &request.agent_type);

    match pty_manager.spawn_session(SessionSpawnConfig {
        session_id: request.session_id,
        workspace_id: request.workspace_id,
        resume: request.resume_target(),
        agent_type: request.agent_type.clone(),
        working_dir: request.effective_dir(),
        rows: pty_rows,
        cols,
        pty_tx: pty_tx.clone(),
        dangerously_skip_permissions: request.dangerously_skip_permissions,
        use_alternate_screen: state.system.use_alternate_screen,
    }) {
        Ok(handle) => {
            state.system.pty_handles.insert(request.session_id, handle);
            if let Some(session) = state.get_session_mut(request.session_id) {
                session.status = SessionStatus::Running;
            }
            true
        }
        Err(_e) => {
            let duration = std::time::Duration::from_secs(5);
            state.ui.toasts.push_back(Toast::new(
                "Failed to start session on launch".to_string(),
                ToastLevel::Error,
                duration,
            ));
            while state.ui.toasts.len() > 5 {
                state.ui.toasts.pop_front();
            }
            state.system.remove_session_buffers(&request.session_id);
            if let Some(session) = state.get_session_mut(request.session_id) {
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
    let stopped_sessions: Vec<(Uuid, AgentType, bool, Option<PathBuf>, Option<String>)> = state
        .data
        .sessions
        .get(&workspace_id)
        .map(|sessions| {
            sessions
                .iter()
                .filter(|s| matches!(s.status, SessionStatus::Stopped | SessionStatus::Errored))
                .map(|s| {
                    (
                        s.id,
                        s.agent_type.clone(),
                        s.dangerously_skip_permissions,
                        s.worktree_path.clone(),
                        s.provider_session_id.clone(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    if stopped_sessions.is_empty() {
        return;
    }

    // Start each stopped session
    for (session_id, agent_type, dangerously_skip_permissions, worktree_path, provider_session_id) in
        stopped_sessions
    {
        spawn_single_session(
            state,
            pty_manager,
            pty_tx,
            SessionStartRequest {
                session_id,
                workspace_id,
                workspace_path: &workspace_path,
                agent_type,
                dangerously_skip_permissions,
                worktree_path: worktree_path.as_deref(),
                provider_session_id,
            },
        );
    }

    // Touch workspace and save
    if let Some(ws) = state
        .data
        .workspaces
        .iter_mut()
        .find(|ws| ws.id == workspace_id)
    {
        ws.touch();
    }
    save_state(state, "failed to save started workspace sessions");
}

/// Queue the selected workspace's sessions for staggered startup. Other
/// workspaces remain stopped until the user selects them.
pub fn queue_selected_workspace_sessions(state: &mut AppState) {
    let Some((workspace_id, workspace_path)) = state
        .selected_workspace()
        .map(|ws| (ws.id, ws.path.clone()))
    else {
        return;
    };

    let stopped_sessions: Vec<PendingSessionStart> = state
        .data
        .sessions
        .get(&workspace_id)
        .map(|sessions| {
            sessions
                .iter()
                .filter(|s| matches!(s.status, SessionStatus::Stopped | SessionStatus::Errored))
                .map(|s| PendingSessionStart {
                    session_id: s.id,
                    workspace_id,
                    workspace_path: workspace_path.clone(),
                    agent_type: s.agent_type.clone(),
                    start_command: s.start_command.clone(),
                    dangerously_skip_permissions: s.dangerously_skip_permissions,
                    worktree_path: s.worktree_path.clone(),
                    provider_session_id: s.provider_session_id.clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    for pending in stopped_sessions {
        state.system.startup_queue.push_back(pending);
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
        state,
        pty_manager,
        pty_tx,
        SessionStartRequest {
            session_id: pending.session_id,
            workspace_id: pending.workspace_id,
            workspace_path: &pending.workspace_path,
            agent_type: pending.agent_type.clone(),
            dangerously_skip_permissions: pending.dangerously_skip_permissions,
            worktree_path: pending.worktree_path.as_deref(),
            provider_session_id: pending.provider_session_id.clone(),
        },
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
                        if let Err(err) = tx.send(Action::SendInput(sid, input)) {
                            report_background_error("failed to send terminal start command", err);
                        }
                    });
                }
            }
        }

        // Touch workspace
        if let Some(ws) = state
            .data
            .workspaces
            .iter_mut()
            .find(|ws| ws.id == pending.workspace_id)
        {
            ws.touch();
        }
    }

    // Save state after each session start
    if state.system.startup_queue.is_empty() {
        save_state(state, "failed to save startup queue state");
    }

    true
}

#[cfg(test)]
mod tests {
    use super::{queue_selected_workspace_sessions, SessionStartRequest};
    use crate::app::AppState;
    use crate::models::{AgentType, Session, SessionStatus, Workspace};
    use uuid::Uuid;

    #[test]
    fn effective_dir_uses_existing_worktree() {
        let workspace_dir = tempfile::tempdir().unwrap();
        let worktree_dir = tempfile::tempdir().unwrap();
        let request = SessionStartRequest {
            session_id: Uuid::new_v4(),
            workspace_id: Uuid::new_v4(),
            workspace_path: workspace_dir.path(),
            agent_type: AgentType::Claude,
            dangerously_skip_permissions: false,
            worktree_path: Some(worktree_dir.path()),
            provider_session_id: None,
        };

        assert_eq!(request.effective_dir(), worktree_dir.path());
    }

    #[test]
    fn effective_dir_falls_back_when_worktree_is_missing() {
        let workspace_dir = tempfile::tempdir().unwrap();
        let missing_worktree = workspace_dir.path().join("missing-worktree");
        let request = SessionStartRequest {
            session_id: Uuid::new_v4(),
            workspace_id: Uuid::new_v4(),
            workspace_path: workspace_dir.path(),
            agent_type: AgentType::Claude,
            dangerously_skip_permissions: false,
            worktree_path: Some(&missing_worktree),
            provider_session_id: None,
        };

        assert_eq!(request.effective_dir(), workspace_dir.path());
    }

    #[test]
    fn effective_dir_uses_workspace_without_worktree() {
        let workspace_dir = tempfile::tempdir().unwrap();
        let request = SessionStartRequest {
            session_id: Uuid::new_v4(),
            workspace_id: Uuid::new_v4(),
            workspace_path: workspace_dir.path(),
            agent_type: AgentType::Claude,
            dangerously_skip_permissions: false,
            worktree_path: None,
            provider_session_id: None,
        };

        assert_eq!(request.effective_dir(), workspace_dir.path());
    }

    fn workspace_with_stopped_session(state: &mut AppState) -> (Uuid, Uuid) {
        let workspace_dir =
            std::env::temp_dir().join(format!("workbench-session-start-{}", Uuid::new_v4()));
        let workspace = Workspace::from_path(workspace_dir);
        let workspace_id = workspace.id;

        let mut session = Session::new(workspace_id, AgentType::Claude, false);
        session.status = SessionStatus::Stopped;
        let session_id = session.id;

        state.data.workspaces.push(workspace);
        state.data.sessions.insert(workspace_id, vec![session]);
        (workspace_id, session_id)
    }

    #[test]
    fn initial_queue_only_includes_selected_workspace() {
        let mut state = AppState::new();
        let (_, selected_session_id) = workspace_with_stopped_session(&mut state);
        let (_, other_session_id) = workspace_with_stopped_session(&mut state);
        state.ui.selected_workspace_idx = 0;

        queue_selected_workspace_sessions(&mut state);

        let queued_ids: Vec<Uuid> = state
            .system
            .startup_queue
            .iter()
            .map(|pending| pending.session_id)
            .collect();
        assert_eq!(queued_ids, vec![selected_session_id]);
        assert!(!queued_ids.contains(&other_session_id));
    }

}
