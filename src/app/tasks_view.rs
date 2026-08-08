//! Flattened view of the selected agent's queue.
//!
//! The pane follows the Sessions pane above it: it shows the work queued for
//! whichever agent the session cursor is on, and nothing else — the agent's
//! own name never appears, since the cursor above already says who it is.
//!
//! Two kinds of row, and the difference matters. A **Todo** is ours: you wrote
//! it, it persists, and the dispatcher sends it when the agent frees up. A
//! **Step** is the agent's, mirrored read-only from its own task list and
//! shown under the item it is currently working on — progress detail for the
//! thing in flight. Rows nest but navigate as a flat list, so rendering and
//! key handling must agree on exactly which rows exist and in what order.

use uuid::Uuid;

use crate::agent_tasks::{AgentTask, TaskBatch};
use crate::app::AppState;
use crate::models::{QueuedTodo, SessionStatus, TodoState};

/// A row of the pane. There is deliberately no row for the agent itself:
/// which agent this is is already answered by the Sessions pane cursor above,
/// and repeating it costs a line the tasks can use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskRow {
    /// A queued item of yours. The only row the edit keys act on.
    Todo { session_id: Uuid, todo: Uuid },
    /// One step of the agent's own list, shown under the running item.
    Step {
        session_id: Uuid,
        batch: usize,
        task: usize,
    },
    /// Non-actionable note (empty queue, why the queue is waiting, …).
    Note { session_id: Uuid, text: String },
}

impl TaskRow {
    pub fn session_id(&self) -> Uuid {
        match self {
            TaskRow::Todo { session_id, .. }
            | TaskRow::Step { session_id, .. }
            | TaskRow::Note { session_id, .. } => *session_id,
        }
    }

    /// The queued item this row acts on, if any.
    pub fn todo_id(&self) -> Option<Uuid> {
        match self {
            TaskRow::Todo { todo, .. } => Some(*todo),
            _ => None,
        }
    }
}

/// The agent whose queue the pane is showing.
pub struct AgentEntry {
    pub session_id: Uuid,
    pub running: bool,
}

/// The agent the pane is showing: whatever the Sessions pane cursor is on.
///
/// `None` when that cursor is on a terminal (or there is no session at all) —
/// terminals have no task list, and silently falling back to some other agent
/// would show tasks that belong to a session the user is not looking at.
pub fn selected_agent(state: &AppState) -> Option<AgentEntry> {
    let session = state.selected_session()?;
    if !session.agent_type.is_agent() || session.worktree_viewer_for.is_some() {
        return None;
    }
    Some(AgentEntry {
        session_id: session.id,
        running: session.status == SessionStatus::Running,
    })
}

/// A queued item by id.
pub fn todo_at(state: &AppState, session_id: Uuid, todo: Uuid) -> Option<&QueuedTodo> {
    state
        .get_session(session_id)?
        .todo_queue
        .items
        .iter()
        .find(|item| item.id == todo)
}

pub fn task_at(state: &AppState, session_id: Uuid, batch: usize, task: usize) -> Option<&AgentTask> {
    state
        .system
        .agent_tasks
        .get(&session_id)?
        .batches()
        .get(batch)?
        .tasks
        .get(task)
}

/// Every row of the pane, in display order — the selected agent's alone.
pub fn rows(state: &AppState) -> Vec<TaskRow> {
    let Some(agent) = selected_agent(state) else {
        return Vec::new();
    };
    let session_id = agent.session_id;
    let Some(session) = state.get_session(session_id) else {
        return Vec::new();
    };
    let queue = &session.todo_queue;

    if queue.is_empty() {
        return vec![TaskRow::Note {
            session_id,
            text: if agent.running {
                "No queued work. Press n to add some.".to_string()
            } else {
                "No queued work — this agent is not running.".to_string()
            },
        }];
    }

    let mut rows = Vec::new();
    for item in &queue.items {
        rows.push(TaskRow::Todo {
            session_id,
            todo: item.id,
        });

        // Under the item in flight, the agent's own steps for it: this is
        // the only place the mirrored list earns its space.
        if item.state == TodoState::Running {
            if let Some(batch) = current_batch(state, session_id) {
                for task in 0..batch.1.tasks.len() {
                    rows.push(TaskRow::Step {
                        session_id,
                        batch: batch.0,
                        task,
                    });
                }
            }
        }
    }
    rows
}

