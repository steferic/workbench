//! Feeding each agent's TODO queue to it, one item per turn.
//!
//! The hard part is not sending text — it is knowing *when*. Workbench used to
//! answer that with output timing, and an agent that had gone quiet to ask a
//! question looked exactly like one that had finished. Lifecycle hooks answer
//! it properly (see `crate::agent_status`), so the queue can wait for a turn
//! to actually end.
//!
//! Three things hold an item back, in order of how annoying it would be to get
//! wrong:
//!
//! 1. The agent is blocked on you. Sending now buries the question under a
//!    new instruction, and the agent cannot read it anyway.
//! 2. You are talking to it. If you typed into that session recently, the
//!    queue waits rather than interleaving with your conversation.
//! 3. Something of ours is already in flight. One item at a time is the point.

use chrono::Utc;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::agent_status::Activity;
use crate::app::{Action, AppState};
use crate::models::SessionStatus;

/// How long the queue keeps quiet after you type into a session.
///
/// Long enough that a follow-up ("no, do it the other way") is not cut off by
/// a queued item; short enough that the queue does not stall after you glance
/// at an agent.
pub const USER_GRACE: Duration = Duration::from_secs(30);

/// Why the queue is not sending anything right now — worth showing, because a
/// queue that silently does nothing is indistinguishable from a broken one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holding {
    /// Nothing pending.
    Empty,
    /// An item is with the agent.
    Running,
    /// You paused it.
    Paused,
    /// The agent is stopped and waiting on you.
    AgentBlocked,
    /// The agent is mid-turn on something else.
    AgentBusy,
    /// You typed into this session a moment ago.
    YouWereTyping,
    /// The session is not running.
    NotRunning,
}

impl Holding {
    pub fn label(&self) -> &'static str {
        match self {
            Holding::Empty => "",
            Holding::Running => "running",
            Holding::Paused => "paused",
            Holding::AgentBlocked => "waiting — agent needs you",
            Holding::AgentBusy => "waiting — agent is busy",
            Holding::YouWereTyping => "waiting — you are typing",
            Holding::NotRunning => "waiting — agent not running",
        }
    }
}

/// What the queue for `session_id` is doing, and whether an item may go now.
pub fn holding(state: &AppState, session_id: Uuid) -> Holding {
    let Some(session) = state.get_session(session_id) else {
        return Holding::Empty;
    };
    let queue = &session.todo_queue;

    if queue.running().is_some() {
        return Holding::Running;
    }
    if queue.next_pending().is_none() {
        return Holding::Empty;
    }
    if queue.paused {
        return Holding::Paused;
    }
    if session.status != SessionStatus::Running {
        return Holding::NotRunning;
    }
    match state.activity(session_id) {
        Activity::NeedsAttention(_) => return Holding::AgentBlocked,
        Activity::Working => return Holding::AgentBusy,
        Activity::Exited => return Holding::NotRunning,
        Activity::Idle => {}
    }
    if typed_recently(state, session_id) {
        return Holding::YouWereTyping;
    }
    Holding::Empty
}

fn typed_recently(state: &AppState, session_id: Uuid) -> bool {
    state
        .data
        .last_send_input
        .get(&session_id)
        .map(|at| at.elapsed() < USER_GRACE)
        .unwrap_or(false)
}

/// Send anything that is ready, and retire anything that has finished.
///
/// Called on the tick. Cheap: it only looks at sessions that have a queue.
pub fn tick(state: &mut AppState, action_tx: &mpsc::UnboundedSender<Action>) {
    let queued: Vec<Uuid> = state
        .data
        .sessions
        .values()
        .flatten()
        .filter(|session| !session.todo_queue.is_empty())
        .map(|session| session.id)
        .collect();

    for session_id in queued {
        retire_finished(state, session_id, action_tx);
        dispatch_next(state, session_id, action_tx);
    }

    wake_idle_managers(state);

    // Records from before the review lifecycle existed — approved, finished,
    // and no verdict — sat in the pane saying "queued" forever, because a
    // proposal without an approved check finished into nothing at all. Hand
    // each one to its manager the way new work now is.
    migrate_stranded(state, action_tx);
}

