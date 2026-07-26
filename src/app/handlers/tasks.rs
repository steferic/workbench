//! Tasks pane: navigation, and steering an agent's task list.
//!
//! The list belongs to the agent — it lives in the agent's context, not in a
//! file we can edit — so "add"/"rewrite"/"drop" are delivered as a message
//! typed into that agent's terminal. The pane then shows whatever the agent
//! actually did with it on its next task-list write.

use crate::app::utilities::load_utility_content;
use crate::app::{tasks_view, Action, AppState, InputMode, TaskEdit, TaskRow, UtilityItem};
use anyhow::Result;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::{report_background_error, save_config};

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
            let Some(row) = tasks_view::selected_row(state) else {
                state.ui.set_task_status("No agent selected");
                return Ok(());
            };
            let session_id = row.session_id();
            let running = state
                .get_session(session_id)
                .map(|s| s.status == crate::models::SessionStatus::Running)
                .unwrap_or(false);
            if !running {
                state.ui.set_task_status("That agent is not running");
                return Ok(());
            }

            // Rewrite/Drop need a task to act on; Add only needs the agent.
            let subject = match (&edit, &row) {
                (TaskEdit::Add, _) => String::new(),
                (_, TaskRow::Task { batch, task, .. }) => {
                    match tasks_view::task_at(state, session_id, *batch, *task) {
                        Some(t) => t.subject.clone(),
                        None => return Ok(()),
                    }
                }
                _ => {
                    state.ui.set_task_status("Select a task first");
                    return Ok(());
                }
            };

            state.ui.task_edit = Some((session_id, edit, subject));
            state.ui.input_mode = InputMode::ComposeTaskMessage;
            state.ui.input_buffer.clear();
        }
        Action::SendTaskMessage(text) => {
            let Some((session_id, edit, subject)) = state.ui.task_edit.take() else {
                state.ui.input_mode = InputMode::Normal;
                return Ok(());
            };
            state.ui.input_mode = InputMode::Normal;
            state.ui.input_buffer.clear();

            let message = compose_message(edit, &subject, text.trim());
            if message.is_empty() {
                return Ok(());
            }
            send_to_agent(action_tx, session_id, &message);

            let name = state
                .get_session(session_id)
                .map(|s| s.display_name())
                .unwrap_or_else(|| "agent".to_string());
            if state.data.idle_queue.contains(&session_id) {
                state.ui.set_task_status(format!("Sent to {name}"));
            } else {
                state.ui.set_task_status(format!("Queued for {name} — it is busy"));
            }
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

/// Phrase the edit as an instruction to the agent. These are deliberately
/// plain: the agent owns its list, so we ask rather than assert.
fn compose_message(edit: TaskEdit, subject: &str, text: &str) -> String {
    match edit {
        TaskEdit::Add => {
            if text.is_empty() {
                String::new()
            } else {
                format!("Add this to your task list: {text}")
            }
        }
        TaskEdit::Rewrite => {
            if text.is_empty() {
                String::new()
            } else {
                format!("Change the task \"{subject}\" in your task list to: {text}")
            }
        }
        TaskEdit::Drop => {
            let reason = if text.is_empty() {
                String::new()
            } else {
                format!(" — {text}")
            };
            format!("Drop the task \"{subject}\" from your task list{reason}. Skip it and move on.")
        }
    }
}

/// Type the message into the agent's terminal and submit it. A busy agent
/// keeps it in its composer and picks it up when it next reads input.
fn send_to_agent(action_tx: &mpsc::UnboundedSender<Action>, session_id: Uuid, message: &str) {
    let bytes: Vec<u8> = message.bytes().collect();
    if let Err(err) = action_tx.send(Action::SendInput(session_id, bytes)) {
        report_background_error("failed to queue task message", err);
        return;
    }
    if let Err(err) = action_tx.send(Action::SendInput(session_id, vec![b'\r'])) {
        report_background_error("failed to queue task message newline", err);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_name_the_task_they_act_on() {
        assert_eq!(
            compose_message(TaskEdit::Add, "", "write the migration"),
            "Add this to your task list: write the migration"
        );
        assert_eq!(
            compose_message(TaskEdit::Rewrite, "Write docs", "write docs AND examples"),
            "Change the task \"Write docs\" in your task list to: write docs AND examples"
        );
        assert_eq!(
            compose_message(TaskEdit::Drop, "Write docs", "covered elsewhere"),
            "Drop the task \"Write docs\" from your task list — covered elsewhere. Skip it and move on."
        );
    }

    #[test]
    fn empty_add_and_rewrite_send_nothing() {
        assert!(compose_message(TaskEdit::Add, "", "   ".trim()).is_empty());
        assert!(compose_message(TaskEdit::Rewrite, "Write docs", "").is_empty());
        // A drop needs no reason to be meaningful.
        assert!(!compose_message(TaskEdit::Drop, "Write docs", "").is_empty());
    }
}
