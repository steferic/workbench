//! The Objectives tab as a flat list of rows.
//!
//! Objectives and the proposals made against them are drawn together, so the
//! cursor has to move through both. Flattening them here rather than in the
//! renderer means the keys and the drawing agree about what is under the
//! cursor — the alternative is two places computing "which row is selected"
//! and disagreeing the first time either changes.

use uuid::Uuid;

use super::AppState;

/// One line in the Objectives tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectiveRow {
    /// A standing priority. `index` is its position, which is its rank.
    Objective { id: Uuid, index: usize },
    /// A manager's suggestion, sitting under the objective it serves — or at
    /// the end, when it serves none.
    Proposal { id: Uuid },
}

impl ObjectiveRow {
    pub fn objective_id(&self) -> Option<Uuid> {
        match self {
            ObjectiveRow::Objective { id, .. } => Some(*id),
            ObjectiveRow::Proposal { .. } => None,
        }
    }

    pub fn proposal_id(&self) -> Option<Uuid> {
        match self {
            ObjectiveRow::Proposal { id } => Some(*id),
            ObjectiveRow::Objective { .. } => None,
        }
    }
}

/// Every row, in the order they are drawn.
///
/// Declined proposals are left out: keeping them would mean the list only ever
/// grows, and a suggestion you turned down is not information you need again.
pub fn rows(state: &AppState) -> Vec<ObjectiveRow> {
    let Some(workspace) = state.selected_workspace() else {
        return Vec::new();
    };
    let live = |proposal: &&crate::models::Proposal| !proposal.is_declined();

    let mut rows = Vec::new();
    for (index, objective) in workspace.objectives.iter().enumerate() {
        rows.push(ObjectiveRow::Objective {
            id: objective.id,
            index,
        });
        for proposal in workspace
            .proposals
            .iter()
            .filter(live)
            .filter(|p| p.objective_id == Some(objective.id))
        {
            rows.push(ObjectiveRow::Proposal { id: proposal.id });
        }
    }
    for proposal in workspace
        .proposals
        .iter()
        .filter(live)
        .filter(|p| p.objective_id.is_none())
    {
        rows.push(ObjectiveRow::Proposal { id: proposal.id });
    }
    rows
}

/// The row under the cursor.
pub fn selected(state: &AppState) -> Option<ObjectiveRow> {
    rows(state).get(state.ui.selected_objective).copied()
}

/// The objective under the cursor, when the cursor is on one.
pub fn selected_objective(state: &AppState) -> Option<&crate::models::Objective> {
    let id = selected(state)?.objective_id()?;
    state
        .selected_workspace()?
        .objectives
        .iter()
        .find(|o| o.id == id)
}

/// The proposal under the cursor, when the cursor is on one.
pub fn selected_proposal(state: &AppState) -> Option<&crate::models::Proposal> {
    let id = selected(state)?.proposal_id()?;
    state
        .selected_workspace()?
        .proposals
        .iter()
        .find(|p| p.id == id)
}

/// Keep the cursor on a row that exists, after the list has changed under it.
pub fn clamp(state: &mut AppState) {
    let len = rows(state).len();
    state.ui.selected_objective = state.ui.selected_objective.min(len.saturating_sub(1));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Objective, Proposal, Workspace};

    fn state_with(objectives: Vec<Objective>, proposals: Vec<Proposal>) -> AppState {
        let mut state = AppState::default();
        let mut workspace = Workspace::new("w".into(), std::path::PathBuf::from("/tmp/w"));
        workspace.objectives = objectives;
        workspace.proposals = proposals;
        state.data.workspaces.push(workspace);
        state
    }

    /// A proposal is drawn under the objective it serves, so the cursor has to
    /// find it there too — the renderer and the keys read the same list.
    #[test]
    fn proposals_sit_under_the_objective_they_serve() {
        let first = Objective::new("first");
        let second = Objective::new("second");
        let mut proposal = Proposal::new("mgr1", "do the thing");
        proposal.objective_id = Some(first.id);

        let state = state_with(vec![first.clone(), second.clone()], vec![proposal.clone()]);
        let rows = rows(&state);

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].objective_id(), Some(first.id));
        assert_eq!(rows[1].proposal_id(), Some(proposal.id));
        assert_eq!(rows[2].objective_id(), Some(second.id));
    }

    /// One tied to nothing still has to be reachable, or it can never be
    /// approved or dismissed.
    #[test]
    fn a_proposal_with_no_objective_lands_at_the_end() {
        let objective = Objective::new("first");
        let loose = Proposal::new("mgr1", "unrelated idea");
        let state = state_with(vec![objective.clone()], vec![loose.clone()]);
        let rows = rows(&state);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].objective_id(), Some(objective.id));
        assert_eq!(rows[1].proposal_id(), Some(loose.id));
    }

    /// Declined ones leave the list. Otherwise it only ever grows, and a
    /// suggestion you turned down is not something to keep scrolling past.
    #[test]
    fn declining_removes_it_from_the_list() {
        let objective = Objective::new("first");
        let mut declined = Proposal::new("mgr1", "no thanks");
        declined.objective_id = Some(objective.id);
        declined.decline();
        let mut kept = Proposal::new("mgr1", "yes please");
        kept.objective_id = Some(objective.id);

        let state = state_with(vec![objective], vec![declined, kept.clone()]);
        let rows = rows(&state);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].proposal_id(), Some(kept.id));
    }

    #[test]
    fn the_cursor_is_pulled_back_onto_the_list() {
        let mut state = state_with(vec![Objective::new("only")], Vec::new());
        state.ui.selected_objective = 9;
        clamp(&mut state);
        assert_eq!(state.ui.selected_objective, 0);

        // And an empty list leaves it somewhere harmless rather than panicking.
        let mut empty = state_with(Vec::new(), Vec::new());
        empty.ui.selected_objective = 4;
        clamp(&mut empty);
        assert_eq!(empty.ui.selected_objective, 0);
        assert!(selected(&empty).is_none());
    }
}