/// The manager's heartbeat: an idle manager in a project with active
/// objectives gets woken to reassess on its own, so the user is no longer
/// its scheduler. Budgeted twice over — a minimum idle gap and a daily cap —
/// because a heartbeat is a standing spend of real turns.
fn wake_idle_managers(state: &mut AppState) {
    use std::time::{Duration, Instant};

    let every = state.system.user_config.manager_wake_minutes;
    if every == 0 {
        return;
    }
    let cap = state.system.user_config.manager_wake_daily_cap;
    let gap = Duration::from_secs(every * 60);
    let today = chrono::Local::now().date_naive();

    let candidates: Vec<(Uuid, Uuid)> = state
        .data
        .workspaces
        .iter()
        .filter(|ws| {
            ws.objectives
                .iter()
                .any(|o| o.state == crate::models::ObjectiveState::Active)
        })
        .flat_map(|ws| {
            state
                .data
                .sessions
                .get(&ws.id)
                .into_iter()
                .flatten()
                .filter(|s| {
                    s.agent_type.is_manager() && s.status == SessionStatus::Running
                })
                .map(|s| (ws.id, s.id))
        })
        .collect();

    for (_ws, manager_id) in candidates {
        // Only a truly idle manager: no queue, no running item, agent free.
        let idle = state
            .get_session(manager_id)
            .map(|s| {
                s.todo_queue.running().is_none() && s.todo_queue.next_pending().is_none()
            })
            .unwrap_or(false)
            && state.activity(manager_id).is_free();
        if !idle {
            continue;
        }
        let entry = state
            .system
            .manager_wakes
            .entry(manager_id)
            .or_insert((today, 0, Instant::now()));
        if entry.0 != today {
            *entry = (today, 0, entry.2);
        }
        if entry.1 >= cap || entry.2.elapsed() < gap {
            // A fresh entry starts its clock now, so a newly created manager
            // is not woken the moment it boots.
            continue;
        }
        entry.1 += 1;
        entry.2 = Instant::now();
        let wake = entry.1;

        if let Some(session) = state.get_session_mut(manager_id) {
            session.todo_queue.add(
                "HEARTBEAT — reassess on your own initiative.\n\n\
Read the objectives, what the agents are doing, and what changed since you \
last looked. Then either propose the next concrete piece of work \
(manager.propose), or state in one line that nothing is needed right now and \
why. Do not repeat proposals that are pending or in flight."
                    .to_string(),
            );
            crate::logger::info(format!(
                "heartbeat: woke manager {} (wake {wake} today)",
                crate::models::Session::short_id_of(manager_id)
            ));
        }
    }
}

/// One-time repair, run cheaply on the tick: an approved proposal whose
/// queued item is done (or gone) but whose review never started gets moved to
/// AwaitingReview and its manager gets a review turn.
fn migrate_stranded(state: &mut AppState, action_tx: &mpsc::UnboundedSender<Action>) {
    use crate::models::TodoState;

    let mut stranded: Vec<(Uuid, Uuid)> = Vec::new();
    for workspace in &state.data.workspaces {
        for proposal in &workspace.proposals {
            if proposal.state != crate::models::ProposalState::Approved
                || proposal.review.is_some()
            {
                continue;
            }
            let in_flight = proposal.todo_id.is_some_and(|todo| {
                state.data.sessions.values().flatten().any(|session| {
                    session
                        .todo_queue
                        .items
                        .iter()
                        .any(|item| item.id == todo && item.state != TodoState::Done)
                })
            });
            if !in_flight {
                stranded.push((workspace.id, proposal.id));
            }
        }
    }
    for (workspace_id, proposal_id) in stranded {
        crate::logger::info(format!(
            "migrating a stranded proposal {proposal_id} to the review lifecycle"
        ));
        crate::app::handlers::tasks::finish_into_review(
            state,
            workspace_id,
            proposal_id,
            action_tx,
        );
    }
}

/// An item is finished when the agent's turn ends after we sent it.
///
/// A session that died mid-item gives the work back instead of losing it.
fn retire_finished(
    state: &mut AppState,
    session_id: Uuid,
    action_tx: &mpsc::UnboundedSender<Action>,
) {
    let Some(session) = state.get_session(session_id) else {
        return;
    };
    let Some(running) = session.todo_queue.running() else {
        return;
    };
    let sent_at = running.sent_at;
    let alive = session.status == SessionStatus::Running;

    if !alive {
        if let Some(session) = state.get_session_mut(session_id) {
            session.todo_queue.requeue_running();
        }
        return;
    }

    // The turn we started has to have begun before it can end: without this,
    // the idle state left over from *before* dispatch would retire the item
    // the instant it went out.
    let started = sent_at
        .map(|at| Utc::now() - at > chrono::TimeDelta::seconds(2))
        .unwrap_or(true);
    if !started {
        return;
    }

    if state.activity(session_id).is_free() {
        let finished = state
            .get_session_mut(session_id)
            .and_then(|session| session.todo_queue.finish_running());
        // A finished item that came from a proposal is the moment to ask the
        // check what it says now. The manager does not get to request this and
        // cannot skip it: the turn ending is what triggers it.
        if let Some(todo_id) = finished {
            verify_finished_work(state, session_id, todo_id, action_tx);
        }
    }
}

