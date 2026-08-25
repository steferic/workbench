//! Standing objectives for a project.
//!
//! What a manager works *toward*, as opposed to the TODO queue, which is what
//! an agent has been told to do next. The difference matters: a queue item is
//! consumed, an objective persists until you say otherwise.
//!
//! Deliberately user-owned. A manager may propose work against these and may
//! propose how one would be checked, but never edits the text and never
//! decides an objective is met — an agent that can rewrite its own goals is
//! the drift failure that sank every previous attempt at this, with extra
//! steps.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// How anyone — manager, agent, or you — would know a piece of work landed.
///
/// A command, not a judgement. The whole design rests on the manager being
/// unable to author the verdict: workbench runs this itself and records what
/// it exited with, so "it passes" is a fact rather than a report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verification {
    /// Run through a shell in the project (or a worktree, when isolated).
    /// Exit 0 is the only thing that counts as a pass.
    pub command: String,
    /// Past this, the run is a failure rather than an open question.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Set when a manager suggested this command and you have not yet agreed
    /// to it. Unapproved verification is not verification.
    #[serde(default)]
    pub proposed: bool,
}

fn default_timeout() -> u64 {
    600
}

impl Verification {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            timeout_secs: default_timeout(),
            proposed: false,
        }
    }

    /// A command a manager suggested. Carries the same shape, and none of the
    /// authority, until it is approved.
    pub fn proposed(command: impl Into<String>) -> Self {
        Self {
            proposed: true,
            ..Self::new(command)
        }
    }
}

/// Whether an objective is currently something to work on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveState {
    /// Live: a manager may propose work against it.
    #[default]
    Active,
    /// Still true, still wanted, not now. Survives so the intent is not lost.
    Held,
    /// You decided it is done. Only you: a manager reports evidence and never
    /// draws this conclusion.
    Met,
}

impl ObjectiveState {
    pub fn label(&self) -> &'static str {
        match self {
            ObjectiveState::Active => "active",
            ObjectiveState::Held => "held",
            ObjectiveState::Met => "met",
        }
    }

    /// Cycles the way the key does: active → held → met → active.
    pub fn next(&self) -> Self {
        match self {
            ObjectiveState::Active => ObjectiveState::Held,
            ObjectiveState::Held => ObjectiveState::Met,
            ObjectiveState::Met => ObjectiveState::Active,
        }
    }
}


/// What a manager is told, once, as the first thing in its queue.
///
/// Delivered through the TODO queue rather than a spawn argument, which means
/// it arrives when the agent is actually ready for it and goes through the
/// same turn-boundary machinery as everything else — no provider-specific
/// prompt flag, and no race with a CLI still starting up.
///
/// Written in the second person and kept short on purpose: it is paid for in
/// tokens on every manager, and a brief nobody reads to the end is a brief
/// that does not constrain anything.
pub fn manager_brief(short_id: &str) -> String {
    format!(
        "You are the manager for this project in workbench. Your job is to keep it \
moving toward its standing objectives by directing the other agents here.\n\n\
RIGHT NOW YOU ARE READ-ONLY. Propose work; do not dispatch any. Queueing work \
for an agent will be refused, and that is deliberate — the point of this phase \
is that a person reads your reasoning before any of it is acted on.\n\n\
Your session id is {short_id}. Send it as `from` on every control-socket call, \
and as `manager` when you propose.\n\n\
The control socket is at $WORKBENCH_CONTROL_SOCK — a Unix socket speaking one \
JSON object per line. Start with:\n\
  {{\"id\":1,\"method\":\"api.schema\",\"from\":\"{short_id}\"}}\n\
  {{\"id\":2,\"method\":\"state.get\",\"from\":\"{short_id}\"}}\n\
`state.get` carries every project with its objectives, and every agent with \
what it is doing.\n\n\
To record a suggestion:\n\
  {{\"id\":3,\"method\":\"manager.propose\",\"params\":{{\
\"manager\":\"{short_id}\",\
\"objective\":\"<objective id, when it serves one>\",\
\"agent\":\"<agent short id, when you would name one>\",\
\"instruction\":\"what you would tell that agent, in full\",\
\"rationale\":\"why this, and why now\"}}}}\n\n\
What is wanted from you:\n\
- Read the objectives, then the repository, then what the agents are already doing.\n\
- Propose a few concrete pieces of work. Few and specific beats many and vague.\n\
- For each, say what command would show it had landed — something that exits 0. \
Propose it; never assume one.\n\
- Prefer the smallest work that moves an objective, and say plainly when an \
objective needs nothing right now.\n\
- Say so when an objective has no way to check it. That is worth knowing.\n\n\
Not yours: rewriting objectives, deciding one is met, or typing into another \
agent. Those stay with the person you work for.\n\n\
Report what you propose in this pane as you go, so it can be read here."
    )
}

