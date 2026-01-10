use super::parallel_task::ParallelTask;
use super::todo::Todo;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Maximum number of pinned terminals per workspace
pub const MAX_PINNED_TERMINALS: usize = 4;

/// Workspace status for organizing active vs paused projects
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum WorkspaceStatus {
    #[default]
    Working,
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: Uuid,
    pub name: String,
    pub path: PathBuf,
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Pinned terminal session IDs - shown stacked in the pinned pane
    #[serde(default)]
    pub pinned_terminal_ids: Vec<Uuid>,
    /// Whether this workspace is actively being worked on
    #[serde(default)]
    pub status: WorkspaceStatus,
    /// Last time this workspace had activity (session created, input sent, etc.)
    #[serde(default)]
    pub last_active_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Todo items for this workspace
    #[serde(default)]
    pub todos: Vec<Todo>,
    /// Parallel tasks for multi-agent task execution
    #[serde(default)]
    pub parallel_tasks: Vec<ParallelTask>,
}

impl Workspace {
    pub fn new(name: String, path: PathBuf) -> Self {
        let now = chrono::Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            path,
            created_at: now,
            pinned_terminal_ids: Vec::new(),
            status: WorkspaceStatus::default(),
            last_active_at: Some(now),
            todos: Vec::new(),
            parallel_tasks: Vec::new(),
        }
    }

    /// Toggle between Working and Paused status
    pub fn toggle_status(&mut self) {
        self.status = match self.status {
            WorkspaceStatus::Working => WorkspaceStatus::Paused,
            WorkspaceStatus::Paused => WorkspaceStatus::Working,
        };
    }

    /// Update last_active_at to now
    pub fn touch(&mut self) {
        self.last_active_at = Some(chrono::Utc::now());
    }

    /// Format last_active_at as a human-readable relative time string
    pub fn last_active_display(&self) -> String {
        match self.last_active_at {
            Some(ts) => {
                let now = chrono::Utc::now();
                let duration = now.signed_duration_since(ts);

                if duration.num_seconds() < 60 {
                    "just now".to_string()
                } else if duration.num_minutes() < 60 {
                    let mins = duration.num_minutes();
                    format!("{}m ago", mins)
                } else if duration.num_hours() < 24 {
                    let hours = duration.num_hours();
                    format!("{}h ago", hours)
                } else if duration.num_days() == 1 {
                    "yesterday".to_string()
                } else if duration.num_days() < 7 {
                    format!("{}d ago", duration.num_days())
                } else if duration.num_weeks() < 4 {
                    let weeks = duration.num_weeks();
                    if weeks == 1 {
                        "1w ago".to_string()
                    } else {
                        format!("{}w ago", weeks)
                    }
                } else {
                    // Show month/day for older items
                    ts.format("%b %d").to_string()
                }
            }
            None => "never".to_string(),
        }
    }

    /// Add a terminal to the pinned list (up to MAX_PINNED_TERMINALS)
    pub fn pin_terminal(&mut self, session_id: Uuid) -> bool {
        if self.pinned_terminal_ids.len() >= MAX_PINNED_TERMINALS {
            return false;
        }
        if !self.pinned_terminal_ids.contains(&session_id) {
            self.pinned_terminal_ids.push(session_id);
        }
        true
    }

    /// Remove a terminal from the pinned list
    pub fn unpin_terminal(&mut self, session_id: Uuid) {
        self.pinned_terminal_ids.retain(|id| *id != session_id);
    }

    pub fn from_path(path: PathBuf) -> Self {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        Self::new(name, path)
    }

    // ============ Todo Management ============

    /// Add a new todo item
    pub fn add_todo(&mut self, description: impl Into<String>) -> Uuid {
        let todo = Todo::new(description);
        let id = todo.id;
        self.todos.push(todo);
        id
    }

    /// Add a suggested todo item (from analyzer)
    pub fn add_suggested_todo(&mut self, description: impl Into<String>) -> Uuid {
        let todo = Todo::suggested(description);
        let id = todo.id;
        self.todos.push(todo);
        id
    }

    /// Remove a todo by ID
    pub fn remove_todo(&mut self, todo_id: Uuid) -> bool {
        let len_before = self.todos.len();
        self.todos.retain(|t| t.id != todo_id);
        self.todos.len() < len_before
    }

    /// Get a mutable todo by ID
    pub fn get_todo_mut(&mut self, todo_id: Uuid) -> Option<&mut Todo> {
        self.todos.iter_mut().find(|t| t.id == todo_id)
    }

    /// Get the next dispatchable todo (Queued first, then Pending)
    pub fn next_pending_todo(&self) -> Option<&Todo> {
        // Queued todos take priority
        self.todos.iter().find(|t| t.is_queued())
            .or_else(|| self.todos.iter().find(|t| t.is_pending()))
    }

    /// Check if there's an in-progress todo in this workspace
    pub fn has_in_progress_todo(&self) -> bool {
        self.todos.iter().any(|t| t.is_in_progress())
    }

    /// Get the IN-PROGRESS todo for a session (not ReadyForReview or Done)
    pub fn todo_for_session(&self, session_id: Uuid) -> Option<&Todo> {
        self.todos.iter().find(|t| {
            t.is_in_progress() && t.assigned_session_id() == Some(session_id)
        })
    }

    /// Get mutable IN-PROGRESS todo for a session
    pub fn todo_for_session_mut(&mut self, session_id: Uuid) -> Option<&mut Todo> {
        self.todos.iter_mut().find(|t| {
            t.is_in_progress() && t.assigned_session_id() == Some(session_id)
        })
    }

    // ============ Parallel Task Management ============

    /// Add a new parallel task
    pub fn add_parallel_task(&mut self, task: ParallelTask) {
        self.parallel_tasks.push(task);
    }

    /// Get a parallel task by ID
    pub fn get_parallel_task(&self, task_id: Uuid) -> Option<&ParallelTask> {
        self.parallel_tasks.iter().find(|t| t.id == task_id)
    }

    /// Get a mutable parallel task by ID
    pub fn get_parallel_task_mut(&mut self, task_id: Uuid) -> Option<&mut ParallelTask> {
        self.parallel_tasks.iter_mut().find(|t| t.id == task_id)
    }

    /// Get the active (running) parallel task, if any
    pub fn active_parallel_task(&self) -> Option<&ParallelTask> {
        use super::parallel_task::ParallelTaskStatus;
        self.parallel_tasks
            .iter()
            .find(|t| matches!(t.status, ParallelTaskStatus::Running | ParallelTaskStatus::AwaitingSelection))
    }

    /// Get a mutable reference to the active parallel task
    pub fn active_parallel_task_mut(&mut self) -> Option<&mut ParallelTask> {
        use super::parallel_task::ParallelTaskStatus;
        self.parallel_tasks
            .iter_mut()
            .find(|t| matches!(t.status, ParallelTaskStatus::Running | ParallelTaskStatus::AwaitingSelection))
    }

    /// Remove a parallel task by ID
    pub fn remove_parallel_task(&mut self, task_id: Uuid) -> bool {
        let len_before = self.parallel_tasks.len();
        self.parallel_tasks.retain(|t| t.id != task_id);
        self.parallel_tasks.len() < len_before
    }

    /// Get the parallel task that contains a specific session
    pub fn parallel_task_for_session(&self, session_id: Uuid) -> Option<&ParallelTask> {
        self.parallel_tasks.iter().find(|t| {
            t.attempts.iter().any(|a| a.session_id == session_id)
        })
    }

    /// Get mutable parallel task that contains a specific session
    pub fn parallel_task_for_session_mut(&mut self, session_id: Uuid) -> Option<&mut ParallelTask> {
        self.parallel_tasks.iter_mut().find(|t| {
            t.attempts.iter().any(|a| a.session_id == session_id)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AgentType, ParallelTask, ParallelTaskAttempt, ParallelTaskStatus};
    use std::path::PathBuf;

    fn create_test_workspace() -> Workspace {
        Workspace::new("test-workspace".to_string(), PathBuf::from("/tmp/workspace"))
    }

    fn create_test_task(workspace_id: Uuid) -> ParallelTask {
        ParallelTask::new(
            workspace_id,
            "Test prompt".to_string(),
            "main".to_string(),
            "abc123".to_string(),
            false, // Don't request report by default in tests
        )
    }

    fn create_test_attempt(task_id: Uuid, agent_type: AgentType) -> ParallelTaskAttempt {
        ParallelTaskAttempt::new(
            task_id,
            Uuid::new_v4(),
            agent_type,
            "test-branch".to_string(),
            PathBuf::from("/tmp/worktree"),
        )
    }

    // ==================== Parallel Task Management Tests ====================

    #[test]
    fn test_add_and_get_parallel_task() {
        let mut ws = create_test_workspace();
        let task = create_test_task(ws.id);
        let task_id = task.id;

        ws.add_parallel_task(task);

        assert_eq!(ws.parallel_tasks.len(), 1);
        let found = ws.get_parallel_task(task_id);
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, task_id);
    }

    #[test]
    fn test_get_parallel_task_not_found() {
        let ws = create_test_workspace();
        assert!(ws.get_parallel_task(Uuid::new_v4()).is_none());
    }

    #[test]
    fn test_remove_parallel_task() {
        let mut ws = create_test_workspace();
        let task = create_test_task(ws.id);
        let task_id = task.id;
        ws.add_parallel_task(task);

        assert!(ws.remove_parallel_task(task_id));
        assert!(ws.parallel_tasks.is_empty());

        // Removing again should return false
        assert!(!ws.remove_parallel_task(task_id));
    }

    #[test]
    fn test_active_parallel_task_running() {
        let mut ws = create_test_workspace();

        // No active task initially
        assert!(ws.active_parallel_task().is_none());

        // Add a running task
        let mut task = create_test_task(ws.id);
        task.status = ParallelTaskStatus::Running;
        ws.add_parallel_task(task);

        assert!(ws.active_parallel_task().is_some());
    }

    #[test]
    fn test_active_parallel_task_awaiting_selection() {
        let mut ws = create_test_workspace();

        let mut task = create_test_task(ws.id);
        task.status = ParallelTaskStatus::AwaitingSelection;
        ws.add_parallel_task(task);

        // AwaitingSelection is also considered active
        assert!(ws.active_parallel_task().is_some());
    }

    #[test]
    fn test_active_parallel_task_completed_not_active() {
        let mut ws = create_test_workspace();

        let mut task = create_test_task(ws.id);
        task.status = ParallelTaskStatus::Completed;
        ws.add_parallel_task(task);

        // Completed tasks are not active
        assert!(ws.active_parallel_task().is_none());
    }

    #[test]
    fn test_active_parallel_task_cancelled_not_active() {
        let mut ws = create_test_workspace();

        let mut task = create_test_task(ws.id);
        task.status = ParallelTaskStatus::Cancelled;
        ws.add_parallel_task(task);

        // Cancelled tasks are not active
        assert!(ws.active_parallel_task().is_none());
    }

    #[test]
    fn test_active_parallel_task_returns_first_active() {
        let mut ws = create_test_workspace();

        // Add a completed task first
        let mut completed_task = create_test_task(ws.id);
        completed_task.status = ParallelTaskStatus::Completed;
        ws.add_parallel_task(completed_task);

        // Add a running task
        let running_task = create_test_task(ws.id);
        let running_task_id = running_task.id;
        ws.add_parallel_task(running_task);

        // Should return the running task, not the completed one
        let active = ws.active_parallel_task().unwrap();
        assert_eq!(active.id, running_task_id);
    }

    #[test]
    fn test_parallel_task_for_session() {
        let mut ws = create_test_workspace();
        let mut task = create_test_task(ws.id);

        let attempt = create_test_attempt(task.id, AgentType::Claude);
        let session_id = attempt.session_id;
        task.add_attempt(attempt);

        ws.add_parallel_task(task);

        // Find task by session ID
        let found = ws.parallel_task_for_session(session_id);
        assert!(found.is_some());
        assert!(found.unwrap().attempts.iter().any(|a| a.session_id == session_id));

        // Non-existent session
        assert!(ws.parallel_task_for_session(Uuid::new_v4()).is_none());
    }

    #[test]
    fn test_parallel_task_for_session_mut() {
        let mut ws = create_test_workspace();
        let mut task = create_test_task(ws.id);

        let attempt = create_test_attempt(task.id, AgentType::Gemini);
        let session_id = attempt.session_id;
        task.add_attempt(attempt);
        ws.add_parallel_task(task);

        // Modify through mutable reference
        if let Some(task) = ws.parallel_task_for_session_mut(session_id) {
            if let Some(attempt) = task.attempts.iter_mut().find(|a| a.session_id == session_id) {
                attempt.prompt_sent = true;
            }
        }

        // Verify modification
        let task = ws.parallel_task_for_session(session_id).unwrap();
        let attempt = task.attempts.iter().find(|a| a.session_id == session_id).unwrap();
        assert!(attempt.prompt_sent);
    }

    #[test]
    fn test_get_parallel_task_mut() {
        let mut ws = create_test_workspace();
        let task = create_test_task(ws.id);
        let task_id = task.id;
        ws.add_parallel_task(task);

        // Modify through mutable reference
        if let Some(task) = ws.get_parallel_task_mut(task_id) {
            task.mark_cancelled();
        }

        // Verify modification
        let task = ws.get_parallel_task(task_id).unwrap();
        assert_eq!(task.status, ParallelTaskStatus::Cancelled);
    }

    // ==================== Reports Tab Workflow Tests ====================
    // These test the workspace methods used by the Reports feature

    #[test]
    fn test_reports_workflow_multiple_attempts() {
        let mut ws = create_test_workspace();
        let mut task = create_test_task(ws.id);
        let task_id = task.id;

        // Add multiple attempts (shown in Reports tab)
        let claude_attempt = create_test_attempt(task.id, AgentType::Claude);
        let gemini_attempt = create_test_attempt(task.id, AgentType::Gemini);
        let codex_attempt = create_test_attempt(task.id, AgentType::Codex);

        let claude_session = claude_attempt.session_id;
        let gemini_session = gemini_attempt.session_id;

        task.add_attempt(claude_attempt);
        task.add_attempt(gemini_attempt);
        task.add_attempt(codex_attempt);
        ws.add_parallel_task(task);

        // Reports tab uses active_parallel_task to get attempts
        let active = ws.active_parallel_task().unwrap();
        assert_eq!(active.attempts.len(), 3);

        // Navigate reports by index
        let idx = 0;
        assert_eq!(active.attempts[idx].agent_type, AgentType::Claude);

        // View report - get session_id for active_session_id
        let session_to_view = active.attempts[idx].session_id;
        assert_eq!(session_to_view, claude_session);

        // Merge report - use attempt ID
        let winner_id = active.attempts[1].id; // Select Gemini
        if let Some(task) = ws.get_parallel_task_mut(task_id) {
            task.mark_completed(winner_id);
        }

        // Task no longer active after merge
        assert!(ws.active_parallel_task().is_none());
    }

    #[test]
    fn test_cancel_active_task_before_new_one() {
        let mut ws = create_test_workspace();

        // Start first task
        let task1 = create_test_task(ws.id);
        let task1_id = task1.id;
        ws.add_parallel_task(task1);

        // Before adding a new task, cancel the old one
        // (This is what parallel.rs does)
        for task in ws.parallel_tasks.iter_mut() {
            if matches!(task.status, ParallelTaskStatus::Running | ParallelTaskStatus::AwaitingSelection) {
                task.status = ParallelTaskStatus::Cancelled;
            }
        }

        // Add second task
        let task2 = create_test_task(ws.id);
        let task2_id = task2.id;
        ws.add_parallel_task(task2);

        // Only the new task should be active
        let active = ws.active_parallel_task().unwrap();
        assert_eq!(active.id, task2_id);
        assert_ne!(active.id, task1_id);

        // First task should be cancelled
        let first = ws.get_parallel_task(task1_id).unwrap();
        assert_eq!(first.status, ParallelTaskStatus::Cancelled);
    }

    #[test]
    fn test_prompt_sent_tracking_via_workspace() {
        let mut ws = create_test_workspace();
        let mut task = create_test_task(ws.id);

        let attempt = create_test_attempt(task.id, AgentType::Claude);
        let session_id = attempt.session_id;
        task.add_attempt(attempt);
        ws.add_parallel_task(task);

        // Initially prompt not sent
        let task = ws.parallel_task_for_session(session_id).unwrap();
        let attempt = task.get_attempt_by_session(session_id).unwrap();
        assert!(!attempt.prompt_sent);

        // Mark as sent (simulates what handler.rs does when session becomes idle)
        if let Some(task) = ws.parallel_task_for_session_mut(session_id) {
            if let Some(attempt) = task.attempts.iter_mut().find(|a| a.session_id == session_id) {
                attempt.prompt_sent = true;
            }
        }

        // Verify
        let task = ws.parallel_task_for_session(session_id).unwrap();
        let attempt = task.get_attempt_by_session(session_id).unwrap();
        assert!(attempt.prompt_sent);
    }
}
