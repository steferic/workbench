use super::parallel_task::ParallelTask;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Maximum number of pinned terminals per workspace
pub const MAX_PINNED_TERMINALS: usize = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: Uuid,
    pub name: String,
    pub path: PathBuf,
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Pinned terminal session IDs - shown stacked in the pinned pane
    #[serde(default)]
    pub pinned_terminal_ids: Vec<Uuid>,
    /// Last time this workspace had activity (session created, input sent, etc.)
    #[serde(default)]
    pub last_active_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Parallel tasks for multi-agent task execution
    #[serde(default)]
    pub parallel_tasks: Vec<ParallelTask>,
    /// Standing priorities for this project, in priority order. Written by
    /// the user; read by a manager (see `models::objective`).
    #[serde(default)]
    pub objectives: Vec<super::Objective>,
    /// What the managers here have suggested, newest last. Suggestions only:
    /// nothing in this list has been queued for anyone.
    #[serde(default)]
    pub proposals: Vec<super::Proposal>,
    /// Currently active worktree session ID (None = viewing main branch)
    #[serde(default)]
    pub active_worktree_session_id: Option<Uuid>,
    /// Last active session ID for this workspace (restored when switching back)
    #[serde(default)]
    pub last_active_session_id: Option<Uuid>,
    /// The one workspace that is not a project: a standing place for agents
    /// that see every project at once (see `Workspace::global`).
    #[serde(default)]
    pub global: bool,
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
            last_active_at: Some(now),
            parallel_tasks: Vec::new(),
            objectives: Vec::new(),
            proposals: Vec::new(),
            active_worktree_session_id: None,
            last_active_session_id: None,
            global: false,
        }
    }

    /// Where the global workspace lives: a directory of workbench's own,
    /// not a repository. An agent started there has no project to be inside
    /// of, which is the point — reads reach every repo, and a write would
    /// land outside any project's branch and worktree bookkeeping, so the
    /// brief tells it to hand writes to a project's agent instead.
    pub fn global_root() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("workbench").join("global"))
    }

    /// The global workspace, rooted at `global_root`.
    pub fn global(path: PathBuf) -> Self {
        let mut workspace = Self::new("Global".into(), path);
        workspace.global = true;
        workspace
    }

    /// Make sure the list holds exactly one global workspace, first. Added on
    /// the first run after this existed (older saved state has none), and
    /// moved to the front if something reordered it — pinned first is how
    /// it stays findable, on the desktop and on the phone alike.
    pub fn ensure_global(workspaces: &mut Vec<Workspace>) {
        let Some(root) = Self::global_root() else {
            return;
        };
        let _ = std::fs::create_dir_all(&root);
        Self::ensure_global_at(workspaces, root);
    }

    /// `ensure_global` with the root given, for callers (and tests) that
    /// should not touch the real config directory.
    pub fn ensure_global_at(workspaces: &mut Vec<Workspace>, root: PathBuf) {
        match workspaces.iter().position(|w| w.global) {
            Some(0) => {}
            Some(at) => {
                let global = workspaces.remove(at);
                workspaces.insert(0, global);
            }
            None => workspaces.insert(0, Self::global(root)),
        }
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

    /// Remove a terminal from the pinned list. Use
    /// `AppState::unpin_terminal_anywhere` instead so per-workspace UI state
    /// stays index-aligned.
    #[allow(dead_code)]
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
        self.parallel_tasks.iter().find(|t| {
            matches!(
                t.status,
                ParallelTaskStatus::Running | ParallelTaskStatus::AwaitingSelection
            )
        })
    }

    /// Remove a parallel task by ID
    pub fn remove_parallel_task(&mut self, task_id: Uuid) -> bool {
        let len_before = self.parallel_tasks.len();
        self.parallel_tasks.retain(|t| t.id != task_id);
        self.parallel_tasks.len() < len_before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AgentType, ParallelTask, ParallelTaskAttempt, ParallelTaskStatus};

    fn create_test_workspace() -> Workspace {
        Workspace::new(
            "test-workspace".to_string(),
            std::env::temp_dir().join("workspace"),
        )
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
            std::env::temp_dir().join("worktree"),
        )
    }

    #[test]
    fn legacy_paused_status_is_ignored_when_loading() {
        let workspace = create_test_workspace();
        let mut saved = serde_json::to_value(&workspace).unwrap();
        saved
            .as_object_mut()
            .unwrap()
            .insert("status".into(), serde_json::Value::String("Paused".into()));

        let loaded: Workspace = serde_json::from_value(saved).unwrap();

        assert_eq!(loaded.id, workspace.id);
        assert_eq!(loaded.name, workspace.name);
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
        let _gemini_session = gemini_attempt.session_id;

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
            if matches!(
                task.status,
                ParallelTaskStatus::Running | ParallelTaskStatus::AwaitingSelection
            ) {
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
        let attempt = ws
            .parallel_tasks
            .iter()
            .flat_map(|task| task.attempts.iter())
            .find(|attempt| attempt.session_id == session_id)
            .unwrap();
        assert!(!attempt.prompt_sent);

        // Mark as sent (simulates what handler.rs does when session becomes idle)
        if let Some(attempt) = ws
            .parallel_tasks
            .iter_mut()
            .flat_map(|task| task.attempts.iter_mut())
            .find(|attempt| attempt.session_id == session_id)
        {
            attempt.prompt_sent = true;
        }

        // Verify
        let attempt = ws
            .parallel_tasks
            .iter()
            .flat_map(|task| task.attempts.iter())
            .find(|attempt| attempt.session_id == session_id)
            .unwrap();
        assert!(attempt.prompt_sent);
    }
}

#[cfg(test)]
mod global_tests {
    use super::Workspace;
    use std::path::PathBuf;

    fn project(name: &str) -> Workspace {
        Workspace::new(name.into(), PathBuf::from(format!("/tmp/{name}")))
    }

    /// Saved state from before the global workspace existed gets one, first.
    #[test]
    fn a_missing_global_workspace_is_added_first() {
        let mut workspaces = vec![project("alpha"), project("beta")];
        Workspace::ensure_global_at(&mut workspaces, PathBuf::from("/tmp/global"));
        assert_eq!(workspaces.len(), 3);
        assert!(workspaces[0].global);
        assert_eq!(workspaces[0].name, "Global");
        assert_eq!(workspaces[1].name, "alpha");
    }

    /// One that drifted down the list is moved back to the front, and a
    /// second is never added.
    #[test]
    fn an_existing_global_workspace_is_kept_and_pinned_first() {
        let global = Workspace::global(PathBuf::from("/tmp/global"));
        let id = global.id;
        let mut workspaces = vec![project("alpha"), global, project("beta")];
        Workspace::ensure_global_at(&mut workspaces, PathBuf::from("/tmp/elsewhere"));
        assert_eq!(workspaces.len(), 3);
        assert_eq!(workspaces[0].id, id, "the same one, not a replacement");
        assert_eq!(workspaces[0].path, PathBuf::from("/tmp/global"));
        Workspace::ensure_global_at(&mut workspaces, PathBuf::from("/tmp/global"));
        assert_eq!(workspaces.len(), 3, "idempotent");
    }
}
