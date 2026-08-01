//! Tasks pane: editing the selected agent's TODO queue.
//!
//! These items are workbench's own state, so unlike the agent's task list they
//! can simply be edited. Dispatch is not done here — `app::todo_dispatch`
//! decides when an item may go out.

use crate::app::utilities::load_utility_content;
use crate::app::{tasks_view, Action, AppState, InputMode, TaskEdit, UtilityItem};
use anyhow::Result;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::save_config;

/// Keep the pane's row cursor meaningful as the Sessions pane cursor moves:
/// a row index into one agent's list means nothing in another's.
pub fn sync_selection(state: &mut AppState) {
    let agent = tasks_view::selected_agent(state).map(|a| a.session_id);
    if state.ui.tasks_agent != agent {
        state.ui.tasks_agent = agent;
        state.ui.selected_task_row = 0;
    }
}

pub fn handle_task_action(
    state: &mut AppState,
    action: Action,
    action_tx: &mpsc::UnboundedSender<Action>,
) -> Result<()> {
    match action {
        Action::SelectNextTask => {
            let count = tasks_view::rows(state).len();
            if count > 0 {
                state.ui.selected_task_row = (state.ui.selected_task_row + 1).min(count - 1);
            }
        }
        Action::SelectPrevTask => {
            state.ui.selected_task_row = state.ui.selected_task_row.saturating_sub(1);
        }
        Action::ToggleTasksTab => {
            state.ui.selected_tasks_tab = state.ui.selected_tasks_tab.toggle();
            state.ui.selected_task_row = 0;
        }
        Action::FocusSelectedTaskAgent => {
            if let Some(row) = tasks_view::selected_row(state) {
                state.set_active_session_id(Some(row.session_id()));
                state.ui.focus = crate::app::FocusPanel::OutputPane;
            }
        }
        Action::EnterTaskEditMode(edit) => {
            let Some(agent) = tasks_view::selected_agent(state) else {
                state.ui.set_task_status("Select an agent in Sessions");
                return Ok(());
            };
            // Editing needs an item under the cursor; adding does not.
            let existing = tasks_view::selected_row(state).and_then(|row| row.todo_id());
            if edit != TaskEdit::Add && existing.is_none() {
                state.ui.set_task_status("Select a queued item first");
                return Ok(());
            }

            state.ui.input_buffer = match (edit, existing) {
                // Editing starts from the current text so a typo is a fix,
                // not a retype.
                (TaskEdit::Rewrite, Some(id)) => tasks_view::todo_at(state, agent.session_id, id)
                    .map(|item| item.text.clone())
                    .unwrap_or_default(),
                _ => String::new(),
            };
            state.ui.task_edit = Some((agent.session_id, edit, String::new()));
            state.ui.input_mode = InputMode::ComposeTaskMessage;
        }
        Action::SendTaskMessage(text) => {
            let Some((session_id, edit, _)) = state.ui.task_edit.take() else {
                state.ui.input_mode = InputMode::Normal;
                return Ok(());
            };
            state.ui.input_mode = InputMode::Normal;
            state.ui.input_buffer.clear();

            let text = text.trim().to_string();
            if text.is_empty() {
                return Ok(());
            }
            let selected = tasks_view::selected_row(state).and_then(|row| row.todo_id());
            let Some(session) = state.get_session_mut(session_id) else {
                return Ok(());
            };

            match edit {
                TaskEdit::Add => {
                    session.todo_queue.add(text);
                    let left = session.todo_queue.pending_count();
                    state.ui.set_task_status(format!("Queued — {left} to run"));
                }
                TaskEdit::Rewrite => {
                    if let Some(item) = selected.and_then(|id| session.todo_queue.get_mut(id)) {
                        item.text = text;
                        state.ui.set_task_status("Updated");
                    }
                }
            }
            super::save_state(state, "failed to save the todo queue");
        }
        Action::DeleteSelectedTodo => {
            let Some(row) = tasks_view::selected_row(state) else {
                return Ok(());
            };
            let (session_id, Some(todo)) = (row.session_id(), row.todo_id()) else {
                state.ui.set_task_status("That row is the agent's, not yours");
                return Ok(());
            };
            if let Some(session) = state.get_session_mut(session_id) {
                // Deleting the item the agent is working on only removes it
                // from the queue; the turn it started is already out there.
                session.todo_queue.remove(todo);
            }
            let count = tasks_view::rows(state).len();
            state.ui.selected_task_row = state.ui.selected_task_row.min(count.saturating_sub(1));
            super::save_state(state, "failed to save the todo queue");
        }
        Action::MoveSelectedTodo(delta) => {
            let Some(row) = tasks_view::selected_row(state) else {
                return Ok(());
            };
            let (session_id, Some(todo)) = (row.session_id(), row.todo_id()) else {
                return Ok(());
            };
            if let Some(session) = state.get_session_mut(session_id) {
                session.todo_queue.shift(todo, delta);
            }
            // Follow the item so repeated presses keep moving the same one.
            let rows = tasks_view::rows(state);
            if let Some(index) = rows.iter().position(|r| r.todo_id() == Some(todo)) {
                state.ui.selected_task_row = index;
            }
            super::save_state(state, "failed to save the todo queue");
        }
        Action::ToggleTodoQueuePaused => {
            let Some(agent) = tasks_view::selected_agent(state) else {
                return Ok(());
            };
            let paused = match state.get_session_mut(agent.session_id) {
                Some(session) => {
                    session.todo_queue.paused = !session.todo_queue.paused;
                    session.todo_queue.paused
                }
                None => return Ok(()),
            };
            state
                .ui
                .set_task_status(if paused { "Queue paused" } else { "Queue running" });
            super::save_state(state, "failed to save the todo queue");
        }
        Action::ClearCompletedTodos => {
            let Some(agent) = tasks_view::selected_agent(state) else {
                return Ok(());
            };
            if let Some(session) = state.get_session_mut(agent.session_id) {
                session.todo_queue.clear_completed();
            }
            let count = tasks_view::rows(state).len();
            state.ui.selected_task_row = state.ui.selected_task_row.min(count.saturating_sub(1));
            state.ui.set_task_status("Cleared finished items");
            super::save_state(state, "failed to save the todo queue");
        }
        Action::AgentTasksRefreshed(trackers) => {
            record_provider_session_ids(state, &trackers);
            state.system.agent_tasks = trackers;
            state.system.task_refresh_inflight = false;
        }
        Action::ActivateUtility => {
            // Handle ToggleTheme - flip dark/light and persist
            if state.ui.selected_utility == UtilityItem::ToggleTheme {
                state.ui.theme_mode = state.ui.theme_mode.toggled();
                let config = crate::persistence::GlobalConfig {
                    banner_visible: state.ui.banner_visible,
                    left_panel_ratio: state.ui.layout.left_panel_ratio,
                    workspace_ratio: state.ui.layout.workspace_ratio,
                    sessions_ratio: state.ui.layout.sessions_ratio,
                    tasks_ratio: state.ui.layout.tasks_ratio,
                    output_split_ratio: state.ui.layout.output_split_ratio,
                    theme_mode: state.ui.theme_mode,
                };
                save_config(state, &config, "failed to save theme config");
            } else {
                load_utility_content(state, action_tx);
                state.set_active_session_id(None);
            }
        }
        _ => {}
    }
    Ok(())
}

