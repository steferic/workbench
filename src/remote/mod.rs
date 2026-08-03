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
mod prompt;
mod server;
mod thread;

pub use prompt::Prompt;
pub use server::{new_token, Remote, RemoteCommand};
pub use thread::{Cursor, Message};

use serde::Serialize;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::agent_status::Activity;
use crate::agent_tasks::Source;
use crate::app::{tasks_view, todo_dispatch, AppState};
use crate::models::{SessionStatus, TodoState};

/// How far back the open conversation goes. Enough to scroll through the
/// morning; not so much that the snapshot stops being phone-sized.
const MAX_MESSAGES: usize = 80;
/// Screen lines carried for an agent whose journal we cannot read.
const FALLBACK_TAIL: usize = 200;

/// What the phone sees. Rebuilt on the tick, small enough to send whole.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Snapshot {
    /// Every project, including ones with no agents yet — you can start one
    /// there from the phone.
    pub projects: Vec<ProjectView>,
    pub agents: Vec<AgentView>,
    /// Seconds since the epoch, so the page can show staleness if the desktop
    /// goes away mid-session.
    pub at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectView {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentView {
    /// Short session id — the address for every write endpoint.
    pub id: String,
    pub project: String,
    /// Which project to start a sibling agent in.
    pub project_id: String,
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
    /// The question the agent is stopped on, read off its screen. Present
    /// means it is blocked on you, whatever else the status says.
    pub prompt: Option<prompt::Prompt>,
    /// The conversation, for the agent you have open. Read from the agent's
    /// own journal, so it is speech rather than terminal chrome.
    pub messages: Vec<Message>,
    /// Screen lines, for an agent whose journal workbench cannot read. Worse
    /// than `messages`, and only ever used instead of it.
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
///
/// The open conversation is read first, because that is the one step that can
/// touch the disk — everything after it is a read of state already in memory.
pub fn publish(state: &mut AppState, shared: &Shared) {
    let open = state.system.remote_focus.and_then(|id| conversation(state, id));
    publish_with(state, shared, open);
}

/// The focused agent's conversation, reading only what the agent has appended
/// since the last tick.
///
/// `None` for a provider with no journal we can read; the caller falls back to
/// the terminal.
fn conversation(state: &mut AppState, session_id: Uuid) -> Option<Vec<Message>> {
    let tracker = state.system.agent_tasks.get(&session_id)?;
    let provider = tracker.provider();
    let Source::File(path) = tracker.source()? else {
        return None;
    };
    let path = path.clone();

    // A cache for another session, or another of its journals, is of no use:
    // codex forks a fresh rollout on every resume.
    let mut cache = match state.system.remote_thread.take() {
        Some(cache) if cache.session == session_id && cache.path == path => cache,
        _ => crate::app::ThreadCache {
            session: session_id,
            path: path.clone(),
            cursor: Default::default(),
            messages: Vec::new(),
        },
    };
    cache.cursor = thread::read_more(&path, provider, cache.cursor, &mut cache.messages);
    if cache.messages.len() > MAX_MESSAGES {
        cache.messages.drain(..cache.messages.len() - MAX_MESSAGES);
    }

    let messages = cache.messages.clone();
    state.system.remote_thread = Some(cache);
    Some(messages)
}

fn publish_with(state: &AppState, shared: &Shared, open: Option<Vec<Message>>) {
    let mut agents = Vec::new();
    let projects: Vec<ProjectView> = state
        .data
        .workspaces
        .iter()
        .map(|workspace| ProjectView {
            id: workspace.id.to_string(),
            name: workspace.name.clone(),
        })
        .collect();

    for workspace in &state.data.workspaces {
        let Some(sessions) = state.data.sessions.get(&workspace.id) else {
            continue;
        };
        for session in sessions {
            if !session.agent_type.is_agent() || session.worktree_viewer_for.is_some() {
                continue;
            }
            let live = session.status == SessionStatus::Running;
            // A question on screen outranks the hook: hooks also fire for
            // plain idleness, and a prompt we can read is proof.
            let question = live.then(|| screen_prompt(state, session.id)).flatten();
            let activity = state.activity(session.id);
            let status = match (session.status, activity) {
                _ if question.is_some() => "blocked",
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
                project_id: workspace.id.to_string(),
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
                prompt: question,
                // Only the conversation you have open travels, so the snapshot
                // stays phone-sized however many agents are running.
                messages: match state.system.remote_focus == Some(session.id) {
                    true => open.clone().unwrap_or_default(),
                    false => Vec::new(),
                },
                // Also the fallback for a session whose journal exists but has
                // nothing in it yet: an agent still booting has said nothing,
                // and an empty screen would look like a broken page.
                tail: match state.system.remote_focus == Some(session.id)
                    && open.as_ref().map(Vec::is_empty).unwrap_or(true)
                {
                    true => output_tail(state, session.id, FALLBACK_TAIL),
                    false => Vec::new(),
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
        snapshot.projects = projects;
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

/// The question on an agent's current screen, if it is stopped on one.
///
/// Public so answering can check the choice is still the one being offered: a
/// tap crosses the network, and the prompt may have been answered at the desk
/// in the meantime. A stray digit typed into a composer is exactly the kind of
/// thing that made the old buttons feel broken.
pub fn prompt_on_screen(state: &AppState, session_id: Uuid) -> Option<Prompt> {
    screen_prompt(state, session_id)
}

fn screen_prompt(state: &AppState, session_id: Uuid) -> Option<Prompt> {
    let parser = state.system.output_buffers.get(&session_id)?;
    prompt::parse(&parser.screen().contents())
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
        let (mut state, _busy, blocked) = state_with_agents();
        let shared: Shared = Default::default();

        publish(&mut state, &shared);

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
        publish(&mut state, &shared);

        let snapshot = shared.lock().unwrap();
        let agent = snapshot
            .agents
            .iter()
            .find(|a| a.id == state.get_session(busy).unwrap().short_id())
            .unwrap();
        assert_eq!(agent.running.as_deref(), Some("fix the redirect"));
        assert_eq!(agent.queued, vec!["write the migration"]);
    }

    /// With no journal to read — no tracker has resolved a log for this
    /// session — the open conversation still has to show something, so the
    /// terminal stands in for it.
    #[test]
    fn the_open_conversation_falls_back_to_the_terminal() {
        let (mut state, busy, _blocked) = state_with_agents();
        state
            .system
            .create_session_buffers(busy, 24, 80, &AgentType::Claude);
        if let Some(parser) = state.system.output_buffers.get_mut(&busy) {
            parser.process(b"I looked at the migration and it needs a backfill.\r\n");
        }
        state.system.update_transcript_from_screen(busy);
        state.system.remote_focus = Some(busy);

        let shared: Shared = Default::default();
        publish(&mut state, &shared);
        let snapshot = shared.lock().unwrap();

        let short = state.get_session(busy).unwrap().short_id();
        let open = snapshot.agents.iter().find(|a| a.id == short).unwrap();
        assert!(
            open.tail.iter().any(|l| l.contains("needs a backfill")),
            "{:?}",
            open.tail
        );

        // Every other agent's output stays on the desktop: one conversation
        // travels, so the snapshot stays phone-sized however many are running.
        drop(snapshot);
        state.system.remote_focus = None;
        publish(&mut state, &shared);
        let snapshot = shared.lock().unwrap();
        let closed = snapshot.agents.iter().find(|a| a.id == short).unwrap();
        assert!(closed.tail.is_empty() && closed.messages.is_empty());
    }

    /// The failure this guards is the phone showing Approve/Deny for a
    /// question nobody asked — the hook fires for plain idleness too.
    #[test]
    fn a_question_on_screen_is_what_marks_an_agent_blocked() {
        let (mut state, busy, _blocked) = state_with_agents();
        state
            .system
            .create_session_buffers(busy, 24, 80, &AgentType::Claude);
        let short = state.get_session(busy).unwrap().short_id();
        let shared: Shared = Default::default();

        publish(&mut state, &shared);
        let view = |snapshot: &Snapshot| {
            snapshot
                .agents
                .iter()
                .find(|a| a.id == short)
                .cloned()
                .unwrap()
        };
        assert!(view(&shared.lock().unwrap()).prompt.is_none());
        assert_eq!(view(&shared.lock().unwrap()).status, "idle");

        if let Some(parser) = state.system.output_buffers.get_mut(&busy) {
            parser.process(
                b" Bash command\r\n\r\n   rm -rf build\r\n\r\n Do you want to proceed?\r\n \
                  \xe2\x9d\xaf 1. Yes\r\n   2. No\r\n",
            );
        }
        publish(&mut state, &shared);
        let blocked = view(&shared.lock().unwrap());
        assert_eq!(blocked.status, "blocked");
        let prompt = blocked.prompt.expect("the question travels with it");
        assert_eq!(prompt.options.len(), 2);
        assert!(
            prompt.lines.iter().any(|l| l.contains("rm -rf build")),
            "the phone has to show what it is agreeing to: {:?}",
            prompt.lines
        );
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
