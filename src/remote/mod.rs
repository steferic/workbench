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
mod push;
mod server;
mod thread;

pub use prompt::Prompt;
pub use push::{push_origin, Push};
pub use server::{new_token, Remote, RemoteCommand};
pub use thread::{Cursor, Message};

use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::agent_status::Activity;
use crate::agent_tasks::Source;
use crate::app::{tasks_view, todo_dispatch, AppState};
use crate::models::{Session, SessionStatus, TodoState};

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
    /// The agent whose conversation is being published (short id), if any.
    /// Focus lives in workbench memory and restarts as "none", while the
    /// page's idea of it lives in localStorage and survives — without this
    /// field the page cannot tell "no new messages" from "nobody is
    /// publishing my conversation", and an open phone kept polling forever
    /// while its thread silently stopped growing.
    pub open: Option<String>,
    /// Everything waiting on the user, most urgent first, across every
    /// project. The phone's decision list and the number on its badge.
    #[serde(default)]
    pub desk: Vec<DeskRowView>,
    /// Seconds since the epoch, so the page can show staleness if the desktop
    /// goes away mid-session.
    pub at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ObjectiveView {
    pub id: String,
    pub text: String,
    /// "active" | "held" | "met"
    pub state: String,
    /// The approved command that decides this is done, if there is one.
    pub done_when: Option<String>,
    /// A command a manager suggested and you have not approved. Present so a
    /// manager can see it has already asked and does not ask again.
    pub proposed_check: Option<String>,
}

/// A suggestion and what became of it, for whoever is reading state.
#[derive(Debug, Clone, Serialize)]
pub struct ProposalView {
    pub id: String,
    /// Where the work stands after approval: "working" | "rework" |
    /// "in_review" | "resolved" | "needs_user". Absent before approval.
    #[serde(default)]
    pub phase: Option<String>,
    pub objective: Option<String>,
    pub agent: Option<String>,
    pub instruction: String,
    /// "pending" | "approved" | "declined"
    pub state: String,
    /// "verified" | "rejected" | "inconclusive", once a check has run.
    pub verdict: Option<String>,
    /// Why the verdict came out that way, in words.
    pub why: Option<String>,
    /// The last few lines the check printed, when it failed. What a manager
    /// needs to propose something better next time.
    pub tail: Option<String>,
}

