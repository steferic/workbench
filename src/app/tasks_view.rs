//! Flattened view of the selected agent's task list.
//!
//! The pane follows the Sessions pane above it: it shows the task lists of
//! whichever agent the session cursor is on, and nothing else. Rows nest
//! (agent → prompt → tasks) but navigate as a flat list, so rendering and key
//! handling must agree on exactly which rows exist and in what order. Both
//! build it here.

use uuid::Uuid;

use crate::agent_tasks::{AgentTask, Provider, TaskBatch};
use crate::app::AppState;
use crate::models::{AgentType, SessionStatus};

/// How many of the agent's task lists to show — enough to see what a
/// follow-up prompt changed, without burying the current one.
pub const MAX_BATCHES_PER_AGENT: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskRow {
    /// The agent these rows belong to — its name, status and branch.
    Agent {
        session_id: Uuid,
    },
    /// The prompt that produced the task list below it.
    Prompt {
        session_id: Uuid,
        batch: usize,
    },
    Task {
        session_id: Uuid,
        batch: usize,
        task: usize,
    },
    /// Non-actionable note (no tasks yet, older lists elided, …).
    Note {
        session_id: Uuid,
        text: String,
    },
}

impl TaskRow {
    pub fn session_id(&self) -> Uuid {
        match self {
            TaskRow::Agent { session_id }
            | TaskRow::Prompt { session_id, .. }
            | TaskRow::Task { session_id, .. }
            | TaskRow::Note { session_id, .. } => *session_id,
        }
    }
}

/// An agent session that can appear in the pane.
pub struct AgentEntry {
    pub session_id: Uuid,
    pub name: String,
    pub agent_type: AgentType,
    pub alias: Option<String>,
    pub branch: Option<String>,
    pub running: bool,
    pub idle: bool,
    /// False when we cannot read this provider's task list (Gemini, Grok, …).
    pub readable: bool,
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
        name: session.display_name(),
        agent_type: session.agent_type.clone(),
        alias: session.alias.clone(),
        branch: session.worktree_branch.clone(),
        running: session.status == SessionStatus::Running,
        idle: state.data.idle_queue.contains(&session.id),
        readable: Provider::for_agent(&session.agent_type).is_some(),
    })
}