/// Kick off the result run for whichever proposal produced this queued item.
fn verify_finished_work(
    state: &mut AppState,
    session_id: Uuid,
    todo_id: Uuid,
    action_tx: &mpsc::UnboundedSender<Action>,
) {
    let Some(workspace_id) = state.workspace_id_for_session(session_id) else {
        return;
    };
    let Some(proposal) = state
        .data
        .workspaces
        .iter()
        .find(|ws| ws.id == workspace_id)
        .and_then(|ws| {
            ws.proposals
                .iter()
                .find(|p| p.todo_id == Some(todo_id))
                .cloned()
        })
    else {
        return; // ordinary queued work, not something a manager proposed
    };
    if let Some((check, dir, workspace_id)) =
        crate::app::handlers::tasks::baseline_for(state, &proposal, session_id)
    {
        // The check runs first; the manager's review turn follows once the
        // verdict lands, so the packet carries it.
        crate::app::handlers::tasks::start_verification(
            state,
            workspace_id,
            proposal.id,
            false,
            check,
            dir,
            action_tx,
        );
        if let Some(stored) = state
            .data
            .workspaces
            .iter_mut()
            .find(|ws| ws.id == workspace_id)
            .and_then(|ws| ws.proposals.iter_mut().find(|p| p.id == proposal.id))
        {
            stored.work_finished();
        }
        return;
    }
    // No approved check: this used to end here, recording nothing — the pane
    // then said "queued" until the end of time. The manager reviews it
    // instead, with the repo marks standing in for the verdict.
    crate::app::handlers::tasks::finish_into_review(state, workspace_id, proposal.id, action_tx);
}

fn dispatch_next(state: &mut AppState, session_id: Uuid, action_tx: &mpsc::UnboundedSender<Action>) {
    if holding(state, session_id) != Holding::Empty {
        return;
    }
    let Some(session) = state.get_session(session_id) else {
        return;
    };
    let Some(next) = session.todo_queue.next_pending() else {
        return;
    };
    let (id, text) = (next.id, next.text.clone());

    if let Some(session) = state.get_session_mut(session_id) {
        session.todo_queue.mark_running(id);
    }
    send(action_tx, session_id, &text);
    // Sending counts as activity, so the agent is not read as idle again
    // before its hooks report the new turn.
    state
        .data
        .last_activity
        .insert(session_id, std::time::Instant::now());
}

