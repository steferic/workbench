//! Workbench from your phone, over the tailnet.
//!
//! An agent that stops for a permission prompt while you are away is dead time
//! until you are back at the desk. This serves a small page — bound to the
//! Tailscale address, never `0.0.0.0` and never Funnel — that shows every
//! agent across every project, lets you queue work, and lets you unblock an
//! agent with a tap.
//!
//! Two directions, both through machinery that already exists:
//!
//! ```text
//! reads   tick → publish(Snapshot) → Arc<Mutex<..>> → GET /api/state
//! writes  POST → Action channel → the same path the TUI's own keys take
//! ```
//!
//! The server never touches `AppState`: it reads a snapshot the event loop
//! publishes, and asks for changes by sending actions. So there is no lock
//! held across a request, and nothing here can corrupt app state.

mod page;
mod server;

pub use server::{new_token, Remote, RemoteCommand};

use serde::Serialize;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::agent_status::Activity;
use crate::app::{tasks_view, todo_dispatch, AppState};
use crate::models::{SessionStatus, TodoState};

/// What the phone sees. Rebuilt on the tick, small enough to send whole.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Snapshot {
    pub agents: Vec<AgentView>,
    /// Seconds since the epoch, so the page can show staleness if the desktop
    /// goes away mid-session.
    pub at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentView {
    /// Short session id — the address for every write endpoint.
    pub id: String,
    pub project: String,
    pub provider: String,
    /// "blocked" | "working" | "idle" | "stopped"
    pub status: String,
    /// The agent's own words when it is blocked ("needs your permission to…").
    pub reason: Option<String>,
    /// The queued item the agent is on, if any.
    pub running: Option<String>,
    /// That item's steps, as the agent reports them.
    pub steps: Vec<StepView>,
    pub queued: Vec<String>,
    pub paused: bool,
    /// Why the queue is holding, when it is.
    pub holding: Option<String>,
    /// The last few lines of output — carried only for a blocked agent, where
    /// it is the difference between approving blind and approving informed.
    pub tail: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepView {
    pub text: String,
    /// "done" | "doing" | "todo"
    pub state: String,
}

/// Shared handoff point between the event loop and the server thread.
pub type Shared = Arc<Mutex<Snapshot>>;

/// Rebuild the snapshot from app state. Called on the tick; cheap enough to
/// run every time rather than inventing a change signal.
pub fn publish(state: &AppState, shared: &Shared) {
    let mut agents = Vec::new();

    for workspace in &state.data.workspaces {
        let Some(sessions) = state.data.sessions.get(&workspace.id) else {
            continue;
        };
        for session in sessions {
            if !session.agent_type.is_agent() || session.worktree_viewer_for.is_some() {
                continue;
            }
            let activity = state.activity(session.id);
            let status = match (session.status, activity) {
                (SessionStatus::Running, Activity::NeedsAttention(_)) => "blocked",
                (SessionStatus::Running, Activity::Working) => "working",
                (SessionStatus::Running, _) => "idle",
                _ => "stopped",
            };

            let queue = &session.todo_queue;
            let running = queue.running().map(|item| item.text.clone());
            let steps = running
                .is_some()
                .then(|| tasks_view::current_batch(state, session.id))
                .flatten()
                .map(|(_, batch)| {
                    batch
                        .tasks
                        .iter()
                        .map(|task| StepView {
                            text: task.subject.clone(),
                            state: match task.state {
                                crate::agent_tasks::TaskState::Completed => "done",
                                crate::agent_tasks::TaskState::InProgress => "doing",
                                crate::agent_tasks::TaskState::Pending => "todo",
                            }
                            .to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default();

            let holding = match todo_dispatch::holding(state, session.id) {
                todo_dispatch::Holding::Empty | todo_dispatch::Holding::Running => None,
                other => Some(other.label().to_string()),
            };

            agents.push(AgentView {
                id: session.short_id(),
                project: workspace.name.clone(),
                provider: session.agent_type.display_name(),
                status: status.to_string(),
                reason: state.activity_reason(session.id).map(str::to_string),
                running,
                steps,
                queued: queue
                    .items
                    .iter()
                    .filter(|item| item.state == TodoState::Pending)
                    .map(|item| item.text.clone())
                    .collect(),
                paused: queue.paused,
                holding,
                tail: if status == "blocked" {
                    output_tail(state, session.id, 12)
                } else {
                    Vec::new()
                },
            });
        }
    }

    // Blocked agents first: that is the row you opened your phone for.
    agents.sort_by_key(|a| match a.status.as_str() {
        "blocked" => 0,
        "working" => 1,
        "idle" => 2,
        _ => 3,
    });

    if let Ok(mut snapshot) = shared.lock() {
        snapshot.agents = agents;
        snapshot.at = chrono::Utc::now().timestamp();
    }
}

/// Resolve a short id from the phone back to a session.
pub fn session_for(state: &AppState, short_id: &str) -> Option<Uuid> {
    state
        .data
        .sessions
        .values()
        .flatten()
        .find(|session| session.short_id().eq_ignore_ascii_case(short_id))
        .map(|session| session.id)
}

/// The last non-empty lines of an agent's output, for deciding whether to
/// approve without walking to the desk.
fn output_tail(state: &AppState, session_id: Uuid, lines: usize) -> Vec<String> {
    let Some(transcript) = state.system.transcript_buffers.get(&session_id) else {
        return Vec::new();
    };
    let len = transcript.len();
    let mut out: Vec<String> = (len.saturating_sub(lines * 3)..len)
        .filter_map(|i| transcript.line(i))
        .map(|line| line.trim_end().to_string())
        .filter(|line| !line.is_empty())
        .collect();
    if out.len() > lines {
        out.drain(..out.len() - lines);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_status::{AgentStatus, Attention};
    use crate::models::{AgentType, Session, Workspace};

    fn state_with_agents() -> (AppState, Uuid, Uuid) {
        let mut state = AppState::default();
        let workspace = Workspace::new("zeta".into(), std::path::PathBuf::from("/tmp/z"));
        let workspace_id = workspace.id;
        let busy = Session::new(workspace_id, AgentType::Claude, false);
        let blocked = Session::new(workspace_id, AgentType::Codex, false);
        let (busy_id, blocked_id) = (busy.id, blocked.id);
        state.data.workspaces.push(workspace);
        state
            .data
            .sessions
            .insert(workspace_id, vec![busy, blocked]);

        state.system.agent_status.insert(
            busy_id,
            AgentStatus {
                activity: Activity::Working,
                reason: "running Edit".into(),
                at: chrono::Utc::now(),
                event: "PreToolUse".into(),
            },
        );
        state.system.agent_status.insert(
            blocked_id,
            AgentStatus {
                activity: Activity::NeedsAttention(Attention::Permission),
                reason: "wants to run shell".into(),
                at: chrono::Utc::now(),
                event: "PermissionRequest".into(),
            },
        );
        (state, busy_id, blocked_id)
    }

    #[test]
    fn the_blocked_agent_is_first_because_that_is_why_you_looked() {
        let (state, _busy, blocked) = state_with_agents();
        let shared: Shared = Default::default();

        publish(&state, &shared);

        let snapshot = shared.lock().unwrap();
        assert_eq!(snapshot.agents.len(), 2);
        assert_eq!(snapshot.agents[0].status, "blocked");
        assert_eq!(
            snapshot.agents[0].id,
            state.get_session(blocked).unwrap().short_id()
        );
        // The agent's own words travel to the phone, so you can decide
        // without reading the terminal.
        assert_eq!(
            snapshot.agents[0].reason.as_deref(),
            Some("wants to run shell")
        );
        assert_eq!(snapshot.agents[1].status, "working");
    }

    #[test]
    fn the_queue_and_what_it_is_waiting_for_travel_too() {
        let (mut state, busy, _blocked) = state_with_agents();
        let queue = &mut state.get_session_mut(busy).unwrap().todo_queue;
        let first = queue.add("fix the redirect");
        queue.add("write the migration");
        queue.mark_running(first);

        let shared: Shared = Default::default();
        publish(&state, &shared);

        let snapshot = shared.lock().unwrap();
        let agent = snapshot
            .agents
            .iter()
            .find(|a| a.id == state.get_session(busy).unwrap().short_id())
            .unwrap();
        assert_eq!(agent.running.as_deref(), Some("fix the redirect"));
        assert_eq!(agent.queued, vec!["write the migration"]);
    }

    #[test]
    fn short_ids_from_the_phone_resolve_back_to_sessions() {
        let (state, busy, _blocked) = state_with_agents();
        let short = state.get_session(busy).unwrap().short_id();

        assert_eq!(session_for(&state, &short), Some(busy));
        assert_eq!(session_for(&state, &short.to_uppercase()), Some(busy));
        assert_eq!(session_for(&state, "nosuchid"), None);
    }
}
