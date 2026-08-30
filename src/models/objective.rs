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
\"agent\":\"<agent short id — omit it to put approved work on the board, \
where the first idle agent claims it>\",\
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
When work you proposed finishes, a REVIEW TURN arrives in your queue with the \
proposal, the repository movement, and any verification output. Judge whether \
the delivered work does what the proposal asked — no more, no wider — then \
answer with `manager.review` and one outcome: `accept` resolves the job; \
`request_changes` (with exact findings) sends corrections back to the same \
agent, at most {max_rounds} rounds; `needs_user` hands it to the person. \
Accepting a job never marks its objective met — that stays with them.\n\n\
Not yours: rewriting objectives, deciding one is met, or typing into another \
agent. Those stay with the person you work for.\n\n\
Report what you propose in this pane as you go, so it can be read here.",
        max_rounds = MAX_REVIEW_ROUNDS
    )
}


/// One objective's recent history, derived from its proposals — the answer
/// to the morning question "is this moving?", and what the burn is.
///
/// Turns are estimated from structure rather than metered: each approved
/// proposal cost its agent one turn plus one per correction round, and its
/// manager one review per round reached. Honest enough to compare objectives
/// against each other, which is all a burn number is for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectiveLedger {
    /// Accepted in the last seven days.
    pub resolved_this_week: usize,
    /// Working, in rework, or awaiting review right now.
    pub in_flight: usize,
    /// Parked on the user.
    pub needs_user: usize,
    /// Estimated agent turns spent, ever.
    pub agent_turns: usize,
    /// Estimated manager review turns spent, ever.
    pub reviews: usize,
    /// The newest movement of any kind, for "last activity".
    pub last_activity: Option<DateTime<Utc>>,
}

/// The ledger for one objective, out of the project's proposal list.
pub fn objective_ledger(proposals: &[Proposal], objective_id: Uuid) -> ObjectiveLedger {
    let mut ledger = ObjectiveLedger::default();
    let week_ago = Utc::now() - chrono::Duration::days(7);
    for proposal in proposals
        .iter()
        .filter(|p| p.objective_id == Some(objective_id))
    {
        let touched = proposal.resolved_at.unwrap_or(proposal.created_at);
        if ledger.last_activity.is_none_or(|seen| touched > seen) {
            ledger.last_activity = Some(touched);
        }
        match proposal.review {
            Some(ReviewPhase::Resolved) => {
                if proposal.resolved_at.is_some_and(|at| at > week_ago) {
                    ledger.resolved_this_week += 1;
                }
            }
            Some(ReviewPhase::Working) | Some(ReviewPhase::AwaitingReview) => {
                ledger.in_flight += 1;
            }
            Some(ReviewPhase::NeedsUser) => ledger.needs_user += 1,
            // Closed is over: not in flight, not waiting on anyone, and not a
            // resolution — nobody accepted it. The turns it burned are still
            // counted below.
            Some(ReviewPhase::Closed) | None => {}
        }
        if proposal.state == ProposalState::Approved {
            let rounds = proposal.review_rounds as usize;
            ledger.agent_turns += 1 + rounds;
            ledger.reviews += rounds
                + usize::from(matches!(
                    proposal.review,
                    Some(ReviewPhase::AwaitingReview)
                        | Some(ReviewPhase::Resolved)
                        | Some(ReviewPhase::NeedsUser)
                        // Closing does not un-spend the review turn the
                        // manager burned to punt it.
                        | Some(ReviewPhase::Closed)
                ));
        }
    }
    ledger
}

/// How a verification command ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Exit 0, and nothing else.
    Passed,
    Failed,
    /// Still running when its time ran out. Not a pass, and not quite a
    /// failure either — worth telling apart, because it usually means the
    /// timeout is wrong rather than the code.
    TimedOut,
    /// Never started: no such command, no such directory.
    CouldNotRun,
}

impl Outcome {
    pub fn passed(&self) -> bool {
        matches!(self, Outcome::Passed)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Outcome::Passed => "passed",
            Outcome::Failed => "failed",
            Outcome::TimedOut => "timed out",
            Outcome::CouldNotRun => "could not run",
        }
    }
}

/// What happened when workbench ran the check.
///
/// A record of a real process, produced by workbench and handed to the
/// manager. The manager never authors one — that asymmetry is the whole
/// reason any of this can be trusted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationRun {
    pub at: DateTime<Utc>,
    pub command: String,
    /// `None` when it was killed or never started.
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    /// The last few KB of output. Enough to see which test failed, bounded so
    /// a runaway build log cannot become the saved state.
    pub tail: String,
    pub outcome: Outcome,
}