fn send(action_tx: &mpsc::UnboundedSender<Action>, session_id: Uuid, text: &str) {
    crate::app::agent_input::submit_text(action_tx, session_id, text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_status::{AgentStatus, Attention};
    use crate::models::{AgentType, Session, Workspace};

    fn state_with_queue(items: &[&str]) -> (AppState, Uuid) {
        let mut state = AppState::default();
        let workspace = Workspace::new("w".into(), std::path::PathBuf::from("/tmp/w"));
        let workspace_id = workspace.id;
        let mut session = Session::new(workspace_id, AgentType::Claude, false);
        for item in items {
            session.todo_queue.add(*item);
        }
        let session_id = session.id;
        state.data.workspaces.push(workspace);
        state.data.sessions.insert(workspace_id, vec![session]);
        (state, session_id)
    }

    fn report(state: &mut AppState, id: Uuid, activity: Activity) {
        state.system.agent_status.insert(
            id,
            AgentStatus {
                activity,
                reason: String::new(),
                at: Utc::now(),
                event: "test".into(),
                transcript: None,
                model: None,
            },
        );
    }

    fn queue(state: &AppState, id: Uuid) -> &crate::models::TodoQueue {
        &state.get_session(id).unwrap().todo_queue
    }

    #[test]
    fn an_idle_agent_gets_the_next_item() {
        let (mut state, id) = state_with_queue(&["fix the redirect", "write the migration"]);
        report(&mut state, id, Activity::Idle);
        let (tx, mut rx) = mpsc::unbounded_channel();

        tick(&mut state, &tx);

        assert_eq!(queue(&state, id).running().unwrap().text, "fix the redirect");
        // Text as a paste, then Enter (see `agent_input`).
        match rx.try_recv().unwrap() {
            Action::SendInput(sent_to, bytes) => {
                assert_eq!(sent_to, id);
                assert_eq!(
                    String::from_utf8(bytes).unwrap(),
                    "\x1b[200~fix the redirect\x1b[201~"
                );
            }
            other => panic!("expected input, got {other:?}"),
        }
        assert!(matches!(rx.try_recv().unwrap(), Action::SendInput(_, b) if b == vec![b'\r']));
    }

    /// The failure that would make this feature worse than useless: talking
    /// over an agent that stopped to ask you something.
    #[test]
    fn a_blocked_agent_is_never_sent_the_next_item() {
        let (mut state, id) = state_with_queue(&["do the thing"]);
        report(
            &mut state,
            id,
            Activity::NeedsAttention(Attention::Permission),
        );
        let (tx, mut rx) = mpsc::unbounded_channel();

        tick(&mut state, &tx);

        assert_eq!(holding(&state, id), Holding::AgentBlocked);
        assert!(queue(&state, id).running().is_none());
        assert!(rx.try_recv().is_err(), "nothing was sent");
    }

    #[test]
    fn the_queue_waits_while_you_are_typing_to_that_agent() {
        let (mut state, id) = state_with_queue(&["do the thing"]);
        report(&mut state, id, Activity::Idle);
        state
            .data
            .last_send_input
            .insert(id, std::time::Instant::now());
        let (tx, mut rx) = mpsc::unbounded_channel();

        tick(&mut state, &tx);

        assert_eq!(holding(&state, id), Holding::YouWereTyping);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_busy_agent_keeps_its_current_turn() {
        let (mut state, id) = state_with_queue(&["do the thing"]);
        report(&mut state, id, Activity::Working);
        let (tx, mut rx) = mpsc::unbounded_channel();

        tick(&mut state, &tx);

        assert_eq!(holding(&state, id), Holding::AgentBusy);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_paused_queue_holds_everything() {
        let (mut state, id) = state_with_queue(&["do the thing"]);
        report(&mut state, id, Activity::Idle);
        state.get_session_mut(id).unwrap().todo_queue.paused = true;
        let (tx, mut rx) = mpsc::unbounded_channel();

        tick(&mut state, &tx);

        assert_eq!(holding(&state, id), Holding::Paused);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn only_one_item_is_in_flight_at_a_time() {
        let (mut state, id) = state_with_queue(&["first", "second"]);
        report(&mut state, id, Activity::Idle);
        let (tx, _rx) = mpsc::unbounded_channel();

        tick(&mut state, &tx);
        // Still idle by report, but the first item has not finished its turn
        // yet — the second must not go out on top of it.
        tick(&mut state, &tx);

        assert_eq!(queue(&state, id).running().unwrap().text, "first");
        assert_eq!(queue(&state, id).pending_count(), 1);
    }

    #[test]
    fn an_item_is_retired_when_the_turn_it_started_ends() {
        let (mut state, id) = state_with_queue(&["first", "second"]);
        report(&mut state, id, Activity::Idle);
        let (tx, _rx) = mpsc::unbounded_channel();
        tick(&mut state, &tx);

        // Backdate the send so the "did the turn actually start" guard passes,
        // then report the turn ending.
        if let Some(item) = state
            .get_session_mut(id)
            .unwrap()
            .todo_queue
            .items
            .first_mut()
        {
            item.sent_at = Some(Utc::now() - chrono::TimeDelta::seconds(10));
        }
        report(&mut state, id, Activity::Idle);

        tick(&mut state, &tx);

        let queue = queue(&state, id);
        assert_eq!(queue.items[0].state, crate::models::TodoState::Done);
        assert_eq!(queue.running().unwrap().text, "second");
    }

    /// Dispatch and completion both look like "idle" from the outside, so a
    /// freshly sent item must not be retired before its turn even begins.
    #[test]
    fn a_just_sent_item_is_not_retired_immediately() {
        let (mut state, id) = state_with_queue(&["first", "second"]);
        report(&mut state, id, Activity::Idle);
        let (tx, _rx) = mpsc::unbounded_channel();

        tick(&mut state, &tx);
        tick(&mut state, &tx);

        assert_eq!(
            queue(&state, id).running().unwrap().text,
            "first",
            "the item must survive the tick right after it was sent"
        );
    }

    #[test]
    fn a_session_that_stops_mid_item_gives_the_work_back() {
        let (mut state, id) = state_with_queue(&["important work"]);
        report(&mut state, id, Activity::Idle);
        let (tx, _rx) = mpsc::unbounded_channel();
        tick(&mut state, &tx);
        assert!(queue(&state, id).running().is_some());

        state.get_session_mut(id).unwrap().status = SessionStatus::Stopped;
        tick(&mut state, &tx);

        let queue = queue(&state, id);
        assert!(queue.running().is_none());
        assert_eq!(queue.pending_count(), 1, "the item is back in the queue");
    }

    /// The rows that started all this: approved before the review lifecycle
    /// existed, finished, and displayed as "queued" until the end of time.
    /// One tick moves them into review and hands their manager a turn.
    #[test]
    fn stranded_proposals_migrate_into_review() {
        let (mut state, worker_id) = state_with_queue(&["old work"]);
        let workspace_id = state.data.workspaces[0].id;
        let mut manager = Session::new(workspace_id, AgentType::Claude.as_manager(), false);
        manager.status = crate::models::SessionStatus::Running;
        let manager_id = manager.id;
        let manager_short = Session::short_id_of(manager_id);
        state
            .data
            .sessions
            .get_mut(&workspace_id)
            .unwrap()
            .push(manager);

        // An approved proposal whose queued item is long done, from before
        // the `review` field existed.
        let todo_id = {
            let session = state.get_session_mut(worker_id).unwrap();
            let id = session.todo_queue.items[0].id;
            session.todo_queue.items[0].state = crate::models::TodoState::Done;
            id
        };
        let mut proposal = crate::models::Proposal::new(manager_short, "old work");
        proposal.agent = Some(Session::short_id_of(worker_id));
        proposal.state = crate::models::ProposalState::Approved;
        proposal.todo_id = Some(todo_id);
        proposal.review = None; // the pre-lifecycle shape
        let proposal_id = proposal.id;
        state.data.workspaces[0].proposals.push(proposal);

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        tick(&mut state, &tx);

        let migrated = state.data.workspaces[0]
            .proposals
            .iter()
            .find(|p| p.id == proposal_id)
            .unwrap();
        assert!(migrated.awaiting_review(), "no longer stranded at queued");
        let turn = state
            .get_session(manager_id)
            .unwrap()
            .todo_queue
            .items
            .last()
            .unwrap();
        assert!(turn.text.contains("REVIEW TURN"), "{}", turn.text);
    }

    /// The heartbeat wakes an idle manager in a project with active
    /// objectives — once per gap, capped per day, and never the moment it
    /// boots.
    #[test]
    fn an_idle_manager_is_woken_on_the_heartbeat_budget() {
        let mut state = AppState::default();
        let mut ws = Workspace::new("w".into(), std::path::PathBuf::from("/tmp/w"));
        ws.objectives.push(crate::models::Objective::new("stay green"));
        let ws_id = ws.id;
        let mut manager = Session::new(ws_id, AgentType::Claude.as_manager(), false);
        manager.status = crate::models::SessionStatus::Running;
        manager.todo_queue.items.clear(); // drop the boot brief; truly idle
        let manager_id = manager.id;
        state.data.workspaces.push(ws);
        state.data.sessions.insert(ws_id, vec![manager]);
        report(&mut state, manager_id, Activity::Idle);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        // First sight only starts the clock: booting is not idling.
        tick(&mut state, &tx);
        assert!(queue(&state, manager_id).items.is_empty(), "no wake at boot");

        // Pretend the gap has long passed.
        let gap = std::time::Duration::from_secs(
            state.system.user_config.manager_wake_minutes * 60 * 2,
        );
        if let Some(entry) = state.system.manager_wakes.get_mut(&manager_id) {
            entry.2 = std::time::Instant::now() - gap;
        }
        tick(&mut state, &tx);
        let items = &queue(&state, manager_id).items;
        assert_eq!(items.len(), 1, "one wake");
        assert!(items[0].text.contains("HEARTBEAT"), "{}", items[0].text);

        // The wake is in the queue, so the manager is no longer idle — and
        // even once idle again, the gap holds.
        tick(&mut state, &tx);
        assert_eq!(queue(&state, manager_id).items.len(), 1, "no double wake");

        // The daily cap is a hard stop however long the gap.
        if let Some(entry) = state.system.manager_wakes.get_mut(&manager_id) {
            entry.1 = state.system.user_config.manager_wake_daily_cap;
            entry.2 = std::time::Instant::now() - gap;
        }
        state
            .get_session_mut(manager_id)
            .unwrap()
            .todo_queue
            .items
            .clear();
        tick(&mut state, &tx);
        assert!(
            queue(&state, manager_id).items.is_empty(),
            "capped for the day"
        );
    }
}