/// The task lists we show for one agent: newest first, capped.
pub fn batches_for(state: &AppState, session_id: Uuid) -> Vec<(usize, &TaskBatch)> {
    let Some(tracker) = state.system.agent_tasks.get(&session_id) else {
        return Vec::new();
    };
    let batches = tracker.batches();
    batches
        .iter()
        .enumerate()
        .rev()
        .take(MAX_BATCHES_PER_AGENT)
        .collect()
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
    let mut rows = vec![TaskRow::Agent { session_id }];

    if !agent.readable {
        rows.push(TaskRow::Note {
            session_id,
            text: format!(
                "{} does not publish a task list",
                agent.agent_type.display_name()
            ),
        });
        return rows;
    }

    let batches = batches_for(state, session_id);
    if batches.is_empty() {
        let found_log = state
            .system
            .agent_tasks
            .get(&session_id)
            .map(|tracker| tracker.has_source())
            .unwrap_or(false);
        rows.push(TaskRow::Note {
            session_id,
            text: match (agent.running, found_log) {
                (false, _) => "not started".to_string(),
                (true, false) => "waiting for the agent to start".to_string(),
                (true, true) => "no task list yet".to_string(),
            },
        });
        return rows;
    }

    let total = state
        .system
        .agent_tasks
        .get(&session_id)
        .map(|t| t.batches().len())
        .unwrap_or(0);

    for (batch_idx, batch) in batches {
        rows.push(TaskRow::Prompt {
            session_id,
            batch: batch_idx,
        });
        for task_idx in 0..batch.tasks.len() {
            rows.push(TaskRow::Task {
                session_id,
                batch: batch_idx,
                task: task_idx,
            });
        }
    }

    if total > MAX_BATCHES_PER_AGENT {
        rows.push(TaskRow::Note {
            session_id,
            text: format!("{} earlier task lists", total - MAX_BATCHES_PER_AGENT),
        });
    }
    rows
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
    use crate::models::{Session, Workspace};
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
        [
            serde_json::json!({
                "type": "user",
                "timestamp": "2026-07-25T10:00:00.000Z",
                "message": {"content": [{"type": "text", "text": prompt}]}
            }),
            serde_json::json!({
                "type": "assistant",
                "message": {"content": [{
                    "type": "tool_use", "id": "t1", "name": "TaskCreate",
                    "input": {"subject": subject, "description": "tail jsonl"}
                }]}
            }),
            serde_json::json!({
                "type": "user",
                "message": {"content": [{
                    "type": "tool_result", "tool_use_id": "t1",
                    "content": format!("Task #1 created successfully: {subject}")
                }]}
            }),
            serde_json::json!({
                "type": "assistant",
                "message": {"content": [{
                    "type": "tool_use", "id": "t2", "name": "TaskUpdate",
                    "input": {"taskId": "1", "status": "in_progress"}
                }]}
            }),
        ]
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join("\n")
            + "\n"
    }

    #[test]
    fn rows_nest_tasks_under_the_prompt_under_the_agent() {
        let (state, session_id, _dir) = state_with_log(&claude_log());
        let rows = rows(&state);

        assert_eq!(
            rows,
            vec![
                TaskRow::Agent { session_id },
                TaskRow::Prompt {
                    session_id,
                    batch: 0
                },
                TaskRow::Task {
                    session_id,
                    batch: 0,
                    task: 0
                },
            ]
        );
        assert_eq!(
            task_at(&state, session_id, 0, 0).unwrap().subject,
            "Parse agent logs"
        );
    }

    /// The whole point of the pane: one agent at a time, the one selected in
    /// the Sessions pane — never every agent's tasks piled together.
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

        state.set_selected_session_idx(0);
        let rows_first = rows(&state);
        assert!(rows_first.iter().all(|r| r.session_id() == first));
        assert_eq!(
            task_at(&state, first, 0, 0).unwrap().subject,
            "Parse agent logs"
        );

        state.set_selected_session_idx(1);
        let rows_second = rows(&state);
        assert!(rows_second.iter().all(|r| r.session_id() == second));
        assert_eq!(
            task_at(&state, second, 0, 0).unwrap().subject,
            "Second agent task"
        );
    }

    #[test]
    fn a_terminal_under_the_cursor_shows_nothing_rather_than_another_agents_tasks() {
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
        let (mut state, _first, dir) = state_with_log(&claude_log());
        let workspace_id = state.data.workspaces[0].id;
        add_agent(&mut state, workspace_id, dir.path(), "b", &claude_log());

        crate::app::handlers::tasks::sync_selection(&mut state);
        state.ui.selected_task_row = 2;

        state.set_selected_session_idx(1);
        crate::app::handlers::tasks::sync_selection(&mut state);
        assert_eq!(state.ui.selected_task_row, 0);

        // Staying on the same agent leaves the cursor alone.
        state.ui.selected_task_row = 1;
        crate::app::handlers::tasks::sync_selection(&mut state);
        assert_eq!(state.ui.selected_task_row, 1);
    }

    #[test]
    fn an_agent_with_no_task_list_still_shows_up() {
        let (state, session_id, _dir) = state_with_log("");
        let rows = rows(&state);
        assert_eq!(rows[0], TaskRow::Agent { session_id });
        assert!(matches!(rows[1], TaskRow::Note { .. }), "{rows:?}");
    }

    #[test]
    fn selection_is_clamped_to_the_rows_that_exist() {
        let (mut state, session_id, _dir) = state_with_log(&claude_log());
        state.ui.selected_task_row = 99;
        assert_eq!(
            selected_row(&state),
            Some(TaskRow::Task {
                session_id,
                batch: 0,
                task: 0
            })
        );
    }
}
