//! The Managers tab as a list of rows.
//!
//! A manager is an ordinary session that happens to carry a brief, so the
//! roster is a filtered view of this project's sessions rather than a second
//! place where managers are stored. The same reasoning as `objectives_view`:
//! the keys and the renderer read one list, so they cannot disagree about what
//! is under the cursor.

use uuid::Uuid;

use super::AppState;

/// One manager, and what it is waiting on you for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagerRow {
    pub session_id: Uuid,
    pub name: String,
    pub badge: String,
    /// Suggestions this manager has made that you have not yet answered.
    /// The only number on the row that asks something of you.
    pub pending: usize,
}

/// Every manager in the selected project, oldest first.
///
/// Creation order rather than activity: the roster is somewhere you look
/// things up, and a list that reorders itself under the cursor is a list you
/// cannot point at.
pub fn rows(state: &AppState) -> Vec<ManagerRow> {
    let Some(workspace) = state.selected_workspace() else {
        return Vec::new();
    };
    let proposals = &workspace.proposals;

    state
        .sessions_for_selected_workspace()
        .iter()
        .filter(|session| session.agent_type.is_manager())
        .map(|session| ManagerRow {
            session_id: session.id,
            name: session.display_name(),
            badge: session.agent_type.badge(),
            pending: proposals
                .iter()
                .filter(|proposal| proposal.is_pending())
                .filter(|proposal| {
                    crate::remote::session_for(state, &proposal.manager) == Some(session.id)
                })
                .count(),
        })
        .collect()
}

/// How many managers this project has.
///
/// Separate from `rows` because this runs every tick to keep the cursor
/// honest, and it does not need the per-row proposal tally that `rows` builds.
pub fn count(state: &AppState) -> usize {
    state
        .sessions_for_selected_workspace()
        .iter()
        .filter(|session| session.agent_type.is_manager())
        .count()
}

/// The manager under the cursor.
pub fn selected(state: &AppState) -> Option<ManagerRow> {
    rows(state).get(state.ui.selected_manager).cloned()
}

/// Keep the cursor on a row that exists, after the list has changed under it.
pub fn clamp(state: &mut AppState) {
    let len = count(state);
    state.ui.selected_manager = state.ui.selected_manager.min(len.saturating_sub(1));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AgentType, Proposal, Session, Workspace};

    fn state_with(
        build: impl Fn(uuid::Uuid) -> Vec<Session>,
        proposals: Vec<Proposal>,
    ) -> AppState {
        let mut state = AppState::default();
        let mut workspace = Workspace::new("w".into(), std::path::PathBuf::from("/tmp/w"));
        workspace.proposals = proposals;
        let id = workspace.id;
        state.data.workspaces.push(workspace);
        state.data.sessions.insert(id, build(id));
        state
    }

    fn manager(workspace: uuid::Uuid) -> Session {
        Session::new(workspace, AgentType::Claude.as_manager(), false)
    }

    fn agent(workspace: uuid::Uuid) -> Session {
        Session::new(workspace, AgentType::Claude, false)
    }

    /// The roster is managers only. An ordinary agent appearing here would
    /// invite you to hand it a brief it was never given.
    #[test]
    fn only_managers_are_listed() {
        let state = state_with(|ws| vec![agent(ws), manager(ws), agent(ws)], Vec::new());
        let rows = rows(&state);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].badge, "M");
    }

    /// The count on a row is what that manager is waiting on you for, so it
    /// has to be that manager's own suggestions and only the unanswered ones.
    #[test]
    fn pending_is_counted_per_manager_and_only_while_unanswered() {
        let ws = uuid::Uuid::new_v4();
        let mine = manager(ws);
        let other = manager(ws);
        let short = mine.short_id();

        let waiting = Proposal::new(short.clone(), "do the thing");
        let mut answered = Proposal::new(short.clone(), "already handled");
        answered.decline();
        let elsewhere = Proposal::new(other.short_id(), "not mine");

        let mine_id = mine.id;
        let state = state_with(
            move |_| vec![mine.clone(), other.clone()],
            vec![waiting, answered, elsewhere],
        );
        let rows = rows(&state);
        let row = rows.iter().find(|r| r.session_id == mine_id).unwrap();
        assert_eq!(row.pending, 1, "declined and other managers' do not count");
    }

    #[test]
    fn the_cursor_is_pulled_back_onto_the_list() {
        let mut state = state_with(|ws| vec![manager(ws)], Vec::new());
        state.ui.selected_manager = 7;
        clamp(&mut state);
        assert_eq!(state.ui.selected_manager, 0);

        let mut empty = state_with(|ws| vec![agent(ws)], Vec::new());
        empty.ui.selected_manager = 3;
        clamp(&mut empty);
        assert_eq!(empty.ui.selected_manager, 0);
        assert!(selected(&empty).is_none());
    }
}
