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
        retire_finished(state, session_id);
        dispatch_next(state, session_id, action_tx);
    }
}

/// An item is finished when the agent's turn ends after we sent it.
///
/// A session that died mid-item gives the work back instead of losing it.
fn retire_finished(state: &mut AppState, session_id: Uuid) {
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
        if let Some(session) = state.get_session_mut(session_id) {
            session.todo_queue.finish_running();
        }
    }
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
    let bytes: Vec<u8> = text.bytes().collect();
    if action_tx.send(Action::SendInput(session_id, bytes)).is_err() {
        return;
    }
    let _ = action_tx.send(Action::SendInput(session_id, vec![b'\r']));
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
        // Text, then Enter.
        match rx.try_recv().unwrap() {
            Action::SendInput(sent_to, bytes) => {
                assert_eq!(sent_to, id);
                assert_eq!(String::from_utf8(bytes).unwrap(), "fix the redirect");
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
}