/// One decision waiting on the user, as the phone shows it.
///
/// Projected from `desk_view::rows` rather than rebuilt here. The ordering —
/// blocked agents, then reviews the manager punted, then ordinary approvals,
/// then proposed checks — is the contract, and two implementations of "what
/// needs me, most urgent first" would drift the moment either grew a fifth
/// kind. So the phone reads the desk's own list and only dresses it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DeskRowView {
    /// "blocked" | "needs_user" | "pending" | "check". What the row is, and
    /// therefore which buttons it carries and what `id` addresses.
    pub kind: String,
    /// Which project it lives in. A decision does not become less yours for
    /// living in another workspace, but on a phone it does need saying —
    /// there is no selected project here to infer it from.
    pub project: String,
    /// What the phone posts back: an agent's short id for `blocked`, a
    /// proposal id for `pending` and `needs_user`, an objective id for
    /// `check`.
    pub id: String,
    /// The decision, in one line.
    pub title: String,
    /// The context that decision needs, which is the whole reason the row
    /// exists rather than a count: the manager's reasoning, the findings it
    /// punted on, the question on screen.
    pub detail: Option<String>,
    /// Who it concerns, when that is not already the title.
    pub agent: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectView {
    pub id: String,
    pub name: String,
    /// Standing priorities, in priority order. What a manager works toward.
    pub objectives: Vec<ObjectiveView>,
    /// What has been suggested here, and how each turned out.
    pub proposals: Vec<ProposalView>,
    /// Dev servers running in this project, reachable from the phone.
    pub servers: Vec<ServerView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServerView {
    pub port: u16,
    /// The program listening, as the OS names it.
    pub command: String,
    /// Where to tap. The tailnet host with the dev server's own port, so it
    /// is the URL already in your browser with the host swapped.
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentView {
    /// Short session id — the address for every write endpoint.
    pub id: String,
    pub project: String,
    /// Which project to start a sibling agent in.
    pub project_id: String,
    pub provider: String,
    /// Whether this session is a manager. Present so the control socket can
    /// refuse to let one dispatch work while it is still read-only, without
    /// having to reach into app state from the server thread.
    #[serde(default)]
    pub manager: bool,
    /// The name this agent gave itself with `workbench alias`. Unique within
    /// a project, and the only address that stays meaningful across a restart
    /// — session ids do not survive one.
    pub alias: Option<String>,
    /// What the agent is actually answering with — "Opus 5" rather than
    /// "Claude". `None` until it has journalled a turn, and always for a
    /// provider whose store does not record one, so the page falls back to
    /// `provider`. Both agents let you change model mid-session, so this can
    /// change under a running conversation.
    pub model: Option<String>,
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
    ///
    /// A request that says what it already has (`?have=`) gets only the newer
    /// ones; the phone appends them.
    pub messages: Vec<Message>,
    /// How many messages this conversation has produced in total, ever. The
    /// phone sends it back as `have`, and the difference is what it is owed.
    /// Counting rather than indexing means trimming the front of the held
    /// window cannot shift what a message is called.
    #[serde(default)]
    pub msg_total: usize,
    /// The phone is further behind than the window we hold, so what it got is
    /// a replacement rather than a continuation.
    #[serde(default)]
    pub msg_reset: bool,
    /// Which counting life `msg_total` belongs to (see `ThreadCache::epoch`).
    /// A phone quoting `have` from another epoch gets a replacement, not a
    /// splice — its count and ours share no origin.
    #[serde(default)]
    pub msg_epoch: String,
    /// Screen lines, for an agent whose journal workbench cannot read. Worse
    /// than `messages`, and only ever used instead of it.
    pub tail: Vec<String>,
    /// Seconds since this agent last finished a turn worth mentioning, if it
    /// was recent. The service worker reads it to tell "finished" apart from
    /// "needs you" — the push itself carries no payload to say which.
    pub finished_ago: Option<i64>,
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

/// The snapshot as it should go to a client that already has `have` messages
/// of the open conversation.
///
/// Sending all eighty every second is what a phone notices: measured at 36 KB
/// a tick, which is two megabytes a minute of cellular radio to say almost
/// nothing. Usually the answer here is "none, you are up to date".
pub fn since(snapshot: &Snapshot, have: usize, epoch: Option<&str>) -> Snapshot {
    let mut trimmed = snapshot.clone();
    for agent in &mut trimmed.agents {
        if agent.messages.is_empty() {
            continue;
        }
        // A `have` counted in another life is not a position in this one.
        // Totals restart at the window size whenever the cache rebuilds — a
        // workbench restart, a focus flip — while the phone's count survives
        // in a page that stays open for days. Splicing across that boundary
        // is what appended the tail of a conversation again after every
        // restart: the phone held 40, the reborn cache counted its 80-message
        // window, and the "40 it was owed" were ones it already had.
        let same_life = epoch == Some(agent.msg_epoch.as_str());
        let owed = agent.msg_total.saturating_sub(have);
        if !same_life || have > agent.msg_total || owed >= agent.messages.len() {
            // Three clients look alike from here and all need a replacement
            // rather than a splice: one further behind than the window we
            // keep, one with nothing, and one *ahead* of us — quoting a count
            // from a previous process life. `have` lives in a page that stays
            // open for days; restart workbench and every count starts over
            // while the phone still quotes the old one. Draining "nothing
            // owed" at it left its log silently frozen on the pre-restart
            // conversation.
            agent.msg_reset = true;
        } else {
            let drop = agent.messages.len() - owed;
            agent.messages.drain(..drop);
        }
    }
    trimmed
}

/// The dev servers the phone can reach, per project.
///
/// A server bound to every interface needs no forwarder and is listed anyway —
/// it is reachable, which is all the phone cares about. One that binds
/// loopback is listed once it has actually been spliced, so a link never
/// points at something that will not answer.
fn dev_servers(state: &AppState) -> std::collections::HashMap<Uuid, Vec<ServerView>> {
    let mut by_project: std::collections::HashMap<Uuid, Vec<ServerView>> = Default::default();
    let Some(host) = state.system.remote.as_ref().map(|r| r.config.addr.ip()) else {
        return by_project;
    };

    let mut roots: Vec<(PathBuf, Uuid)> = Vec::new();
    for workspace in &state.data.workspaces {
        roots.push((workspace.path.clone(), workspace.id));
        for session in state.data.sessions.get(&workspace.id).into_iter().flatten() {
            if let Some(worktree) = &session.worktree_path {
                roots.push((worktree.clone(), workspace.id));
            }
        }
    }

    for (server, project) in crate::ports::owned_by(&state.system.dev_servers, &roots) {
        if server.port == state.system.user_config.remote_port {
            continue;
        }
        if server.loopback_only && !state.system.forwarded.contains_key(&server.port) {
            continue;
        }
        by_project.entry(project).or_default().push(ServerView {
            port: server.port,
            command: server.command.clone(),
            url: format!("http://{host}:{}", server.port),
        });
    }
    by_project
}

/// Which file holds a session's conversation, and how to read it.
///
/// A live tracker knows; a stopped session has none, because only running ones
/// are resolved to a store. So the path is remembered on the session, and a
/// finished agent stays readable — the journal outlives the process, and being
/// unable to read what an agent did once it stopped is a poor reason to have
/// walked to the desk.
fn journal(state: &AppState, session_id: Uuid) -> Option<(crate::agent_tasks::Provider, PathBuf)> {
    if let Some(tracker) = state.system.agent_tasks.get(&session_id) {
        if let Some(Source::File(path)) = tracker.source() {
            return Some((tracker.provider(), path.clone()));
        }
    }
    let session = state.get_session(session_id)?;
    let provider = crate::agent_tasks::Provider::for_agent(&session.agent_type)?;
    Some((provider, session.journal_path.clone()?))
}

/// The focused agent's conversation, reading only what the agent has appended
/// since the last tick.
///
/// `None` for a provider with no journal we can read; the caller falls back to
/// the terminal.
fn conversation(state: &mut AppState, session_id: Uuid) -> Option<(Vec<Message>, usize, String)> {
    let (provider, path) = journal(state, session_id)?;

    // A cache for another session, or another of its journals, is of no use.
    let mut cache = match state.system.remote_thread.take() {
        Some(cache) if cache.session == session_id && cache.path == path => cache,
        _ => crate::app::ThreadCache {
            session: session_id,
            path: path.clone(),
            cursor: Default::default(),
            messages: Vec::new(),
            total: 0,
            // A new cache is a new counting life — the phone must not splice
            // a `have` from the old one onto totals from this one.
            epoch: Uuid::new_v4().simple().to_string(),
        },
    };
    let before = cache.messages.len();
    cache.cursor = thread::read_more(&path, provider, cache.cursor, &mut cache.messages);
    if cache.messages.len() < before {
        // Fewer messages than we already had means the file shrank underneath
        // the cursor and `read_more` started over from the tail. What it read
        // is a different conversation's worth of history, so the count
        // restarts with it — `+=` here underflowed, and one wrapped total
        // poisoned the phone's `have` into a number that never parses again.
        // A restarted count is a new life too: an epoch change makes the
        // phone replace rather than splice.
        cache.total = cache.messages.len();
        cache.epoch = Uuid::new_v4().simple().to_string();
    } else {
        cache.total += cache.messages.len() - before;
    }
    if cache.messages.len() > MAX_MESSAGES {
        cache.messages.drain(..cache.messages.len() - MAX_MESSAGES);
    }

    let read = (cache.messages.clone(), cache.total, cache.epoch.clone());
    state.system.remote_thread = Some(cache);
    Some(read)
}

fn publish_with(state: &AppState, shared: &Shared, open: Option<(Vec<Message>, usize, String)>) {
    let desk = phone_desk_rows(state);
    let mut agents = Vec::new();
    let servers = dev_servers(state);
    let projects: Vec<ProjectView> = state
        .data
        .workspaces
        .iter()
        .map(|workspace| ProjectView {
            id: workspace.id.to_string(),
            name: workspace.name.clone(),
            objectives: workspace
                .objectives
                .iter()
                .map(|objective| ObjectiveView {
                    id: objective.id.to_string(),
                    text: objective.text.clone(),
                    state: objective.state.label().to_string(),
                    done_when: objective
                        .done_when
                        .as_ref()
                        .filter(|v| !v.proposed)
                        .map(|v| v.command.clone()),
                    proposed_check: objective
                        .done_when
                        .as_ref()
                        .filter(|v| v.proposed)
                        .map(|v| v.command.clone()),
                })
                .collect(),
            proposals: workspace
                .proposals
                .iter()
                .map(|proposal| ProposalView {
                    id: proposal.id.to_string(),
                    phase: proposal.review.map(|phase| {
                        match phase {
                            crate::models::ReviewPhase::Working if proposal.review_rounds > 0 => {
                                "rework"
                            }
                            crate::models::ReviewPhase::Working => "working",
                            crate::models::ReviewPhase::AwaitingReview => "in_review",
                            crate::models::ReviewPhase::Resolved => "resolved",
                            crate::models::ReviewPhase::NeedsUser => "needs_user",
                            crate::models::ReviewPhase::Closed => "closed",
                        }
                        .to_string()
                    }),
                    objective: proposal.objective_id.map(|id| id.to_string()),
                    agent: proposal.agent.clone(),
                    instruction: proposal.instruction.clone(),
                    state: match proposal.state {
                        crate::models::ProposalState::Pending => "pending",
                        crate::models::ProposalState::Approved => "approved",
                        crate::models::ProposalState::Declined => "declined",
                    }
                    .to_string(),
                    verdict: proposal.verdict.as_ref().map(|v| v.label().to_string()),
                    why: proposal.verdict.as_ref().map(|v| v.why().to_string()),
                    tail: proposal
                        .result
                        .as_ref()
                        .filter(|run| !run.outcome.passed())
                        .map(|run| run.tail.clone()),
                })
                .collect(),
            servers: servers.get(&workspace.id).cloned().unwrap_or_default(),
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
            let (status, question) = agent_state(state, session);

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
                manager: session.agent_type.is_manager(),
                alias: session.alias.clone(),
                model: state.session_model(session.id),
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
                    true => open.clone().map(|(msgs, _, _)| msgs).unwrap_or_default(),
                    false => Vec::new(),
                },
                msg_total: match state.system.remote_focus == Some(session.id) {
                    true => open.as_ref().map(|(_, total, _)| *total).unwrap_or(0),
                    false => 0,
                },
                msg_reset: false,
                msg_epoch: match state.system.remote_focus == Some(session.id) {
                    true => open
                        .as_ref()
                        .map(|(_, _, epoch)| epoch.clone())
                        .unwrap_or_default(),
                    false => String::new(),
                },
                // Also the fallback for a session whose journal exists but has
                // nothing in it yet: an agent still booting has said nothing,
                // and an empty screen would look like a broken page.
                tail: match state.system.remote_focus == Some(session.id)
                    && open.as_ref().map(|(msgs, _, _)| msgs.is_empty()).unwrap_or(true)
                {
                    true => output_tail(state, session.id, FALLBACK_TAIL),
                    false => Vec::new(),
                },
                finished_ago: state
                    .system
                    .remote_finished
                    .get(&session.short_id())
                    .map(|at| (chrono::Utc::now() - *at).num_seconds()),
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
        snapshot.desk = desk;
        // Through get_session, not straight off the uuid: a focus left
        // pointing at a deleted session publishes as "none", so the page
        // knows to claim it afresh rather than trusting a ghost.
        snapshot.open = state
            .system
            .remote_focus
            .and_then(|id| state.get_session(id))
            .map(|session| session.short_id());
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

/// Everything waiting on the user, dressed for the phone.
///
/// The list and its order come from `desk_view::rows`, which is what keeps
/// the phone and the desk from disagreeing about what is most urgent; all
/// this adds is the context a row needs to be decided without walking to the
/// desktop. Rows whose subject has vanished between the desk building the
/// list and this reading it are dropped rather than rendered blank.
pub fn phone_desk_rows(state: &AppState) -> Vec<DeskRowView> {
    use crate::app::desk_view::DeskRow;

    let proposal = |workspace_id: Uuid, proposal_id: Uuid| {
        state
            .data
            .workspaces
            .iter()
            .find(|ws| ws.id == workspace_id)
            .and_then(|ws| ws.proposals.iter().find(|p| p.id == proposal_id))
    };

    crate::app::desk_view::rows(state)
        .into_iter()
        .filter_map(|row| match row {
            DeskRow::BlockedAgent {
                session_id,
                project,
            } => {
                let session = state.get_session(session_id)?;
                let who = session
                    .alias
                    .clone()
                    .unwrap_or_else(|| session.agent_type.display_name().to_string());
                // The question itself is the decision context, and it is on
                // screen; the hook's one-liner is the fallback for an agent
                // whose prompt we could not parse.
                let detail = prompt_on_screen(state, session_id)
                    .map(|prompt| prompt.lines.join("\n"))
                    .filter(|lines| !lines.trim().is_empty())
                    .or_else(|| state.activity_reason(session_id).map(str::to_string));
                Some(DeskRowView {
                    kind: "blocked".into(),
                    project,
                    id: session.short_id(),
                    title: format!("{who} is waiting on you"),
                    detail,
                    agent: Some(session.short_id()),
                })
            }
            DeskRow::NeedsUser {
                workspace_id,
                proposal_id,
                project,
            } => {
                let found = proposal(workspace_id, proposal_id)?;
                Some(DeskRowView {
                    kind: "needs_user".into(),
                    project,
                    id: proposal_id.to_string(),
                    title: found.instruction.clone(),
                    // Why the manager could not close it. Without this the
                    // row is a yes/no with nothing to decide on.
                    detail: found.findings.clone(),
                    agent: found.agent.clone(),
                })
            }
            DeskRow::PendingProposal {
                workspace_id,
                proposal_id,
                project,
            } => {
                let found = proposal(workspace_id, proposal_id)?;
                Some(DeskRowView {
                    kind: "pending".into(),
                    project,
                    id: proposal_id.to_string(),
                    title: found.instruction.clone(),
                    detail: (!found.rationale.trim().is_empty())
                        .then(|| found.rationale.clone()),
                    agent: found.agent.clone(),
                })
            }
            DeskRow::ProposedCheck {
                workspace_id,
                objective_id,
                project,
            } => {
                let objective = state
                    .data
                    .workspaces
                    .iter()
                    .find(|ws| ws.id == workspace_id)
                    .and_then(|ws| ws.objectives.iter().find(|o| o.id == objective_id))?;
                let check = objective.done_when.as_ref().filter(|v| v.proposed)?;
                Some(DeskRowView {
                    kind: "check".into(),
                    project,
                    id: objective_id.to_string(),
                    title: check.command.clone(),
                    // Which objective it would be the proof of. Approving a
                    // command without knowing what it decides is not a
                    // decision.
                    detail: Some(objective.text.clone()),
                    agent: None,
                })
            }
        })
        .collect()
}

/// The one word an agent's state reduces to, plus the question that decided
/// it — deciding the first requires finding the second, and both callers want
/// both.
///
/// Shared so the phone, the control socket and a peer reading the roster
/// cannot disagree about whether an agent needs a human. They did: the roster
/// used to derive its own status from the idle queue, which knows only
/// busy-or-not, so an agent stopped on a permission prompt advertised itself
/// to peers as merely busy — as something that would finish on its own.
pub fn agent_state(state: &AppState, session: &Session) -> (&'static str, Option<Prompt>) {
    let live = session.status == SessionStatus::Running;
    // A question on screen outranks the hook: hooks also fire for plain
    // idleness, and a prompt we can read is proof.
    let question = live.then(|| screen_prompt(state, session.id)).flatten();
    let status = match (session.status, state.activity(session.id)) {
        _ if question.is_some() => "blocked",
        (SessionStatus::Running, Activity::NeedsAttention(_)) => "blocked",
        (SessionStatus::Running, Activity::Working) => "working",
        (SessionStatus::Running, _) => "idle",
        _ => "stopped",
    };
    (status, question)
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

    /// A world holding one of every kind of decision, in two projects.
    fn world_with_every_decision() -> AppState {
        use crate::models::{Objective, Proposal, ProposalState, Verification};

        let mut state = AppState::default();
        let mut alpha = Workspace::new("alpha".into(), std::path::PathBuf::from("/tmp/a"));
        let alpha_id = alpha.id;

        let mut objective = Objective::new("keep it green");
        objective.done_when = Some(Verification::proposed("cargo test"));
        alpha.objectives.push(objective);

        let mut pending = Proposal::new("m1", "small fix");
        pending.rationale = "because the log is noisy".into();
        pending.agent = Some("aaaa1111".into());
        let mut parked = Proposal::new("m1", "risky change");
        parked.state = ProposalState::Approved;
        parked.needs_user("could not tell if the migration ran".into());
        alpha.proposals.push(pending);
        alpha.proposals.push(parked);

        let mut agent = Session::new(alpha_id, AgentType::Claude, false);
        agent.status = SessionStatus::Running;
        agent.alias = Some("backend".into());
        let agent_id = agent.id;
        state.data.workspaces.push(alpha);
        state.data.sessions.insert(alpha_id, vec![agent]);
        state.system.agent_status.insert(
            agent_id,
            AgentStatus {
                activity: Activity::NeedsAttention(Attention::Permission),
                reason: "wants to run rm -rf build".into(),
                at: chrono::Utc::now(),
                event: "Notification".into(),
                transcript: None,
                model: None,
            },
        );

        let mut beta = Workspace::new("beta".into(), std::path::PathBuf::from("/tmp/b"));
        beta.proposals.push(Proposal::new("m2", "over here too"));
        state.data.workspaces.push(beta);
        state
    }

    /// Every kind reaches the phone, in the desk's order, carrying the
    /// context the decision needs — a row that is only a title is a yes/no
    /// with nothing to decide on.
    #[test]
    fn phone_desk_carries_all_four_kinds_with_their_context() {
        let state = world_with_every_decision();
        let rows = phone_desk_rows(&state);

        let kinds: Vec<&str> = rows.iter().map(|r| r.kind.as_str()).collect();
        assert_eq!(kinds, ["blocked", "needs_user", "pending", "pending", "check"]);

        let blocked = &rows[0];
        assert_eq!(blocked.project, "alpha");
        assert!(blocked.title.contains("backend"), "{}", blocked.title);
        assert_eq!(
            blocked.detail.as_deref(),
            Some("wants to run rm -rf build"),
            "the question is the whole reason to look"
        );

        let needs_user = &rows[1];
        assert_eq!(needs_user.title, "risky change");
        assert_eq!(
            needs_user.detail.as_deref(),
            Some("could not tell if the migration ran"),
            "why the manager punted is what you are deciding on"
        );

        let pending = &rows[2];
        assert_eq!(pending.title, "small fix");
        assert_eq!(pending.detail.as_deref(), Some("because the log is noisy"));
        assert_eq!(pending.agent.as_deref(), Some("aaaa1111"));

        // Every row names its project — on a phone there is no selected
        // project to infer it from.
        assert_eq!(rows[3].project, "beta");

        let check = &rows[4];
        assert_eq!(check.title, "cargo test", "the command is the decision");
        assert_eq!(
            check.detail.as_deref(),
            Some("keep it green"),
            "and the objective is what it would prove"
        );
    }

    /// The badge counts what `desk` holds, and that is every kind. It used to
    /// count pending proposals alone, so a blocked agent, a punted review and
    /// an unapproved check all read as "nothing waiting".
    #[test]
    fn phone_desk_badge_counts_every_decision_not_just_proposals() {
        let state = world_with_every_decision();
        let rows = phone_desk_rows(&state);
        assert_eq!(rows.len(), 5);

        let pending_only = rows.iter().filter(|r| r.kind == "pending").count();
        assert_eq!(pending_only, 2);
        assert!(
            rows.len() > pending_only,
            "the old badge input would have hidden {} decisions",
            rows.len() - pending_only
        );

        // Nothing waiting is an honest zero, not a stale count.
        assert!(phone_desk_rows(&AppState::default()).is_empty());
    }

    /// The page reads `data.desk` and switches on `kind`. Renaming either
    /// end is a silent break — the sheet renders empty and the badge reads
    /// zero while four things wait — so the wire shape is pinned here.
    #[test]
    fn phone_desk_serializes_under_the_names_the_page_reads() {
        let state = world_with_every_decision();
        let snapshot = Snapshot {
            desk: phone_desk_rows(&state),
            ..Default::default()
        };
        let json = serde_json::to_value(&snapshot).unwrap();
        let rows = json["desk"].as_array().expect("`desk` is what the page reads");
        assert_eq!(rows.len(), 5);
        for row in rows {
            for field in ["kind", "project", "id", "title"] {
                assert!(row[field].is_string(), "every row needs {field}: {row}");
            }
        }
        let kinds: Vec<&str> = rows.iter().map(|r| r["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["blocked", "needs_user", "pending", "pending", "check"]);
        // Optional fields travel as null rather than being absent, so the
        // page's `r.detail ? … : ""` reads the same either way.
        assert!(rows[4]["agent"].is_null(), "a check concerns no agent");
        assert_eq!(rows[0]["agent"].as_str().map(str::len), Some(8));
    }

    /// A row whose subject vanished between the desk listing it and the
    /// phone dressing it is dropped, not rendered as a blank decision.
    #[test]
    fn phone_desk_drops_a_row_whose_check_was_just_approved() {
        let mut state = world_with_every_decision();
        assert_eq!(phone_desk_rows(&state).len(), 5);
        if let Some(check) = state.data.workspaces[0].objectives[0].done_when.as_mut() {
            check.proposed = false;
        }
        let rows = phone_desk_rows(&state);
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|r| r.kind != "check"));
    }

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
                transcript: None,
                model: None,
            },
        );
        state.system.agent_status.insert(
            blocked_id,
            AgentStatus {
                activity: Activity::NeedsAttention(Attention::Permission),
                reason: "wants to run shell".into(),
                at: chrono::Utc::now(),
                event: "PermissionRequest".into(),
                transcript: None,
                model: None,
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

    /// The failure this guards: a stopped agent's history is on disk, but only
    /// running sessions are resolved to a store — so after a workbench restart
    /// the phone offered "this agent is stopped" and nothing else.
    #[test]
    fn a_stopped_agent_still_reads_back_from_its_journal() {
        use std::io::Write;

        let (mut state, busy, _blocked) = state_with_agents();
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"{{"type":"user","message":{{"role":"user","content":"ship the migration"}}}}"#
        )
        .unwrap();
        file.flush().unwrap();

        {
            let session = state.get_session_mut(busy).unwrap();
            session.status = SessionStatus::Stopped;
            session.journal_path = Some(file.path().to_path_buf());
        }
        state.system.remote_focus = Some(busy);

        let shared: Shared = Default::default();
        publish(&mut state, &shared);

        let snapshot = shared.lock().unwrap();
        let short = state.get_session(busy).unwrap().short_id();
        let stopped = snapshot.agents.iter().find(|a| a.id == short).unwrap();
        assert_eq!(stopped.status, "stopped");
        assert_eq!(
            stopped.messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
            vec!["ship the migration"],
            "the journal outlives the process, so the conversation should too"
        );
    }

    /// A phone polls once a second on a cellular radio. Re-sending eighty
    /// messages each time to say nothing new is the difference between two
    /// megabytes a minute and almost none.
    #[test]
    fn a_client_is_sent_only_the_messages_it_does_not_have() {
        let mut snapshot = Snapshot::default();
        let say = |text: &str| Message {
            role: crate::remote::thread::Role::Agent,
            text: text.into(),
            at: None,
        };
        snapshot.agents.push(AgentView {
            id: "ab12cd34".into(),
            project: "workbench".into(),
            project_id: "p".into(),
            provider: "Claude".into(),
            manager: false,
            alias: None,
            model: Some("Opus 5".into()),
            status: "working".into(),
            reason: None,
            running: None,
            steps: Vec::new(),
            queued: Vec::new(),
            paused: false,
            holding: None,
            prompt: None,
            messages: vec![say("one"), say("two"), say("three")],
            msg_total: 3,
            msg_reset: false,
            msg_epoch: "life-1".into(),
            tail: Vec::new(),
            finished_ago: None,
        });

        // Up to date: nothing owed.
        let caught_up = since(&snapshot, 3, Some("life-1"));
        assert!(caught_up.agents[0].messages.is_empty());
        assert!(!caught_up.agents[0].msg_reset);

        // Two behind: the last two, to be appended.
        let behind = since(&snapshot, 1, Some("life-1"));
        assert_eq!(
            behind.agents[0].messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
            vec!["two", "three"]
        );
        assert!(!behind.agents[0].msg_reset);

        // Nothing at all, or further behind than the window we keep: take
        // ours wholesale rather than splicing onto a gap.
        let fresh = since(&snapshot, 0, Some("life-1"));
        assert_eq!(fresh.agents[0].messages.len(), 3);
        assert!(fresh.agents[0].msg_reset);
    }

    /// A phone that knows MORE of the conversation than the server is not
    /// caught up — it is from a previous life. `have` lives in the page, and
    /// the page is a home-screen app that stays open for days; restart
    /// workbench and every count starts over from the tail window while the
    /// phone still quotes the old total. Draining "nothing owed" at it leaves
    /// its log silently frozen on the pre-restart conversation.
    #[test]
    fn a_client_that_is_ahead_of_us_is_reset_not_starved() {
        let mut snapshot = Snapshot::default();
        snapshot.agents.push(AgentView {
            id: "ab12cd34".into(),
            project: "workbench".into(),
            project_id: "p".into(),
            provider: "Claude".into(),
            manager: false,
            alias: None,
            model: None,
            status: "working".into(),
            reason: None,
            running: None,
            steps: Vec::new(),
            queued: Vec::new(),
            paused: false,
            holding: None,
            prompt: None,
            messages: vec![Message {
                role: crate::remote::thread::Role::Agent,
                text: "after the restart".into(),
                at: None,
            }],
            msg_total: 1,
            msg_reset: false,
            msg_epoch: "life-1".into(),
            tail: Vec::new(),
            finished_ago: None,
        });

        let ahead = since(&snapshot, 347, Some("life-1"));
        assert!(
            ahead.agents[0].msg_reset,
            "a client quoting a count we never issued needs a replacement, not silence"
        );
        assert_eq!(ahead.agents[0].messages.len(), 1);
    }

    /// A journal that gets *shorter* under the cursor must not poison the
    /// count. `read_more` copes — it throws its parse away and re-reads the
    /// tail — but the caller then computed `total += new_len - old_len` on
    /// unsigned numbers. One shrink and the total wraps to ~2^64; the page
    /// echoes that back as `?have=1.8e19`, which fails to parse as a usize,
    /// so every poll gets the full window and the phone concatenates the
    /// whole conversation onto itself once a second.
    #[test]
    fn a_journal_replaced_underneath_does_not_poison_the_count() {
        let (mut state, busy, _blocked) = state_with_agents();
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        for i in 0..10 {
            writeln!(
                file,
                r#"{{"type":"user","message":{{"role":"user","content":"m{i}"}}}}"#
            )
            .unwrap();
        }
        file.flush().unwrap();
        state.get_session_mut(busy).unwrap().journal_path = Some(file.path().to_path_buf());
        state.system.remote_focus = Some(busy);

        let shared: Shared = Default::default();
        publish(&mut state, &shared);
        assert_eq!(
            shared
                .lock()
                .unwrap()
                .agents
                .iter()
                .find(|a| a.msg_total > 0)
                .unwrap()
                .msg_total,
            10
        );

        // The same path, replaced with a shorter conversation.
        std::fs::write(
            file.path(),
            concat!(
                r#"{"type":"user","message":{"role":"user","content":"fresh"}}"#,
                "
"
            ),
        )
        .unwrap();
        publish(&mut state, &shared);

        let snapshot = shared.lock().unwrap().clone();
        let agent = snapshot
            .agents
            .iter()
            .find(|a| !a.messages.is_empty())
            .unwrap();
        assert_eq!(
            agent.msg_total, 1,
            "a replaced file is a fresh count, not an overflow"
        );
        assert_eq!(agent.messages.len(), 1);

        // And the phone that held the old count is reset on its next poll.
        assert!(
            since(&snapshot, 10, Some("life-1"))
                .agents
                .iter()
                .find(|a| a.id == agent.id)
                .unwrap()
                .msg_reset
        );
    }

    /// The window is trimmed from the front once it is full, so a message's
    /// position in it changes while `msg_total` does not — which is the whole
    /// reason the phone counts rather than indexes.
    #[test]
    fn trimming_the_window_does_not_disturb_the_count() {
        let (mut state, busy, _blocked) = state_with_agents();
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        for i in 0..MAX_MESSAGES + 5 {
            writeln!(
                file,
                r#"{{"type":"user","message":{{"role":"user","content":"m{i}"}}}}"#
            )
            .unwrap();
        }
        file.flush().unwrap();
        state.get_session_mut(busy).unwrap().journal_path = Some(file.path().to_path_buf());
        state.system.remote_focus = Some(busy);

        let shared: Shared = Default::default();
        publish(&mut state, &shared);
        let snapshot = shared.lock().unwrap().clone();
        let agent = &snapshot.agents.iter().find(|a| a.msg_total > 0).unwrap();

        assert_eq!(agent.messages.len(), MAX_MESSAGES, "the window is capped");
        assert_eq!(agent.msg_total, MAX_MESSAGES + 5, "the count is not");
        assert_eq!(agent.messages.last().unwrap().text, format!("m{}", MAX_MESSAGES + 4));

        // A phone holding all of them is owed nothing, even though the five
        // it holds from the start are no longer in the window. It quotes the
        // epoch it was told, as the page does.
        let short = agent.id.clone();
        let life = agent.msg_epoch.clone();
        let caught_up = since(&snapshot, MAX_MESSAGES + 5, Some(&life));
        let same = caught_up.agents.iter().find(|a| a.id == short).unwrap();
        assert!(same.messages.is_empty() && !same.msg_reset);

        // One behind gets exactly one, not the window.
        let behind = since(&snapshot, MAX_MESSAGES + 4, Some(&life));
        let same = behind.agents.iter().find(|a| a.id == short).unwrap();
        assert_eq!(same.messages.len(), 1);
    }

    /// The bug this whole mechanism exists for: after a workbench restart the
    /// count restarts at the window size, while the phone's `have` survives in
    /// a page that stays open for days. The old splice saw "have 40, total 80,
    /// owed 40" and served 40 messages the phone already held — the visible
    /// symptom was the last page of a conversation repeated once per restart.
    #[test]
    fn a_have_from_another_life_is_replaced_not_spliced() {
        let say = |text: &str| Message {
            role: crate::remote::thread::Role::Agent,
            text: text.into(),
            at: None,
        };
        let mut snapshot = Snapshot::default();
        snapshot.agents.push(AgentView {
            id: "aaaa1111".into(),
            project: "demo".into(),
            project_id: "p".into(),
            provider: "Codex".into(),
            manager: false,
            alias: None,
            model: None,
            status: "idle".into(),
            reason: None,
            running: None,
            steps: Vec::new(),
            queued: Vec::new(),
            paused: false,
            holding: None,
            prompt: None,
            messages: (0..80).map(|i| say(&format!("m{i}"))).collect(),
            msg_total: 80,
            msg_reset: false,
            msg_epoch: "life-2".into(),
            tail: Vec::new(),
            finished_ago: None,
        });

        // The phone holds 40 messages counted in life-1. In life-2's terms it
        // looks merely behind — but its 40 overlap the window it would be
        // served, and splicing would duplicate them.
        let crossed = since(&snapshot, 40, Some("life-1"));
        let agent = &crossed.agents[0];
        assert!(agent.msg_reset, "a foreign have must reset, not splice");
        assert_eq!(agent.messages.len(), 80, "the reset carries the window");

        // The same numbers within one life are an ordinary splice.
        let native = since(&snapshot, 40, Some("life-2"));
        let agent = &native.agents[0];
        assert!(!agent.msg_reset);
        assert_eq!(agent.messages.len(), 40, "owed exactly the newer half");

        // A page that has never seen an epoch (or predates them) resets too:
        // no shared origin, no splice.
        let blank = since(&snapshot, 40, None);
        assert!(blank.agents[0].msg_reset);
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