/// What the repository looked like at a moment.
///
/// Both halves are needed. Uncommitted work moves the shortstat; committed
/// work moves HEAD and can leave the shortstat exactly where it was. Watching
/// only one of them calls real work "nothing happened" about half the time —
/// and on a shared branch, where a working-tree diff cannot be attributed to
/// one assignment anyway, HEAD is the honest signal.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoMark {
    #[serde(default)]
    pub head: Option<String>,
    /// A hash of the working tree's contents, gitignore respected.
    ///
    /// The stat below cannot see a file that is not tracked yet, and writing a
    /// new file is the most ordinary thing an agent does — without this, that
    /// work reads as "nothing changed" and a real result gets thrown away.
    #[serde(default)]
    pub tree: Option<String>,
    #[serde(default)]
    pub insertions: usize,
    #[serde(default)]
    pub deletions: usize,
}

impl RepoMark {
    /// Whether anything happened between the two.
    pub fn changed_from(&self, before: &RepoMark) -> bool {
        self != before
    }
}

/// Where a proposal's work stands after approval — the loop the manager
/// closes.
///
/// The manager that proposed the work reviews what came back; approving the
/// original job authorized exactly this loop and nothing wider. New or
/// expanded work still takes a new proposal, and resolving a job never marks
/// its objective met — that decision stays with the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewPhase {
    /// Queued or running with the agent.
    Working,
    /// The agent finished; a review turn is with the manager.
    AwaitingReview,
    /// The manager accepted. Done — but "done" for the job, not the objective.
    Resolved,
    /// The manager could not decide, or the correction loop hit its bound.
    /// The one state that asks for a person.
    NeedsUser,
    /// The person answered, and the answer was stop: a punted review declined
    /// rather than bought another lap. Terminal, and the one phase a manager
    /// never writes — it is the record of a user ending the loop.
    ///
    /// Distinct from `ProposalState::Declined`, which means the suggestion was
    /// never taken up at all. This one was approved and worked on first, so it
    /// stays `Approved` and goes on owing the ledger the turns it burned.
    Closed,
}

/// How many correction rounds a single approval buys. Past this the loop has
/// earned a person's eyes, not another lap.
pub const MAX_REVIEW_ROUNDS: u8 = 3;

/// What the two runs, taken together, mean.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The check passes and the repository moved. Worth your review — never
    /// "shipped": a green suite is the absence of one kind of failure, not
    /// correctness.
    Verified,
    Rejected { why: String },
    /// Nothing can be concluded, usually because it was already broken.
    /// Surfaced rather than retried: retrying a check that was failing before
    /// anyone touched it just burns turns.
    Inconclusive { why: String },
}

impl Verdict {
    pub fn label(&self) -> &'static str {
        match self {
            Verdict::Verified => "verified",
            Verdict::Rejected { .. } => "rejected",
            Verdict::Inconclusive { .. } => "inconclusive",
        }
    }

    pub fn why(&self) -> &str {
        match self {
            Verdict::Verified => "the check passes and the repository moved",
            Verdict::Rejected { why } | Verdict::Inconclusive { why } => why,
        }
    }
}

/// The verdict table.
///
/// Kept as a pure function of the three facts so it can be read, argued with,
/// and tested without running anything. `baseline` is what the check said
/// before the agent was given the work — without it, an agent is credited for
/// a suite that was already green and blamed for one that was already red.
pub fn judge(baseline: Option<&VerificationRun>, result: &VerificationRun, changed: bool) -> Verdict {
    let was_passing = baseline.map(|run| run.outcome.passed());

    match (was_passing, result.outcome.passed()) {
        // It passes and something moved. True whether it was passing before
        // (kept working) or failing (got fixed).
        (_, true) if changed => Verdict::Verified,

        // Passing but nothing moved: the most quietly wrong outcome there is,
        // and the reason a diff is checked at all. An agent that reports
        // success having done nothing lands exactly here.
        (_, true) => Verdict::Rejected {
            why: "the check passes but nothing in the repository changed".into(),
        },

        // It was passing before and is not now. That is a regression whatever
        // else the work achieved.
        (Some(true), false) => Verdict::Rejected {
            why: format!("{} — it was passing before this", result.outcome.label()),
        },

        // Broken before, broken after. Say so; do not retry.
        (Some(false), false) => Verdict::Inconclusive {
            why: format!("{} — but it was already failing before this", result.outcome.label()),
        },

        // No baseline to compare against, so a failure is only a failure.
        (None, false) => Verdict::Rejected {
            why: format!("{}, with nothing to compare against", result.outcome.label()),
        },
    }
}