/// The agent's newest task list, which belongs to whatever it is doing now.
pub fn current_batch(state: &AppState, session_id: Uuid) -> Option<(usize, &TaskBatch)> {
    let tracker = state.system.agent_tasks.get(&session_id)?;
    let batches = tracker.batches();
    let index = batches.len().checked_sub(1)?;
    Some((index, batches.last()?))
}

/// The row the user is on, clamped to what actually exists.
pub fn selected_row(state: &AppState) -> Option<TaskRow> {
    let rows = rows(state);
    if rows.is_empty() {
        return None;
    }
    rows.get(state.ui.selected_task_row.min(rows.len() - 1))
        .cloned()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::agent_tasks::{Provider, TaskSource, TaskTracker};
    use crate::models::{AgentType, Session, Workspace};
    use chrono::Utc;

    /// The shared fixture: one Claude agent, one prompt, one in-progress task.
    pub(crate) fn fixture() -> (AppState, Uuid, tempfile::TempDir) {
        state_with_log(&claude_log())
    }

    /// A workspace with one Claude session whose log is `log`, already parsed.
    fn state_with_log(log: &str) -> (AppState, Uuid, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut state = AppState::default();
        let workspace = Workspace::new("w".into(), dir.path().to_path_buf());
        let workspace_id = workspace.id;
        state.data.workspaces.push(workspace);
        state.data.sessions.insert(workspace_id, Vec::new());
        let session_id = add_agent(&mut state, workspace_id, dir.path(), "a", log);
        (state, session_id, dir)
    }

    /// Append an agent session to `workspace_id` with its own parsed log.
    fn add_agent(
        state: &mut AppState,
        workspace_id: Uuid,
        dir: &std::path::Path,
        log_name: &str,
        log: &str,
    ) -> Uuid {
        let path = dir.join(format!("{log_name}.jsonl"));
        std::fs::write(&path, log).unwrap();

        let session = Session::new(workspace_id, AgentType::Claude, false);
        let session_id = session.id;
        state
            .data
            .sessions
            .get_mut(&workspace_id)
            .expect("workspace sessions")
            .push(session);

        let mut tracker = TaskTracker::with_source(Provider::Claude, crate::agent_tasks::Source::File(path));
        tracker.refresh(
            &TaskSource {
                provider: Provider::Claude,
                session_uuid: session_id.to_string(),
                cwd: dir.to_path_buf(),
                started_at: Utc::now(),
                conversation: None,
                spawned_at: None,
                reported: None,
            },
            &std::collections::HashSet::new(),
        );
        state.system.agent_tasks.insert(session_id, tracker);
        session_id
    }

    fn claude_log() -> String {
        claude_log_with("make the tasks pane", "Parse agent logs")
    }

    fn claude_log_with(prompt: &str, subject: &str) -> String {
        format!(
            "{}\n",
            [
                claude_prompt_line(prompt),
                claude_create_line("t1", subject),
                claude_created_line("t1", 1, subject),
                claude_update_line("1", "in_progress"),
            ]
            .join("\n")
        )
    }

    fn claude_prompt_line(prompt: &str) -> String {
        serde_json::json!({
            "type": "user",
            "timestamp": "2026-07-25T10:00:00.000Z",
            "message": {"content": [{"type": "text", "text": prompt}]}
        })
        .to_string()
    }

    fn claude_create_line(tool_id: &str, subject: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "message": {"content": [{
                "type": "tool_use", "id": tool_id, "name": "TaskCreate",
                "input": {"subject": subject, "description": "tail jsonl"}
            }]}
        })
        .to_string()
    }

    fn claude_created_line(tool_id: &str, n: usize, subject: &str) -> String {
        serde_json::json!({
            "type": "user",
            "message": {"content": [{
                "type": "tool_result", "tool_use_id": tool_id,
                "content": format!("Task #{n} created successfully: {subject}")
            }]}
        })
        .to_string()
    }

    fn claude_update_line(id: &str, status: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "message": {"content": [{
                "type": "tool_use", "id": "upd", "name": "TaskUpdate",
                "input": {"taskId": id, "status": status}
            }]}
        })
        .to_string()
    }

    /// Queue an item for a session and return its id.
    fn queue(state: &mut AppState, session_id: Uuid, text: &str) -> Uuid {
        state
            .get_session_mut(session_id)
            .unwrap()
            .todo_queue
            .add(text)
    }

    #[test]
    fn the_queue_is_the_pane_and_the_agents_steps_hide_until_one_runs() {
        let (mut state, session_id, _dir) = state_with_log(&claude_log());
        let first = queue(&mut state, session_id, "fix the redirect");
        queue(&mut state, session_id, "write the migration");

        // Nothing dispatched yet: just your two items, no agent steps.
        assert_eq!(
            rows(&state),
            vec![
                TaskRow::Todo {
                    session_id,
                    todo: first
                },
                TaskRow::Todo {
                    session_id,
                    todo: rows(&state)[1].todo_id().unwrap()
                },
            ]
        );

        // Once an item is with the agent, its steps appear beneath it.
        state
            .get_session_mut(session_id)
            .unwrap()
            .todo_queue
            .mark_running(first);
        let rows = rows(&state);
        assert_eq!(rows[0].todo_id(), Some(first));
        assert!(
            matches!(rows[1], TaskRow::Step { .. }),
            "the agent's step belongs under the item it is working on: {rows:?}"
        );
        assert_eq!(
            task_at(&state, session_id, 0, 0).unwrap().subject,
            "Parse agent logs"
        );
        // The second item still follows the running one and its steps.
        assert!(rows.last().unwrap().todo_id().is_some());
    }

    /// The whole point of the pane: one agent at a time, the one selected in
    /// the Sessions pane — never every agent's work piled together.
    #[test]
    fn only_the_session_under_the_cursor_contributes_rows() {
        let (mut state, first, dir) = state_with_log(&claude_log());
        let workspace_id = state.data.workspaces[0].id;
        let second = add_agent(
            &mut state,
            workspace_id,
            dir.path(),
            "b",
            &claude_log_with("second agent prompt", "Second agent task"),
        );
        queue(&mut state, first, "first agent work");
        queue(&mut state, second, "second agent work");

        state.set_selected_session_idx(0);
        let rows_first = rows(&state);
        assert!(rows_first.iter().all(|r| r.session_id() == first));
        assert_eq!(
            todo_at(&state, first, rows_first[0].todo_id().unwrap())
                .unwrap()
                .text,
            "first agent work"
        );

        state.set_selected_session_idx(1);
        let rows_second = rows(&state);
        assert!(rows_second.iter().all(|r| r.session_id() == second));
        assert_eq!(
            todo_at(&state, second, rows_second[0].todo_id().unwrap())
                .unwrap()
                .text,
            "second agent work"
        );
    }

    #[test]
    fn a_terminal_under_the_cursor_shows_nothing_rather_than_another_agents_work() {
        let (mut state, _session_id, _dir) = state_with_log(&claude_log());
        let workspace_id = state.data.workspaces[0].id;
        let terminal = Session::new(
            workspace_id,
            AgentType::Terminal("shell".to_string()),
            false,
        );
        state
            .data
            .sessions
            .get_mut(&workspace_id)
            .unwrap()
            .push(terminal);

        state.set_selected_session_idx(1);
        assert!(rows(&state).is_empty());
    }

    #[test]
    fn moving_to_another_agent_resets_the_row_cursor() {
        let (mut state, first, dir) = state_with_log(&claude_log());
        let workspace_id = state.data.workspaces[0].id;
        let second = add_agent(&mut state, workspace_id, dir.path(), "b", &claude_log());
        queue(&mut state, first, "a");
        queue(&mut state, second, "b");

        crate::app::handlers::tasks::sync_selection(&mut state);
        state.ui.selected_task_row = 1;

        state.set_selected_session_idx(1);
        crate::app::handlers::tasks::sync_selection(&mut state);
        assert_eq!(state.ui.selected_task_row, 0);

        // Staying on the same agent leaves the cursor alone.
        state.ui.selected_task_row = 0;
        crate::app::handlers::tasks::sync_selection(&mut state);
        assert_eq!(state.ui.selected_task_row, 0);
    }

    #[test]
    fn an_agent_with_an_empty_queue_says_so() {
        let (state, _session_id, _dir) = state_with_log(&claude_log());
        let rows = rows(&state);
        assert_eq!(rows.len(), 1);
        assert!(matches!(rows[0], TaskRow::Note { .. }), "{rows:?}");
    }

    #[test]
    fn selection_is_clamped_to_the_rows_that_exist() {
        let (mut state, session_id, _dir) = state_with_log(&claude_log());
        let only = queue(&mut state, session_id, "the only item");
        state.ui.selected_task_row = 99;
        assert_eq!(
            selected_row(&state),
            Some(TaskRow::Todo {
                session_id,
                todo: only
            })
        );
    }
}
