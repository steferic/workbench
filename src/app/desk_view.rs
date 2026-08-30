//! Everything waiting on the user, from every project, in one list.
//!
//! Decisions used to scatter — pending proposals in one tab, "needs you"
//! reviews in another, unapproved checks on their objectives, blocked agents
//! in a third pane — and finding them all was itself work. The desk is the
//! single answer to "what needs me?": ordered by urgency, actionable in
//! place, and empty when the honest answer is nothing.

use uuid::Uuid;

use super::AppState;
use crate::models::{ProposalState, ReviewPhase, SessionStatus};

/// One thing waiting on the user. Ordering of the variants is the ordering
/// of the desk: what is stalling *right now* outranks what can wait a beat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeskRow {
    /// An agent stopped on a question. Costing time every second unread.
    BlockedAgent { session_id: Uuid, project: String },
    /// A review the manager could not close.
    NeedsUser {
        workspace_id: Uuid,
        proposal_id: Uuid,
        project: String,
    },
    /// A manager's suggestion awaiting a yes or no.
    PendingProposal {
        workspace_id: Uuid,
        proposal_id: Uuid,
        project: String,
    },
    /// A proposed done-when check nobody has approved.
    ProposedCheck {
        workspace_id: Uuid,
        objective_id: Uuid,
        project: String,
    },
}

/// The desk, most urgent first. Every project, not just the selected one —
/// a decision does not become less yours for living in another workspace.
pub fn rows(state: &AppState) -> Vec<DeskRow> {
    let mut blocked = Vec::new();
    let mut needs_user = Vec::new();
    let mut pending = Vec::new();
    let mut checks = Vec::new();

    for workspace in &state.data.workspaces {
        let project = workspace.name.clone();
        for proposal in &workspace.proposals {
            if proposal.review == Some(ReviewPhase::NeedsUser) {
                needs_user.push(DeskRow::NeedsUser {
                    workspace_id: workspace.id,
                    proposal_id: proposal.id,
                    project: project.clone(),
                });
            } else if proposal.state == ProposalState::Pending {
                pending.push(DeskRow::PendingProposal {
                    workspace_id: workspace.id,
                    proposal_id: proposal.id,
                    project: project.clone(),
                });
            }
        }
        for objective in &workspace.objectives {
            if objective
                .done_when
                .as_ref()
                .is_some_and(|check| check.proposed)
            {
                checks.push(DeskRow::ProposedCheck {
                    workspace_id: workspace.id,
                    objective_id: objective.id,
                    project: project.clone(),
                });
            }
        }
        for session in state.data.sessions.get(&workspace.id).into_iter().flatten() {
            if session.status == SessionStatus::Running
                && state
                    .system
                    .agent_status
                    .get(&session.id)
                    .is_some_and(|status| status.activity.needs_attention().is_some())
            {
                blocked.push(DeskRow::BlockedAgent {
                    session_id: session.id,
                    project: project.clone(),
                });
            }
        }
    }

    let mut rows = blocked;
    rows.extend(needs_user);
    rows.extend(pending);
    rows.extend(checks);
    rows
}

/// The row under the cursor.
pub fn selected(state: &AppState) -> Option<DeskRow> {
    rows(state).get(state.ui.selected_desk_row).cloned()
}

/// Keep the cursor on a row that exists.
pub fn clamp(state: &mut AppState) {
    let len = rows(state).len();
    state.ui.selected_desk_row = state.ui.selected_desk_row.min(len.saturating_sub(1));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AgentType, Objective, Proposal, Session, Verification, Workspace};

    fn world() -> AppState {
        let mut state = AppState::default();
        let mut ws = Workspace::new("alpha".into(), std::path::PathBuf::from("/tmp/a"));
        let ws_id = ws.id;

        let mut objective = Objective::new("keep it green");
        objective.done_when = Some(Verification::proposed("cargo test"));
        ws.objectives.push(objective);

        let pending = Proposal::new("m1", "small fix");
        let mut parked = Proposal::new("m1", "risky change");
        parked.state = ProposalState::Approved;
        parked.needs_user("manager punted".into());
        ws.proposals.push(pending);
        ws.proposals.push(parked);

        let mut agent = Session::new(ws_id, AgentType::Claude, false);
        agent.status = SessionStatus::Running;
        let agent_id = agent.id;
        state.data.workspaces.push(ws);
        state.data.sessions.insert(ws_id, vec![agent]);
        state.system.agent_status.insert(
            agent_id,
            crate::agent_status::AgentStatus {
                activity: crate::agent_status::Activity::NeedsAttention(
                    crate::agent_status::Attention::Permission,
                ),
                reason: "may I".into(),
                at: chrono::Utc::now(),
                event: "Notification".into(),
                transcript: None,
                model: None,
            },
        );
        state
    }

    /// The desk gathers all four kinds, most urgent first: an agent stalled
    /// on a question is costing time right now, then reviews the manager
    /// punted, then ordinary approvals, then checks.
    #[test]
    fn the_desk_orders_by_urgency() {
        let state = world();
        let rows = rows(&state);
        assert_eq!(rows.len(), 4);
        assert!(matches!(rows[0], DeskRow::BlockedAgent { .. }));
        assert!(matches!(rows[1], DeskRow::NeedsUser { .. }));
        assert!(matches!(rows[2], DeskRow::PendingProposal { .. }));
        assert!(matches!(rows[3], DeskRow::ProposedCheck { .. }));
    }

    /// The desk reaches into every project — a decision does not become less
    /// yours for living in an unselected workspace.
    #[test]
    fn the_desk_sees_unselected_workspaces() {
        let mut state = world();
        let mut other = Workspace::new("beta".into(), std::path::PathBuf::from("/tmp/b"));
        other.proposals.push(Proposal::new("m2", "over here too"));
        state.data.workspaces.push(other);
        // selected workspace stays index 0
        assert_eq!(
            rows(&state)
                .iter()
                .filter(|r| matches!(r, DeskRow::PendingProposal { project, .. } if project == "beta"))
                .count(),
            1
        );
    }
}