/// What has been decided about a suggestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalState {
    /// Written by a manager, waiting to be read.
    #[default]
    Pending,
    /// You turned it into work. The queued item it became is recorded, so the
    /// suggestion and the work stay connected once results start arriving.
    Approved,
    /// You said no. Kept on disk rather than deleted so a manager can see it
    /// asked and was refused, but dropped from the list you scroll.
    Declined,
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
    #[serde(default)]
    pub state: ProposalState,
    /// The queued item this became, once approved.
    #[serde(default)]
    pub todo_id: Option<Uuid>,
    /// What the check said before the agent was handed the work.
    #[serde(default)]
    pub baseline: Option<VerificationRun>,
    /// And after its turn ended.
    #[serde(default)]
    pub result: Option<VerificationRun>,
    /// The repository as it stood at each point, so "did anything happen" is
    /// a comparison rather than a guess.
    #[serde(default)]
    pub before: Option<RepoMark>,
    #[serde(default)]
    pub after: Option<RepoMark>,
    /// What the two runs mean together. Set by workbench, never by a manager.
    #[serde(default)]
    pub verdict: Option<Verdict>,
    /// Where the work stands after approval. `None` until approved (and on
    /// records from before this field existed, which the retire pass
    /// migrates).
    #[serde(default)]
    pub review: Option<ReviewPhase>,
    /// Correction rounds so far (see `MAX_REVIEW_ROUNDS`).
    #[serde(default)]
    pub review_rounds: u8,
    /// The manager's latest findings, verbatim — what it asked to change, or
    /// why it punted to the user.
    #[serde(default)]
    pub findings: Option<String>,
    /// When the manager accepted, for the objective's ledger.
    #[serde(default)]
    pub resolved_at: Option<DateTime<Utc>>,
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
            state: ProposalState::default(),
            todo_id: None,
            baseline: None,
            result: None,
            before: None,
            after: None,
            verdict: None,
            review: None,
            review_rounds: 0,
            findings: None,
            resolved_at: None,
        }
    }

    pub fn is_pending(&self) -> bool {
        self.state == ProposalState::Pending
    }

    pub fn is_declined(&self) -> bool {
        self.state == ProposalState::Declined
    }

    pub fn decline(&mut self) {
        self.state = ProposalState::Declined;
    }

    /// Record that this became a queued item.
    pub fn approve(&mut self, todo_id: Uuid) {
        self.state = ProposalState::Approved;
        self.todo_id = Some(todo_id);
        self.review = Some(ReviewPhase::Working);
    }

    /// The agent's turn ended: the work now waits on the manager.
    pub fn work_finished(&mut self) {
        self.review = Some(ReviewPhase::AwaitingReview);
    }

    /// The manager asked for corrections. Returns false — and parks the
    /// proposal on the user — once the loop has used up its rounds: past the
    /// bound, another lap is not the manager's to grant.
    pub fn request_changes(&mut self, findings: String, new_todo: Uuid) -> bool {
        if self.review_rounds >= MAX_REVIEW_ROUNDS {
            self.findings = Some(findings);
            self.review = Some(ReviewPhase::NeedsUser);
            return false;
        }
        self.review_rounds += 1;
        self.findings = Some(findings);
        self.todo_id = Some(new_todo);
        self.review = Some(ReviewPhase::Working);
        true
    }

    pub fn accept(&mut self) {
        self.review = Some(ReviewPhase::Resolved);
        self.resolved_at = Some(Utc::now());
    }

    pub fn needs_user(&mut self, why: String) {
        self.findings = Some(why);
        self.review = Some(ReviewPhase::NeedsUser);
    }

    /// Whether this proposal is waiting on its manager's review.
    pub fn awaiting_review(&self) -> bool {
        self.review == Some(ReviewPhase::AwaitingReview)
    }

    /// An approved job on the board, waiting for an idle agent to claim it:
    /// no agent named, nothing queued yet.
    pub fn open_on_board(&self) -> bool {
        self.state == ProposalState::Approved
            && self.review == Some(ReviewPhase::Working)
            && self.agent.is_none()
            && self.todo_id.is_none()
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
    /// Read by autonomous mode, which is not wired up yet.
    #[allow(dead_code)]
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

#[cfg(test)]
mod verdict_tests {
    use super::*;

    fn run(outcome: Outcome) -> VerificationRun {
        VerificationRun {
            at: Utc::now(),
            command: "cargo test".into(),
            exit_code: if outcome.passed() { Some(0) } else { Some(1) },
            duration_ms: 10,
            tail: String::new(),
            outcome,
        }
    }

    #[test]
    fn passing_with_a_change_is_verified() {
        let pass = run(Outcome::Passed);
        assert_eq!(judge(Some(&run(Outcome::Passed)), &pass, true), Verdict::Verified);
        // Broken before, fixed now: the good case for a fix objective.
        assert_eq!(judge(Some(&run(Outcome::Failed)), &pass, true), Verdict::Verified);
        assert_eq!(judge(None, &pass, true), Verdict::Verified);
    }

    /// The quietly wrong outcome this whole mechanism exists to catch: an
    /// agent reports success, the suite is green because it was already
    /// green, and not a line was written.
    #[test]
    fn passing_with_nothing_changed_is_rejected() {
        let verdict = judge(Some(&run(Outcome::Passed)), &run(Outcome::Passed), false);
        assert!(matches!(verdict, Verdict::Rejected { .. }));
        assert!(verdict.why().contains("nothing in the repository changed"));
    }

    /// A regression is a rejection whatever else the work achieved.
    #[test]
    fn breaking_something_that_worked_is_rejected() {
        let verdict = judge(Some(&run(Outcome::Passed)), &run(Outcome::Failed), true);
        assert!(matches!(verdict, Verdict::Rejected { .. }));
        assert!(verdict.why().contains("was passing before"));
    }

    /// Already broken stays inconclusive rather than being blamed on whoever
    /// touched it last — and rather than being retried forever.
    #[test]
    fn already_failing_is_inconclusive_not_a_rejection() {
        let verdict = judge(Some(&run(Outcome::Failed)), &run(Outcome::Failed), true);
        assert!(matches!(verdict, Verdict::Inconclusive { .. }));
        assert!(verdict.why().contains("already failing"));
    }

    #[test]
    fn failing_with_no_baseline_is_rejected_and_says_why() {
        let verdict = judge(None, &run(Outcome::Failed), true);
        assert!(matches!(verdict, Verdict::Rejected { .. }));
        assert!(verdict.why().contains("nothing to compare"));
    }

    /// A timeout is reported as itself, not flattened into "failed".
    #[test]
    fn a_timeout_keeps_its_name() {
        let verdict = judge(Some(&run(Outcome::Passed)), &run(Outcome::TimedOut), true);
        assert!(verdict.why().contains("timed out"), "{}", verdict.why());
    }

    /// Committed work moves HEAD without moving the working-tree stat, which
    /// is exactly the case a shortstat-only check would call "nothing done".
    #[test]
    fn a_commit_counts_as_a_change() {
        let before = RepoMark { head: Some("aaa".into()), tree: Some("t1".into()), insertions: 0, deletions: 0 };
        let committed = RepoMark { head: Some("bbb".into()), tree: Some("t1".into()), insertions: 0, deletions: 0 };
        assert!(committed.changed_from(&before));

        let uncommitted = RepoMark { head: Some("aaa".into()), tree: Some("t2".into()), insertions: 12, deletions: 3 };
        assert!(uncommitted.changed_from(&before));
        assert!(!before.clone().changed_from(&before));
    }

    /// The ledger answers "is this moving, and at what burn" from structure
    /// alone: resolved-this-week, in flight, parked, and estimated turns.
    #[test]
    fn the_ledger_reads_movement_and_burn_off_the_proposals() {
        let objective = Objective::new("stay green");
        let mut done = Proposal::new("m", "fix a");
        done.objective_id = Some(objective.id);
        done.approve(Uuid::new_v4());
        done.review_rounds = 2;
        done.accept();
        let mut flying = Proposal::new("m", "fix b");
        flying.objective_id = Some(objective.id);
        flying.approve(Uuid::new_v4());
        let mut parked = Proposal::new("m", "fix c");
        parked.objective_id = Some(objective.id);
        parked.approve(Uuid::new_v4());
        parked.needs_user("unclear".into());
        let unrelated = Proposal::new("m", "other objective");

        let ledger =
            objective_ledger(&[done, flying, parked, unrelated], objective.id);
        assert_eq!(ledger.resolved_this_week, 1);
        assert_eq!(ledger.in_flight, 1);
        assert_eq!(ledger.needs_user, 1);
        // done: 1+2 agent turns, 2+1 reviews; flying: 1, 0+0 (still working);
        // parked: 1, 0+1.
        assert_eq!(ledger.agent_turns, 5);
        assert_eq!(ledger.reviews, 4);
        assert!(ledger.last_activity.is_some());
    }
}