/// Remember which provider conversation each session owns, so a restart can
/// resume THAT conversation instead of the directory's most recent one (which
/// several agents in one project would all land on).
///
/// The id is re-read every refresh rather than trusted forever: resuming can
/// leave an agent writing to a different conversation than the one we asked
/// for, and the log we resolved is the ground truth for where it ended up.
fn record_provider_session_ids(
    state: &mut AppState,
    trackers: &std::collections::HashMap<Uuid, crate::agent_tasks::TaskTracker>,
) {
    let mut changed = false;
    for (session_id, tracker) in trackers {
        let Some(id) = tracker.provider_session_id() else {
            continue;
        };
        if let Some(session) = state.get_session_mut(*session_id) {
            if session.provider_session_id.as_deref() != Some(id.as_str()) {
                session.provider_session_id = Some(id);
                changed = true;
            }
        }
    }
    if changed {
        super::save_state(state, "failed to save agent conversation ids");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TaskRow;
    use crate::models::{AgentType, Session, Workspace};
    use tokio::sync::mpsc;

    fn state_with_agent() -> (AppState, Uuid) {
        let mut state = AppState::default();
        let workspace = Workspace::new("w".into(), std::path::PathBuf::from("/tmp/w"));
        let workspace_id = workspace.id;
        let session = Session::new(workspace_id, AgentType::Claude, false);
        let session_id = session.id;
        state.data.workspaces.push(workspace);
        state.data.sessions.insert(workspace_id, vec![session]);
        (state, session_id)
    }

    fn act(state: &mut AppState, action: Action) {
        let (tx, _rx) = mpsc::unbounded_channel();
        handle_task_action(state, action, &tx).unwrap();
    }

    fn add(state: &mut AppState, text: &str) {
        act(state, Action::EnterTaskEditMode(TaskEdit::Add));
        act(state, Action::SendTaskMessage(text.to_string()));
    }

    fn queue(state: &AppState, id: Uuid) -> &crate::models::TodoQueue {
        &state.get_session(id).unwrap().todo_queue
    }

    #[test]
    fn several_items_can_be_queued_up_front() {
        let (mut state, id) = state_with_agent();

        add(&mut state, "fix the redirect");
        add(&mut state, "write the migration");
        add(&mut state, "update the README");

        let texts: Vec<&str> = queue(&state, id)
            .items
            .iter()
            .map(|i| i.text.as_str())
            .collect();
        assert_eq!(
            texts,
            vec!["fix the redirect", "write the migration", "update the README"]
        );
        assert_eq!(queue(&state, id).pending_count(), 3);
    }

    #[test]
    fn editing_starts_from_the_current_text() {
        let (mut state, id) = state_with_agent();
        add(&mut state, "fix the redirect");
        state.ui.selected_task_row = 0;

        act(&mut state, Action::EnterTaskEditMode(TaskEdit::Rewrite));
        assert_eq!(state.ui.input_buffer, "fix the redirect");

        act(
            &mut state,
            Action::SendTaskMessage("fix the redirect properly".into()),
        );
        assert_eq!(queue(&state, id).items[0].text, "fix the redirect properly");
    }

    #[test]
    fn items_can_be_reordered_and_the_cursor_follows() {
        let (mut state, id) = state_with_agent();
        add(&mut state, "first");
        add(&mut state, "second");
        state.ui.selected_task_row = 1;

        act(&mut state, Action::MoveSelectedTodo(-1));

        assert_eq!(queue(&state, id).items[0].text, "second");
        assert_eq!(state.ui.selected_task_row, 0, "cursor followed the item");
    }

    #[test]
    fn finished_items_can_be_cleared_without_touching_the_rest() {
        let (mut state, id) = state_with_agent();
        add(&mut state, "done one");
        add(&mut state, "still queued");
        let first = queue(&state, id).items[0].id;
        state.get_session_mut(id).unwrap().todo_queue.mark_running(first);
        state.get_session_mut(id).unwrap().todo_queue.finish_running();

        act(&mut state, Action::ClearCompletedTodos);

        assert_eq!(queue(&state, id).items.len(), 1);
        assert_eq!(queue(&state, id).items[0].text, "still queued");
    }

    #[test]
    fn pausing_holds_the_queue_and_says_so() {
        let (mut state, id) = state_with_agent();
        add(&mut state, "work");

        act(&mut state, Action::ToggleTodoQueuePaused);
        assert!(queue(&state, id).paused);
        assert_eq!(state.ui.task_status(), Some("Queue paused"));

        act(&mut state, Action::ToggleTodoQueuePaused);
        assert!(!queue(&state, id).paused);
    }

    #[test]
    fn an_agents_own_step_is_not_something_you_can_delete() {
        // The fixture's agent has a parsed task list, so a running item gets
        // real Step rows beneath it.
        let (mut state, session_id, _dir) = crate::app::tasks_view::tests::fixture();
        let todo = state
            .get_session_mut(session_id)
            .unwrap()
            .todo_queue
            .add("work");
        state
            .get_session_mut(session_id)
            .unwrap()
            .todo_queue
            .mark_running(todo);

        let rows = tasks_view::rows(&state);
        let step = rows
            .iter()
            .position(|r| matches!(r, TaskRow::Step { .. }))
            .expect("the running item has agent steps under it");
        state.ui.selected_task_row = step;

        act(&mut state, Action::DeleteSelectedTodo);

        assert_eq!(
            queue(&state, session_id).items.len(),
            1,
            "a row that belongs to the agent must not delete your queued item"
        );
        assert_eq!(state.ui.task_status(), Some("That row is the agent's, not yours"));
    }
}
