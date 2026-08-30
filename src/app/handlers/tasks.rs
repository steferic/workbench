//! Tasks pane: editing the selected agent's TODO queue.
//!
//! These items are workbench's own state, so unlike the agent's task list they
//! can simply be edited. Dispatch is not done here — `app::todo_dispatch`
//! decides when an item may go out.

use crate::app::utilities::load_utility_content;
use crate::app::objectives_view;
use crate::app::{
    tasks_view, Action, AppState, InputMode, TaskEdit, UtilityItem, UtilitySection,
};
use anyhow::Result;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::save_config;

fn global_config(state: &AppState) -> crate::persistence::GlobalConfig {
    crate::persistence::GlobalConfig {
        banner_visible: state.ui.banner_visible,
        left_panel_ratio: state.ui.layout.left_panel_ratio,
        workspace_ratio: state.ui.layout.workspace_ratio,
        sessions_ratio: state.ui.layout.sessions_ratio,
        tasks_ratio: state.ui.layout.tasks_ratio,
        output_split_ratio: state.ui.layout.output_split_ratio,
        theme_mode: state.ui.theme_mode,
    }
}

fn toggle_banner(state: &mut AppState) {
    state.ui.banner_visible = !state.ui.banner_visible;
    // The next draw computes the new pane geometry; resize PTYs immediately
    // after that frame so terminal applications use the reclaimed row.
    crate::app::pty_ops::request_pty_resize(state);
}

/// Keep the pane's row cursor meaningful as the Sessions pane cursor moves:
/// a row index into one agent's list means nothing in another's.
pub fn sync_selection(state: &mut AppState) {
    let agent = tasks_view::selected_agent(state).map(|a| a.session_id);
    if state.ui.tasks_agent != agent {
        state.ui.tasks_agent = agent;
        state.ui.selected_task_row = 0;
    }
    // Deleting a manager, or switching project, can strand this cursor past
    // the end of a list nobody pressed Tab on.
    crate::app::managers_view::clamp(state);
    crate::app::desk_view::clamp(state);
}