/// A manager's suggestion: this agent, this instruction, toward this
/// objective, for this reason.
///
/// A suggestion and nothing more. Recording one does not queue anything and
/// does not touch an agent — turning it into work is a separate, deliberate
/// act. That gap is the point of the read-only phase: you get to judge the
/// manager's reasoning before it can act on any of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub id: Uuid,
    /// The objective this serves, when the manager tied it to one.
    #[serde(default)]
    pub objective_id: Option<Uuid>,
    /// Short id of the manager that wrote it.
    pub manager: String,
    /// Short id of the agent it suggests, when it named one.
    #[serde(default)]
    pub agent: Option<String>,
    /// What it would tell that agent to do.
    pub instruction: String,
    /// Why. The part actually worth reading in this phase.
    #[serde(default)]
    pub rationale: String,
    pub created_at: DateTime<Utc>,
}

impl Proposal {
    pub fn new(manager: impl Into<String>, instruction: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            objective_id: None,
            manager: manager.into(),
            agent: None,
            instruction: instruction.into(),
            rationale: String::new(),
            created_at: Utc::now(),
        }
    }
}

/// One standing priority for a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Objective {
    pub id: Uuid,
    /// Your words. A manager reads this and never writes it.
    pub text: String,
    pub state: ObjectiveState,
    pub created_at: DateTime<Utc>,
    /// How this would be checked. `None` means a manager may work on it only
    /// with you approving each step — there is nothing to hold the result to.
    #[serde(default)]
    pub done_when: Option<Verification>,
}

impl Objective {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            text: text.into(),
            state: ObjectiveState::default(),
            created_at: Utc::now(),
            done_when: None,
        }
    }

    /// Whether a manager may act on this without asking each time.
    ///
    /// Both halves are required. An objective on hold is not wanted now, and
    /// one with no approved check has no definition of done — acting
    /// autonomously on either is how a manager ends up generating motion
    /// instead of progress.
    pub fn is_autonomous_ready(&self) -> bool {
        self.state == ObjectiveState::Active
            && self
                .done_when
                .as_ref()
                .map(|v| !v.proposed && !v.command.trim().is_empty())
                .unwrap_or(false)
    }
}

/// Priority is the order they sit in, so moving one is the whole API.
///
/// Ranking by position rather than a stored number means there is no way for
/// two objectives to claim the same priority, and no renumbering pass.
pub fn move_objective(objectives: &mut Vec<Objective>, id: Uuid, delta: isize) {
    let Some(from) = objectives.iter().position(|o| o.id == id) else {
        return;
    };
    let to = (from as isize + delta).clamp(0, objectives.len() as isize - 1) as usize;
    if to == from {
        return;
    }
    let item = objectives.remove(from);
    objectives.insert(to, item);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_objective_is_active_and_unverifiable() {
        let objective = Objective::new("keep the control socket covered");
        assert_eq!(objective.state, ObjectiveState::Active);
        assert!(objective.done_when.is_none());
        // Nothing to hold work to, so nothing runs unattended.
        assert!(!objective.is_autonomous_ready());
    }

    /// The gate this exists for: a command a manager suggested is not a
    /// licence to act on it.
    #[test]
    fn a_proposed_check_does_not_unlock_autonomy() {
        let mut objective = Objective::new("raise coverage");
        objective.done_when = Some(Verification::proposed("cargo test"));
        assert!(!objective.is_autonomous_ready());

        objective.done_when.as_mut().unwrap().proposed = false;
        assert!(objective.is_autonomous_ready());
    }

    #[test]
    fn a_held_objective_is_never_worked_on_unattended() {
        let mut objective = Objective::new("rewrite the parser");
        objective.done_when = Some(Verification::new("cargo test"));
        assert!(objective.is_autonomous_ready());

        objective.state = ObjectiveState::Held;
        assert!(!objective.is_autonomous_ready());
        objective.state = ObjectiveState::Met;
        assert!(!objective.is_autonomous_ready());
    }

    #[test]
    fn an_empty_command_is_not_a_check() {
        let mut objective = Objective::new("tidy up");
        objective.done_when = Some(Verification::new("   "));
        assert!(!objective.is_autonomous_ready());
    }

    #[test]
    fn state_cycles_through_every_value() {
        let mut state = ObjectiveState::Active;
        state = state.next();
        assert_eq!(state, ObjectiveState::Held);
        state = state.next();
        assert_eq!(state, ObjectiveState::Met);
        state = state.next();
        assert_eq!(state, ObjectiveState::Active);
    }

    #[test]
    fn moving_reorders_and_stops_at_the_ends() {
        let mut objectives = vec![
            Objective::new("first"),
            Objective::new("second"),
            Objective::new("third"),
        ];
        let second = objectives[1].id;

        move_objective(&mut objectives, second, -1);
        assert_eq!(objectives[0].text, "second");

        // Already at the top: a no-op, not a wrap-around.
        move_objective(&mut objectives, second, -1);
        assert_eq!(objectives[0].text, "second");

        move_objective(&mut objectives, second, 2);
        assert_eq!(objectives[2].text, "second");
        move_objective(&mut objectives, second, 5);
        assert_eq!(objectives[2].text, "second");

        // An id that is not there changes nothing.
        let before = objectives.clone();
        move_objective(&mut objectives, Uuid::new_v4(), -1);
        assert_eq!(objectives, before);
    }
}