pub fn handle_task_action(
    state: &mut AppState,
    action: Action,
    action_tx: &mpsc::UnboundedSender<Action>,
) -> Result<()> {
    match action {
        // j/k mean "down/up the list in front of me", so they follow the tab
        // rather than each list owning its own pair of keys.
        Action::SelectNextTask => {
            if state.ui.selected_tasks_tab == crate::app::TasksTab::Desk {
                let count = crate::app::desk_view::rows(state).len();
                if count > 0 {
                    state.ui.selected_desk_row =
                        (state.ui.selected_desk_row + 1).min(count - 1);
                }
                return Ok(());
            }
            if state.ui.selected_tasks_tab == crate::app::TasksTab::Managers {
                let count = crate::app::managers_view::rows(state).len();
                if count > 0 {
                    state.ui.selected_manager = (state.ui.selected_manager + 1).min(count - 1);
                }
                return Ok(());
            }
            // A row taller than the pane is read through before it is left:
            // j walks the hidden lines into view first, like a pager, and
            // only then moves to the next row.
            if state.ui.objective_overflow > 0 {
                state.ui.objective_scroll += 1;
                return Ok(());
            }
            let count = objectives_view::rows(state).len();
            if count > 0 {
                let next = (state.ui.selected_objective + 1).min(count - 1);
                if next != state.ui.selected_objective {
                    state.ui.selected_objective = next;
                    state.ui.objective_scroll = 0;
                }
            }
        }
        Action::SelectPrevTask => {
            if state.ui.selected_tasks_tab == crate::app::TasksTab::Desk {
                state.ui.selected_desk_row = state.ui.selected_desk_row.saturating_sub(1);
                return Ok(());
            }
            if state.ui.selected_tasks_tab == crate::app::TasksTab::Managers {
                state.ui.selected_manager = state.ui.selected_manager.saturating_sub(1);
                return Ok(());
            }
            if state.ui.objective_scroll > 0 {
                state.ui.objective_scroll -= 1;
                return Ok(());
            }
            let prev = state.ui.selected_objective.saturating_sub(1);
            if prev != state.ui.selected_objective {
                state.ui.selected_objective = prev;
                state.ui.objective_scroll = 0;
            }
        }
        Action::ToggleTasksTab => {
            state.ui.selected_tasks_tab = state.ui.selected_tasks_tab.toggle();
            state.ui.selected_task_row = 0;
            state.ui.objective_scroll = 0;
            clamp_objective_cursor(state);
        }
        // The desk acts wherever the row lives — the whole point is not
        // having to travel to a decision before making it.
        Action::DeskDecide(yes) => {
            match crate::app::desk_view::selected(state) {
                Some(crate::app::desk_view::DeskRow::PendingProposal {
                    workspace_id,
                    proposal_id,
                    ..
                }) => {
                    if yes && assignment_first(state, workspace_id, proposal_id) {
                        return Ok(());
                    }
                    match decide_proposal(state, workspace_id, proposal_id, yes, action_tx) {
                        Ok(message) | Err(message) => state.ui.set_task_status(message),
                    }
                }
                Some(crate::app::desk_view::DeskRow::ProposedCheck {
                    workspace_id,
                    objective_id,
                    ..
                }) => {
                    decide_check(state, workspace_id, objective_id, yes);
                }
                Some(crate::app::desk_view::DeskRow::NeedsUser {
                    workspace_id,
                    proposal_id,
                    ..
                }) => {
                    // Yes re-arms the loop with a fresh set of rounds — the
                    // user's approval buys what the manager's could not. No
                    // closes the review for good. Which is which lives in
                    // `decide_needs_user`, shared with the overlay and the
                    // phone.
                    match decide_needs_user(state, workspace_id, proposal_id, yes, action_tx) {
                        Ok(message) | Err(message) => state.ui.set_task_status(message),
                    }
                }
                Some(crate::app::desk_view::DeskRow::BlockedAgent { .. }) => {
                    state
                        .ui
                        .set_task_status("Open it (Enter) and answer the agent directly");
                }
                None => {}
            }
            crate::app::desk_view::clamp(state);
        }
        Action::OpenDetail => {
            let Some(workspace_id) = state.selected_workspace().map(|ws| ws.id) else {
                return Ok(());
            };
            state.ui.detail = objectives_view::selected(state).map(|row| match row {
                crate::app::objectives_view::ObjectiveRow::Proposal { id } => {
                    crate::app::DetailTarget::Proposal {
                        workspace_id,
                        proposal_id: id,
                    }
                }
                crate::app::objectives_view::ObjectiveRow::Objective { id, .. } => {
                    crate::app::DetailTarget::Objective {
                        workspace_id,
                        objective_id: id,
                    }
                }
            });
        }
        Action::CloseDetail => {
            state.ui.detail = None;
        }
        Action::DeskDecideDetail(yes) => {
            let Some(crate::app::DetailTarget::Proposal {
                workspace_id,
                proposal_id,
            }) = state.ui.detail
            else {
                state.ui.detail = None;
                return Ok(());
            };
            state.ui.detail = None;
            let phase = state
                .data
                .workspaces
                .iter()
                .find(|ws| ws.id == workspace_id)
                .and_then(|ws| ws.proposals.iter().find(|p| p.id == proposal_id))
                .map(|p| (p.is_pending(), p.review));
            match phase {
                Some((true, _)) => {
                    if yes && assignment_first(state, workspace_id, proposal_id) {
                        return Ok(());
                    }
                    match decide_proposal(state, workspace_id, proposal_id, yes, action_tx) {
                        Ok(message) | Err(message) => state.ui.set_task_status(message),
                    }
                }
                Some((false, Some(crate::models::ReviewPhase::NeedsUser))) => {
                    match decide_needs_user(state, workspace_id, proposal_id, yes, action_tx) {
                        Ok(message) | Err(message) => state.ui.set_task_status(message),
                    }
                }
                _ => state.ui.set_task_status("Nothing to decide here"),
            }
        }
        Action::DeskOpen => {
            if let Some(row) = crate::app::desk_view::selected(state) {
                open_desk_row(state, row);
            }
        }
        Action::FocusSelectedTaskAgent => {
            // Enter means "open what is under the cursor", and which list that
            // is depends on the tab.
            if state.ui.selected_tasks_tab == crate::app::TasksTab::Managers {
                if let Some(row) = crate::app::managers_view::selected(state) {
                    state.set_active_session_id(Some(row.session_id));
                    state.ui.focus = crate::app::FocusPanel::OutputPane;
                }
                return Ok(());
            }
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
            // An objective in progress owns the buffer: it was opened from the
            // Objectives tab and has nothing to do with any session's queue.
            if let Some((workspace_id, editing)) = state.ui.objective_edit.take() {
                state.ui.input_mode = InputMode::Normal;
                state.ui.input_buffer.clear();
                let text = text.trim().to_string();
                if text.is_empty() {
                    return Ok(());
                }
                let Some(ws) = state
                    .data
                    .workspaces
                    .iter_mut()
                    .find(|ws| ws.id == workspace_id)
                else {
                    return Ok(());
                };
                match editing.and_then(|id| ws.objectives.iter_mut().find(|o| o.id == id)) {
                    Some(objective) => {
                        objective.text = text;
                        state.ui.set_task_status("Objective updated");
                    }
                    None => {
                        let objective = crate::models::Objective::new(text);
                        let id = objective.id;
                        ws.objectives.push(objective);
                        let n = ws.objectives.len();
                        focus_objective_row(state, id);
                        state.ui.set_task_status(format!("Objective added — {n} total"));
                    }
                }
                super::save_state(state, "failed to save objectives");
                return Ok(());
            }

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
        Action::EditObjective(rewrite) => {
            let Some(workspace_id) = state.selected_workspace().map(|ws| ws.id) else {
                state.ui.set_task_status("Open a project first");
                return Ok(());
            };
            let existing = selected_objective_id(state);
            if rewrite && existing.is_none() {
                state.ui.set_task_status("Select an objective first");
                return Ok(());
            }
            state.ui.input_buffer = if rewrite {
                selected_objective(state).map(|o| o.text.clone()).unwrap_or_default()
            } else {
                String::new()
            };
            state.ui.objective_edit = Some((workspace_id, rewrite.then_some(existing).flatten()));
            state.ui.input_mode = InputMode::ComposeTaskMessage;
        }
        Action::DeleteObjective => {
            let Some(id) = selected_objective_id(state) else {
                return Ok(());
            };
            if let Some(ws) = state.selected_workspace_mut() {
                ws.objectives.retain(|o| o.id != id);
            }
            clamp_objective_cursor(state);
            state.ui.set_task_status("Objective removed");
            super::save_state(state, "failed to save objectives");
        }
        Action::CycleObjectiveState => {
            let Some(id) = selected_objective_id(state) else {
                return Ok(());
            };
            let mut label = "";
            if let Some(ws) = state.selected_workspace_mut() {
                if let Some(objective) = ws.objectives.iter_mut().find(|o| o.id == id) {
                    objective.state = objective.state.next();
                    label = objective.state.label();
                }
            }
            state.ui.set_task_status(format!("Objective {label}"));
            super::save_state(state, "failed to save objectives");
        }
        Action::MoveObjective(delta) => {
            let Some(id) = selected_objective_id(state) else {
                return Ok(());
            };
            if let Some(ws) = state.selected_workspace_mut() {
                crate::models::move_objective(&mut ws.objectives, id, delta);
            }
            // Follow the item, or the cursor lands on whatever was pushed out
            // of the way.
            focus_objective_row(state, id);
            super::save_state(state, "failed to save objectives");
        }
        Action::ApproveProposal => {
            // `a` means "yes, this" for whichever row the cursor is on: a
            // proposal becomes work, a proposed check becomes the thing work
            // will be held to.
            if objectives_view::selected(state).and_then(|r| r.objective_id()).is_some() {
                approve_selected_check(state);
            } else {
                approve_selected_proposal(state, action_tx);
            }
        }
        Action::DeclineProposal => {
            let Some(id) = objectives_view::selected(state).and_then(|r| r.proposal_id()) else {
                state.ui.set_task_status("Select a proposal first");
                return Ok(());
            };
            let Some(workspace) = state.selected_workspace().map(|ws| ws.id) else {
                return Ok(());
            };
            match decide_proposal(state, workspace, id, false, action_tx) {
                Ok(message) | Err(message) => state.ui.set_task_status(message),
            }
            objectives_view::clamp(state);
        }
        Action::VerificationFinished {
            workspace_id,
            proposal_id,
            baseline,
            run,
            mark,
        } => {
            record_verification(state, workspace_id, proposal_id, baseline, *run, mark);
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
            if state.ui.utility_section == UtilitySection::Themes {
                state.ui.theme_mode = state.ui.selected_theme;
                let config = global_config(state);
                save_config(state, &config, "failed to save theme config");
            } else if state.ui.utility_section == UtilitySection::Utilities {
                if state.ui.selected_utility == UtilityItem::ToggleBanner {
                    toggle_banner(state);
                    let config = global_config(state);
                    save_config(state, &config, "failed to save banner config");
                } else {
                    load_utility_content(state, action_tx);
                    state.set_active_session_id(None);
                }
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
        // The journal is worth remembering even when the id is not readable:
        // it is what lets a stopped agent still be read (see `remote`).
        let journal = match tracker.source() {
            Some(crate::agent_tasks::Source::File(path)) => Some(path.clone()),
            _ => None,
        };
        let Some(id) = tracker.provider_session_id() else {
            continue;
        };
        if let Some(session) = state.get_session_mut(*session_id) {
            if session.provider_session_id.as_deref() != Some(id.as_str()) {
                session.provider_session_id = Some(id);
                changed = true;
            }
            if journal.is_some() && session.journal_path != journal {
                session.journal_path = journal;
                changed = true;
            }
        }
    }
    if changed {
        super::save_state(state, "failed to save agent conversation ids");
    }
}

/// Ask, off the event loop, what a check says right now.
///
/// A suite takes minutes and the UI has other agents to draw, so this hands
/// the work to a blocking thread and hears back as an action. The repository
/// is marked in the same breath: comparing the two marks is what decides
/// whether anything actually happened.
pub(crate) fn start_verification(
    state: &AppState,
    workspace_id: uuid::Uuid,
    proposal_id: uuid::Uuid,
    baseline: bool,
    check: crate::models::Verification,
    dir: std::path::PathBuf,
    action_tx: &mpsc::UnboundedSender<Action>,
) {
    let _ = state;
    let tx = action_tx.clone();
    tokio::task::spawn_blocking(move || {
        let mark = crate::app::verify::mark(&dir);
        let run = crate::app::verify::run(
            &check.command,
            &dir,
            std::time::Duration::from_secs(check.timeout_secs),
        );
        let _ = tx.send(Action::VerificationFinished {
            workspace_id,
            proposal_id,
            baseline,
            run: Box::new(run),
            mark,
        });
    });
}

/// File a finished run against its proposal, and judge once both are in.
fn record_verification(
    state: &mut AppState,
    workspace_id: uuid::Uuid,
    proposal_id: uuid::Uuid,
    baseline: bool,
    run: crate::models::VerificationRun,
    mark: crate::models::RepoMark,
) {
    let Some(workspace) = state
        .data
        .workspaces
        .iter_mut()
        .find(|ws| ws.id == workspace_id)
    else {
        return;
    };
    let Some(proposal) = workspace.proposals.iter_mut().find(|p| p.id == proposal_id) else {
        return;
    };

    let outcome = run.outcome;
    if baseline {
        // The mark here is taken *before* the check runs, which is what the
        // "after" is later compared against.
        proposal.before = Some(mark);
        proposal.baseline = Some(run);
        crate::logger::info(format!("baseline for a proposal: {}", outcome.label()));
        super::save_state(state, "failed to save a baseline");
        return;
    }

    proposal.after = Some(mark);
    proposal.result = Some(run);

    let changed = match (proposal.after.as_ref(), proposal.before.as_ref()) {
        (Some(after), Some(before)) => after.changed_from(before),
        // Nothing to compare against: assume the work happened rather than
        // reject it on a comparison that was never made.
        _ => true,
    };
    let verdict = crate::models::judge(
        proposal.baseline.as_ref(),
        proposal.result.as_ref().expect("just set"),
        changed,
    );
    crate::logger::info(format!("verdict: {} — {}", verdict.label(), verdict.why()));
    let status = format!("{} — {}", verdict.label(), verdict.why());
    proposal.verdict = Some(verdict);
    state.ui.set_task_status(status);
    super::save_state(state, "failed to save a verdict");
    // The verdict was the last thing the review turn waited on.
    queue_review_turn(state, workspace_id, proposal_id);
}

/// The agent's turn on a proposal ended: record where the repository landed,
/// move the proposal to AwaitingReview, and hand its manager a review turn.
///
/// The path for work with no approved check — the marks stand in for the
/// verdict — and for records stranded from before the lifecycle existed.
pub(crate) fn finish_into_review(
    state: &mut AppState,
    workspace_id: uuid::Uuid,
    proposal_id: uuid::Uuid,
    _action_tx: &mpsc::UnboundedSender<Action>,
) {
    let dir = state
        .data
        .workspaces
        .iter()
        .find(|ws| ws.id == workspace_id)
        .map(|ws| ws.path.clone());
    let Some(workspace) = state
        .data
        .workspaces
        .iter_mut()
        .find(|ws| ws.id == workspace_id)
    else {
        return;
    };
    let Some(proposal) = workspace.proposals.iter_mut().find(|p| p.id == proposal_id) else {
        return;
    };
    proposal.work_finished();
    if proposal.after.is_none() {
        if let Some(dir) = dir {
            proposal.after = Some(crate::app::verify::mark(&dir));
        }
    }
    super::save_state(state, "failed to save a finished proposal");
    queue_review_turn(state, workspace_id, proposal_id);
}

/// Queue the review turn on the manager that proposed the work.
///
/// The whole packet an accept/request-changes decision needs: what was asked,
/// where the repository moved, what the check said if one ran, and the exact
/// call to answer with. If the manager is gone, the proposal goes to the user
/// instead — work reviewed by nobody must not read as done.
pub(crate) fn queue_review_turn(
    state: &mut AppState,
    workspace_id: uuid::Uuid,
    proposal_id: uuid::Uuid,
) {
    let Some(proposal) = state
        .data
        .workspaces
        .iter()
        .find(|ws| ws.id == workspace_id)
        .and_then(|ws| ws.proposals.iter().find(|p| p.id == proposal_id))
        .cloned()
    else {
        return;
    };
    if !proposal.awaiting_review() {
        return;
    }

    let Some(manager_session) = crate::remote::session_for(state, &proposal.manager) else {
        if let Some(stored) = state
            .data
            .workspaces
            .iter_mut()
            .find(|ws| ws.id == workspace_id)
            .and_then(|ws| ws.proposals.iter_mut().find(|p| p.id == proposal_id))
        {
            stored.needs_user("its manager is no longer running".to_string());
        }
        super::save_state(state, "failed to save an orphaned review");
        return;
    };

    let repo_moved = match (proposal.after.as_ref(), proposal.before.as_ref()) {
        (Some(after), Some(before)) => {
            if after.changed_from(before) { "the repository moved" } else { "NOTHING CHANGED in the repository" }
        }
        _ => "no before/after comparison was possible",
    };
    let verification = match (&proposal.verdict, &proposal.result) {
        (Some(verdict), Some(run)) => format!(
            "{} — {}\nlast output:\n{}",
            verdict.label(),
            verdict.why(),
            run.tail.trim()
        ),
        _ => "no approved check ran".to_string(),
    };
    let agent = proposal.agent.clone().unwrap_or_else(|| "unknown".into());
    let text = format!(
        "REVIEW TURN — round {}/{} for proposal {}\n\n\
You proposed, and it was approved: {}\n\
Agent: {} (read its work: `workbench transcript {}` and the diff: `git diff` / `git log`)\n\
Repository: {}\n\
Verification: {}\n\n\
Judge whether the work as delivered does what the proposal asked — no more, no wider. \
Then answer on the control socket with exactly one of:\n\
  {{\"method\":\"manager.review\",\"params\":{{\"manager\":\"{}\",\"proposal\":\"{}\",\"outcome\":\"accept\"}}}}\n\
  {{\"method\":\"manager.review\",\"params\":{{\"manager\":\"{}\",\"proposal\":\"{}\",\"outcome\":\"request_changes\",\"findings\":\"exact, bounded corrections within the original scope\"}}}}\n\
  {{\"method\":\"manager.review\",\"params\":{{\"manager\":\"{}\",\"proposal\":\"{}\",\"outcome\":\"needs_user\",\"findings\":\"what a person must decide\"}}}}\n\n\
Corrections may only tighten this same job; new or wider work takes a new proposal. \
After round {}/{} the loop stops and the user takes over.",
        proposal.review_rounds + 1,
        crate::models::MAX_REVIEW_ROUNDS,
        proposal.id,
        proposal.instruction.trim(),
        agent,
        agent,
        repo_moved,
        verification,
        proposal.manager,
        proposal.id,
        proposal.manager,
        proposal.id,
        proposal.manager,
        proposal.id,
        crate::models::MAX_REVIEW_ROUNDS,
        crate::models::MAX_REVIEW_ROUNDS,
    );

    if let Some(session) = state.get_session_mut(manager_session) {
        session.todo_queue.add(text);
        crate::logger::info(format!(
            "review turn queued for manager {} (proposal {})",
            proposal.manager, proposal_id
        ));
    }
    super::save_state(state, "failed to save a review turn");
}

/// Agree to the check a manager suggested for the objective under the cursor.
///
/// Until this happens the command is a suggestion: shown, not trusted, and
/// not enough to let anything run against that objective unattended.
/// Approve or drop a proposed check, wherever its objective lives.
pub(crate) fn decide_check(
    state: &mut AppState,
    workspace_id: uuid::Uuid,
    objective_id: uuid::Uuid,
    yes: bool,
) {
    let Some(objective) = state
        .data
        .workspaces
        .iter_mut()
        .find(|ws| ws.id == workspace_id)
        .and_then(|ws| ws.objectives.iter_mut().find(|o| o.id == objective_id))
    else {
        return;
    };
    match objective.done_when.as_mut() {
        Some(check) if check.proposed => {
            if yes {
                check.proposed = false;
                let command = check.command.clone();
                state.ui.set_task_status(format!("Check approved: {command}"));
            } else {
                objective.done_when = None;
                state.ui.set_task_status("Check dropped");
            }
            super::save_state(state, "failed to save the check decision");
        }
        _ => {}
    }
}

/// Whether a proposal is still parked on the user.
///
/// Both decisions a "needs you" row offers are guarded on this, and for the
/// same reason: the row can be acted on twice. The desk and the phone show it
/// at once, a tap can be in flight while a key lands, and a phone that
/// retries a post has no idea the first one arrived.
pub(crate) fn is_parked_on_user(state: &AppState, workspace_id: uuid::Uuid, proposal_id: uuid::Uuid) -> bool {
    state
        .data
        .workspaces
        .iter()
        .find(|ws| ws.id == workspace_id)
        .and_then(|ws| ws.proposals.iter().find(|p| p.id == proposal_id))
        .is_some_and(|p| p.review == Some(crate::models::ReviewPhase::NeedsUser))
}

/// The user overriding an exhausted or punted review loop: the last findings
/// go back to the agent as a fresh job, and the round counter starts over —
/// the user's approval buys what the manager's could not.
///
/// Guarded on the proposal still being parked. Without that, a second arrival
/// — a double tap, a phone retrying a post it never saw answered, a key at
/// the desk racing a tap — re-armed work that was already working and queued
/// the same job twice.
pub(crate) fn rearm_review(
    state: &mut AppState,
    workspace_id: uuid::Uuid,
    proposal_id: uuid::Uuid,
    _action_tx: &mpsc::UnboundedSender<Action>,
) -> Result<String, String> {
    let Some(snapshot) = state
        .data
        .workspaces
        .iter()
        .find(|ws| ws.id == workspace_id)
        .and_then(|ws| ws.proposals.iter().find(|p| p.id == proposal_id))
        .cloned()
    else {
        return Err("No such proposal".to_string());
    };
    if snapshot.review != Some(crate::models::ReviewPhase::NeedsUser) {
        return Err("That review is not waiting on you".to_string());
    }
    let agent = snapshot
        .agent
        .clone()
        .ok_or_else(|| "No agent named on that proposal".to_string())?;
    let session_id = crate::remote::session_for(state, &agent)
        .ok_or_else(|| format!("No agent {agent} here any more"))?;
    let text = match &snapshot.findings {
        Some(findings) => format!(
            "The user re-approved this after review stalled. Address exactly:\n{findings}\n\n\
Original job: {}",
            snapshot.instruction
        ),
        None => snapshot.instruction.clone(),
    };
    let Some(session) = state.get_session_mut(session_id) else {
        return Err(format!("No agent {agent} here any more"));
    };
    let todo = session.todo_queue.add(text);
    if let Some(stored) = state
        .data
        .workspaces
        .iter_mut()
        .find(|ws| ws.id == workspace_id)
        .and_then(|ws| ws.proposals.iter_mut().find(|p| p.id == proposal_id))
    {
        stored.review_rounds = 0;
        stored.todo_id = Some(todo);
        stored.review = Some(crate::models::ReviewPhase::Working);
    }
    super::save_state(state, "failed to save the re-arm");
    Ok(format!("Re-armed and queued for {agent}"))
}

/// The other half of a "needs you" decision: the user ending a review the
/// manager punted, rather than buying it another lap.
///
/// Not `decide_proposal`. That one guards on `is_pending`, and a punted
/// review is not pending — it was approved, handed to an agent and worked on
/// before it came back. Routing a decline through there answered "Already
/// decided" and left the row sitting on the desk, which is the one thing a
/// decline must not do.
///
/// Ends the review and queues nothing, recording an outcome of its own:
/// `ReviewPhase::Closed`, which is terminal and which every surface labels as
/// closed rather than inferring something cheerier.
///
/// It stays `Approved` on purpose — the work was approved and it did happen,
/// and the objective's ledger counts an approved proposal's turns and its
/// manager's review turn. Clearing `review` instead would have been the
/// cheaper change and a dishonest one: `None` on an approved proposal is what
/// a record from before the field existed looks like, and it rendered as
/// "queued" in the objectives tab and as plain "approved" on the phone —
/// a job the user had just stopped, still advertising itself as running.
pub(crate) fn decline_needs_user(
    state: &mut AppState,
    workspace_id: uuid::Uuid,
    proposal_id: uuid::Uuid,
) -> Result<String, String> {
    if !is_parked_on_user(state, workspace_id, proposal_id) {
        return Err("That review is not waiting on you".to_string());
    }
    let Some(stored) = state
        .data
        .workspaces
        .iter_mut()
        .find(|ws| ws.id == workspace_id)
        .and_then(|ws| ws.proposals.iter_mut().find(|p| p.id == proposal_id))
    else {
        return Err("No such proposal".to_string());
    };
    stored.review = Some(crate::models::ReviewPhase::Closed);
    super::save_state(state, "failed to save the decision");
    Ok("Review closed".to_string())
}

/// Which decision a "needs you" row actually takes, in one place.
///
/// Three callers offer this row — the desk, its detail overlay, and the
/// phone — and each used to spell out the yes and the no itself. They did not
/// agree for long: all three sent the no to a helper that refuses anything
/// already approved.
pub(crate) fn decide_needs_user(
    state: &mut AppState,
    workspace_id: uuid::Uuid,
    proposal_id: uuid::Uuid,
    yes: bool,
    action_tx: &mpsc::UnboundedSender<Action>,
) -> Result<String, String> {
    if yes {
        rearm_review(state, workspace_id, proposal_id, action_tx)
    } else {
        decline_needs_user(state, workspace_id, proposal_id)
    }
}

/// Jump to whatever a desk row is about: the agent's terminal, or the
/// objectives tab of the project it lives in, cursor on the item.
fn open_desk_row(state: &mut AppState, row: crate::app::desk_view::DeskRow) {
    use crate::app::desk_view::DeskRow;
    match row {
        DeskRow::BlockedAgent { session_id, .. } => {
            if let Some(idx) = state
                .data
                .workspaces
                .iter()
                .position(|ws| Some(ws.id) == state.workspace_id_for_session(session_id))
            {
                state.ui.selected_workspace_idx = idx;
            }
            state.set_active_session_id(Some(session_id));
            state.ui.focus = crate::app::FocusPanel::OutputPane;
        }
        // A proposal's Enter is the decision context, not a journey: the
        // detail overlay carries everything the manager saw.
        DeskRow::NeedsUser { workspace_id, proposal_id, .. }
        | DeskRow::PendingProposal { workspace_id, proposal_id, .. } => {
            state.ui.detail = Some(crate::app::DetailTarget::Proposal {
                workspace_id,
                proposal_id,
            });
        }
        DeskRow::ProposedCheck { workspace_id, objective_id, .. } => {
            focus_objectives(state, workspace_id);
            let rows = objectives_view::rows(state);
            if let Some(at) = rows.iter().position(|r| r.objective_id() == Some(objective_id)) {
                state.ui.selected_objective = at;
            }
        }
    }
}

fn focus_objectives(state: &mut AppState, workspace_id: uuid::Uuid) {
    if let Some(idx) = state
        .data
        .workspaces
        .iter()
        .position(|ws| ws.id == workspace_id)
    {
        state.ui.selected_workspace_idx = idx;
    }
    state.ui.selected_tasks_tab = crate::app::TasksTab::Objectives;
    state.ui.objective_scroll = 0;
}

fn approve_selected_check(state: &mut AppState) {
    let Some(id) = objectives_view::selected(state).and_then(|r| r.objective_id()) else {
        return;
    };
    let mut approved = None;
    if let Some(ws) = state.selected_workspace_mut() {
        if let Some(objective) = ws.objectives.iter_mut().find(|o| o.id == id) {
            match objective.done_when.as_mut() {
                Some(check) if check.proposed => {
                    check.proposed = false;
                    approved = Some(check.command.clone());
                }
                Some(_) => {}
                None => {}
            }
        }
    }
    match approved {
        Some(command) => {
            state.ui.set_task_status(format!("Check approved: {command}"));
            super::save_state(state, "failed to approve a check");
        }
        None => state
            .ui
            .set_task_status("No check proposed for that objective"),
    }
}

/// Turn the selected proposal into work.
///
/// This is the only place a manager's suggestion becomes something an agent
/// will see, and it is reached by a keypress. Everything it refuses, it
/// refuses out loud: a proposal that names nobody, an agent that has gone, or
/// one that is itself a manager has no sensible reading, and silently doing
/// something adjacent would be worse than saying so.
fn approve_selected_proposal(state: &mut AppState, action_tx: &mpsc::UnboundedSender<Action>) {
    let Some(proposal) = objectives_view::selected_proposal(state).map(|p| p.id) else {
        state.ui.set_task_status("Select a proposal first");
        return;
    };
    let Some(workspace) = state.selected_workspace().map(|ws| ws.id) else {
        return;
    };
    if assignment_first(state, workspace, proposal) {
        return;
    }
    match decide_proposal(state, workspace, proposal, true, action_tx) {
        Ok(message) | Err(message) => state.ui.set_task_status(message),
    }
    objectives_view::clamp(state);
}

/// An approval that names nobody first asks who: sets up the Assign Agent
/// picker and reports whether it did. "Approve" silently doing nothing is
/// how this flow was discovered broken.
pub(crate) fn assignment_first(
    state: &mut AppState,
    workspace_id: uuid::Uuid,
    proposal_id: uuid::Uuid,
) -> bool {
    let unassigned = state
        .data
        .workspaces
        .iter()
        .find(|ws| ws.id == workspace_id)
        .and_then(|ws| ws.proposals.iter().find(|p| p.id == proposal_id))
        .is_some_and(|p| p.is_pending() && p.agent.is_none());
    if unassigned {
        state.ui.assign = Some((workspace_id, proposal_id));
        state.ui.detail = None;
        state.ui.input_mode = crate::app::InputMode::AssignAgent;
        state
            .ui
            .set_task_status("No agent named — pick who does it");
    }
    unassigned
}

/// Approve or decline one proposal, wherever it lives.
///
/// The one implementation behind the TUI's `a`/`x` and the phone's buttons:
/// approving queues the instruction for the named agent and runs the
/// baseline check, exactly as pressing `a` does. By id and workspace rather
/// than by cursor, because the phone has no cursor and the proposal need not
/// be in the selected workspace.
pub(crate) fn decide_proposal(
    state: &mut AppState,
    workspace_id: uuid::Uuid,
    proposal_id: uuid::Uuid,
    approve: bool,
    action_tx: &mpsc::UnboundedSender<Action>,
) -> Result<String, String> {
    let proposal = state
        .data
        .workspaces
        .iter()
        .find(|ws| ws.id == workspace_id)
        .and_then(|ws| ws.proposals.iter().find(|p| p.id == proposal_id))
        .cloned()
        .ok_or_else(|| "No such proposal".to_string())?;
    if !proposal.is_pending() {
        return Err("Already decided".to_string());
    }

    if !approve {
        if let Some(ws) = state
            .data
            .workspaces
            .iter_mut()
            .find(|ws| ws.id == workspace_id)
        {
            if let Some(stored) = ws.proposals.iter_mut().find(|p| p.id == proposal_id) {
                stored.decline();
            }
        }
        super::save_state(state, "failed to save the decision");
        return Ok("Declined".to_string());
    }

    let target = proposal
        .agent
        .clone()
        .ok_or_else(|| "That proposal names no agent".to_string())?;
    let session_id = crate::remote::session_for(state, &target)
        .ok_or_else(|| format!("No agent {target} here any more"))?;
    // The hierarchy stays one level deep, even by hand: approving a manager
    // into another manager's queue is not a thing to allow by accident.
    let directable = state
        .get_session(session_id)
        .map(|s| s.agent_type.is_directable())
        .unwrap_or(false);
    if !directable {
        return Err(format!("{target} is not an agent that takes work"));
    }

    let Some(session) = state.get_session_mut(session_id) else {
        return Err(format!("No agent {target} here any more"));
    };
    let todo_id = session.todo_queue.add(proposal.instruction.clone());
    let left = session.todo_queue.pending_count();

    if let Some(ws) = state
        .data
        .workspaces
        .iter_mut()
        .find(|ws| ws.id == workspace_id)
    {
        if let Some(stored) = ws.proposals.iter_mut().find(|p| p.id == proposal_id) {
            stored.approve(todo_id);
        }
    }

    // Ask the check what it says *now*, before the agent has touched
    // anything. Without this the agent is credited for a suite that was
    // already green, or blamed for one that was already red.
    if let Some((check, dir, ws_id)) = baseline_for(state, &proposal, session_id) {
        start_verification(state, ws_id, proposal.id, true, check, dir, action_tx);
    }

    super::save_state(state, "failed to save the approval");
    Ok(format!("Queued for {target} — {left} to run"))
}

/// The check to run for a proposal, and where to run it.
///
/// `None` when the objective it serves has no approved check — which is the
/// ordinary case early on, and means the work simply arrives unverified
/// rather than being blocked.
pub(crate) fn baseline_for(
    state: &AppState,
    proposal: &crate::models::Proposal,
    session_id: uuid::Uuid,
) -> Option<(crate::models::Verification, std::path::PathBuf, uuid::Uuid)> {
    let objective_id = proposal.objective_id?;
    let workspace_id = state.workspace_id_for_session(session_id)?;
    let workspace = state
        .data
        .workspaces
        .iter()
        .find(|ws| ws.id == workspace_id)?;
    let check = workspace
        .objectives
        .iter()
        .find(|o| o.id == objective_id)?
        .done_when
        .as_ref()
        // A command a manager suggested is not one you agreed to run.
        .filter(|check| !check.proposed && !check.command.trim().is_empty())?
        .clone();
    // A worktree-isolated agent is checked where it works, so its diff and its
    // check describe the same tree.
    let dir = state
        .get_session(session_id)
        .and_then(|s| s.worktree_path.clone())
        .unwrap_or_else(|| workspace.path.clone());
    Some((check, dir, workspace_id))
}

/// The objective under the cursor — `None` when the cursor is on a proposal.
pub(crate) fn selected_objective(state: &AppState) -> Option<&crate::models::Objective> {
    objectives_view::selected_objective(state)
}

pub(crate) fn selected_objective_id(state: &AppState) -> Option<uuid::Uuid> {
    selected_objective(state).map(|o| o.id)
}

fn clamp_objective_cursor(state: &mut AppState) {
    objectives_view::clamp(state);
}

/// Put the cursor on a given objective's row.
///
/// Rows are objectives and proposals interleaved, so an objective's position
/// in the list is not its position on screen — following the item after it
/// moves means finding its row, not its index.
fn focus_objective_row(state: &mut AppState, id: uuid::Uuid) {
    if let Some(at) = objectives_view::rows(state)
        .iter()
        .position(|row| row.objective_id() == Some(id))
    {
        state.ui.selected_objective = at;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tasks_view::TaskRow;
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

    /// A world with one proposal parked on the user, worked on by `id`.
    fn state_with_parked_review() -> (AppState, Uuid, Uuid) {
        use crate::models::{Proposal, ProposalState};

        let (mut state, session_id) = state_with_agent();
        let mut parked = Proposal::new("m1", "tidy the parser");
        parked.state = ProposalState::Approved;
        parked.agent = Some(crate::models::Session::short_id_of(session_id));
        parked.review_rounds = 3;
        parked.needs_user("could not tell if the migration ran".into());
        let proposal_id = parked.id;
        state.data.workspaces[0].proposals.push(parked);
        (state, session_id, proposal_id)
    }

    fn desk_kinds(state: &AppState) -> Vec<&'static str> {
        crate::app::desk_view::rows(state)
            .iter()
            .map(|row| match row {
                crate::app::desk_view::DeskRow::BlockedAgent { .. } => "blocked",
                crate::app::desk_view::DeskRow::NeedsUser { .. } => "needs_user",
                crate::app::desk_view::DeskRow::PendingProposal { .. } => "pending",
                crate::app::desk_view::DeskRow::ProposedCheck { .. } => "check",
            })
            .collect()
    }

    /// The bug this fixes: a "needs you" row's no went to `decide_proposal`,
    /// which refuses anything not pending. A punted review is approved, so
    /// every decline answered "Already decided" and the row stayed on the
    /// desk — the one thing a decline must not do.
    #[test]
    fn declining_a_parked_review_takes_it_off_the_desk() {
        let (mut state, session_id, proposal_id) = state_with_parked_review();
        let workspace_id = state.data.workspaces[0].id;
        let (tx, _rx) = mpsc::unbounded_channel();
        assert_eq!(desk_kinds(&state), ["needs_user"]);

        let outcome = decide_needs_user(&mut state, workspace_id, proposal_id, false, &tx);
        assert!(outcome.is_ok(), "{outcome:?}");
        assert!(
            desk_kinds(&state).is_empty(),
            "a declined review must leave the desk"
        );
        assert!(
            queue(&state, session_id).items.is_empty(),
            "declining queues nothing"
        );

        // The outcome is terminal and says what it is, rather than being
        // inferred from an absent phase.
        let after = state.data.workspaces[0].proposals[0].clone();
        assert_eq!(after.review, Some(crate::models::ReviewPhase::Closed));

        // And every surface says so. The objectives tab used to read "queued"
        // here, because a closed job is still approved and nothing above the
        // state arm claimed it.
        assert_eq!(
            crate::tui::components::tasks_pane::proposal_verb(&after),
            "closed ",
            "a job the user stopped must not advertise itself as running"
        );

        // What happened is kept: the work did happen, and the ledger goes on
        // counting the turns it cost.
        assert_eq!(after.state, crate::models::ProposalState::Approved);
        assert_eq!(after.review_rounds, 3);
        assert!(after.findings.is_some());

        // And a second no is refused rather than doing it again.
        assert!(decide_needs_user(&mut state, workspace_id, proposal_id, false, &tx).is_err());
    }

    /// Closing a review must not un-spend what it already burned: the agent's
    /// turns and the manager's punt are history, not a live commitment.
    #[test]
    fn a_closed_review_keeps_its_burn_in_the_objective_ledger() {
        use crate::models::{objective_ledger, Objective};

        let (mut state, session_id, proposal_id) = state_with_parked_review();
        let workspace_id = state.data.workspaces[0].id;
        let objective = Objective::new("keep it green");
        let objective_id = objective.id;
        state.data.workspaces[0].objectives.push(objective);
        state.data.workspaces[0].proposals[0].objective_id = Some(objective_id);
        let _ = session_id;

        let before = objective_ledger(&state.data.workspaces[0].proposals, objective_id);
        assert_eq!(before.needs_user, 1);
        assert_eq!(before.agent_turns, 4, "one job plus three rework rounds");
        assert_eq!(before.reviews, 4, "three rounds plus the punt");

        let (tx, _rx) = mpsc::unbounded_channel();
        decide_needs_user(&mut state, workspace_id, proposal_id, false, &tx).unwrap();

        let after = objective_ledger(&state.data.workspaces[0].proposals, objective_id);
        assert_eq!(after.needs_user, 0, "nothing is waiting on you any more");
        assert_eq!(after.in_flight, 0, "and nothing is running");
        assert_eq!(after.resolved_this_week, 0, "nobody accepted it");
        assert_eq!(after.agent_turns, before.agent_turns, "the burn is history");
        assert_eq!(after.reviews, before.reviews);
    }

    /// Re-arming twice queued the job twice: nothing checked that the first
    /// call had already moved the proposal out of "needs you". A stale tap or
    /// a retried post is enough to hit it.
    #[test]
    fn re_arming_twice_queues_the_work_once() {
        let (mut state, session_id, proposal_id) = state_with_parked_review();
        let workspace_id = state.data.workspaces[0].id;
        let (tx, _rx) = mpsc::unbounded_channel();

        assert!(decide_needs_user(&mut state, workspace_id, proposal_id, true, &tx).is_ok());
        assert_eq!(queue(&state, session_id).items.len(), 1);
        let first = state.data.workspaces[0].proposals[0].todo_id;

        let again = decide_needs_user(&mut state, workspace_id, proposal_id, true, &tx);
        assert!(again.is_err(), "a second re-arm is refused: {again:?}");
        assert_eq!(
            queue(&state, session_id).items.len(),
            1,
            "the agent must not be handed the same job twice"
        );
        assert_eq!(
            state.data.workspaces[0].proposals[0].todo_id, first,
            "and the proposal still points at the item it really became"
        );
    }

    /// Once re-armed the row is working, not parked, so it leaves the desk on
    /// its own — and the decline that no longer applies is refused.
    #[test]
    fn a_re_armed_review_leaves_the_desk() {
        let (mut state, _session_id, proposal_id) = state_with_parked_review();
        let workspace_id = state.data.workspaces[0].id;
        let (tx, _rx) = mpsc::unbounded_channel();

        assert!(decide_needs_user(&mut state, workspace_id, proposal_id, true, &tx).is_ok());
        assert!(desk_kinds(&state).is_empty());
        assert!(decide_needs_user(&mut state, workspace_id, proposal_id, false, &tx).is_err());
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
    fn banner_toggle_updates_visibility_and_requests_a_resize() {
        let mut state = AppState::default();
        assert!(state.ui.banner_visible);
        assert!(!state.system.pty_resize_pending);

        toggle_banner(&mut state);

        assert!(!state.ui.banner_visible);
        assert!(state.system.pty_resize_pending);
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
