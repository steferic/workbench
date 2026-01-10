//! Parallel task data structures for multi-agent task execution.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

use super::AgentType;

/// A parallel task that runs across multiple agents simultaneously.
///
/// Each agent works in their own git worktree on a separate branch,
/// allowing concurrent work without conflicts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParallelTask {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub prompt: String,
    pub source_branch: String,
    pub source_commit: String,
    pub status: ParallelTaskStatus,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub winner_attempt_id: Option<Uuid>,
    pub attempts: Vec<ParallelTaskAttempt>,
    /// Whether to request a PARALLEL_REPORT.md from agents
    #[serde(default)]
    pub request_report: bool,
}

impl ParallelTask {
    /// Create a new parallel task
    pub fn new(
        workspace_id: Uuid,
        prompt: String,
        source_branch: String,
        source_commit: String,
        request_report: bool,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            prompt,
            source_branch,
            source_commit,
            status: ParallelTaskStatus::Running,
            created_at: Utc::now(),
            completed_at: None,
            winner_attempt_id: None,
            attempts: Vec::new(),
            request_report,
        }
    }

    /// Add an attempt to this task
    pub fn add_attempt(&mut self, attempt: ParallelTaskAttempt) {
        self.attempts.push(attempt);
    }

    /// Get an attempt by ID
    pub fn get_attempt(&self, attempt_id: Uuid) -> Option<&ParallelTaskAttempt> {
        self.attempts.iter().find(|a| a.id == attempt_id)
    }

    /// Check if all attempts are completed (either done or failed)
    pub fn all_attempts_finished(&self) -> bool {
        !self.attempts.is_empty()
            && self.attempts.iter().all(|a| {
                matches!(a.status, AttemptStatus::Completed | AttemptStatus::Failed)
            })
    }

    /// Mark the task as awaiting selection (all agents done)
    pub fn mark_awaiting_selection(&mut self) {
        self.status = ParallelTaskStatus::AwaitingSelection;
    }

    /// Mark the task as completed with a winner
    pub fn mark_completed(&mut self, winner_attempt_id: Uuid) {
        self.status = ParallelTaskStatus::Completed;
        self.completed_at = Some(Utc::now());
        self.winner_attempt_id = Some(winner_attempt_id);
    }

    /// Mark the task as cancelled
    pub fn mark_cancelled(&mut self) {
        self.status = ParallelTaskStatus::Cancelled;
        self.completed_at = Some(Utc::now());
    }

    /// Get the full prompt to send to agents, including report instructions if requested
    pub fn full_prompt(&self) -> String {
        if self.request_report {
            format!(
                "{}\n\n---\nWhen you are done, please write a brief summary of your changes to a file called PARALLEL_REPORT.md in the root of this repository. Include:\n- What approach you took\n- Key changes made\n- Any trade-offs or considerations",
                self.prompt
            )
        } else {
            self.prompt.clone()
        }
    }
}

/// Status of a parallel task
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParallelTaskStatus {
    /// Agents are actively working on the task
    Running,
    /// All agents have finished, waiting for user to select winner
    AwaitingSelection,
    /// Winner has been merged, task is complete
    Completed,
    /// Task was cancelled by user
    Cancelled,
}

/// An individual agent's attempt at a parallel task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParallelTaskAttempt {
    pub id: Uuid,
    pub task_id: Uuid,
    pub session_id: Uuid,
    pub agent_type: AgentType,
    pub branch_name: String,
    pub worktree_path: PathBuf,
    pub status: AttemptStatus,
    pub report_content: Option<String>,
    #[serde(default)]
    pub prompt_sent: bool,
}

impl ParallelTaskAttempt {
    /// Create a new attempt
    pub fn new(
        task_id: Uuid,
        session_id: Uuid,
        agent_type: AgentType,
        branch_name: String,
        worktree_path: PathBuf,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            task_id,
            session_id,
            agent_type,
            branch_name,
            worktree_path,
            status: AttemptStatus::Running,
            report_content: None,
            prompt_sent: false,
        }
    }

    /// Set the report content
    pub fn set_report(&mut self, content: String) {
        self.report_content = Some(content);
    }

    /// Get a preview of the report (first 100 chars)
    pub fn report_preview(&self) -> Option<String> {
        self.report_content.as_ref().map(|content| {
            let preview: String = content.chars().take(100).collect();
            if content.len() > 100 {
                format!("{}...", preview)
            } else {
                preview
            }
        })
    }
}

/// Status of an individual attempt
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttemptStatus {
    /// Agent is actively working
    Running,
    /// Agent has finished successfully
    Completed,
    /// Agent failed or errored out
    Failed,
}

impl AttemptStatus {
    /// Get a display string
    pub fn display(&self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn create_test_task() -> ParallelTask {
        ParallelTask::new(
            Uuid::new_v4(),
            "Fix the login bug".to_string(),
            "main".to_string(),
            "abc123".to_string(),
            false, // Don't request report by default in tests
        )
    }

    fn create_test_attempt(task_id: Uuid, agent_type: AgentType) -> ParallelTaskAttempt {
        let branch_name = format!("parallel-test/{}", agent_type.badge().to_lowercase());
        ParallelTaskAttempt::new(
            task_id,
            Uuid::new_v4(),
            agent_type,
            branch_name,
            PathBuf::from("/tmp/worktree"),
        )
    }

    // ==================== ParallelTask Tests ====================

    #[test]
    fn test_parallel_task_creation() {
        let workspace_id = Uuid::new_v4();
        let task = ParallelTask::new(
            workspace_id,
            "Review this code".to_string(),
            "develop".to_string(),
            "def456".to_string(),
            true,
        );

        assert_eq!(task.workspace_id, workspace_id);
        assert_eq!(task.prompt, "Review this code");
        assert_eq!(task.source_branch, "develop");
        assert_eq!(task.source_commit, "def456");
        assert_eq!(task.status, ParallelTaskStatus::Running);
        assert!(task.attempts.is_empty());
        assert!(task.completed_at.is_none());
        assert!(task.winner_attempt_id.is_none());
        assert!(task.request_report);
    }

    #[test]
    fn test_add_and_get_attempt() {
        let mut task = create_test_task();
        let attempt = create_test_attempt(task.id, AgentType::Claude);
        let attempt_id = attempt.id;

        task.add_attempt(attempt);

        assert_eq!(task.attempts.len(), 1);

        // Get by attempt ID
        let found = task.get_attempt(attempt_id);
        assert!(found.is_some());
        assert_eq!(found.unwrap().agent_type, AgentType::Claude);

        // Get non-existent
        assert!(task.get_attempt(Uuid::new_v4()).is_none());
    }

    #[test]
    fn test_all_attempts_finished() {
        let mut task = create_test_task();

        // Empty task - not finished
        assert!(!task.all_attempts_finished());

        // Add running attempt
        let attempt1 = create_test_attempt(task.id, AgentType::Claude);
        task.add_attempt(attempt1);
        assert!(!task.all_attempts_finished());

        // Mark as completed
        task.attempts[0].status = AttemptStatus::Completed;
        assert!(task.all_attempts_finished());

        // Add another running attempt
        let attempt2 = create_test_attempt(task.id, AgentType::Gemini);
        task.add_attempt(attempt2);
        assert!(!task.all_attempts_finished());

        // Mark second as failed
        task.attempts[1].status = AttemptStatus::Failed;
        assert!(task.all_attempts_finished());
    }

    #[test]
    fn test_task_status_transitions() {
        let mut task = create_test_task();
        assert_eq!(task.status, ParallelTaskStatus::Running);

        // Transition to awaiting selection
        task.mark_awaiting_selection();
        assert_eq!(task.status, ParallelTaskStatus::AwaitingSelection);

        // Transition to completed
        let winner_id = Uuid::new_v4();
        task.mark_completed(winner_id);
        assert_eq!(task.status, ParallelTaskStatus::Completed);
        assert_eq!(task.winner_attempt_id, Some(winner_id));
        assert!(task.completed_at.is_some());
    }

    #[test]
    fn test_task_cancellation() {
        let mut task = create_test_task();
        task.mark_cancelled();

        assert_eq!(task.status, ParallelTaskStatus::Cancelled);
        assert!(task.completed_at.is_some());
        assert!(task.winner_attempt_id.is_none());
    }

    // ==================== ParallelTaskAttempt Tests ====================

    #[test]
    fn test_attempt_creation() {
        let task_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();

        let attempt = ParallelTaskAttempt::new(
            task_id,
            session_id,
            AgentType::Claude,
            "parallel-abc/claude".to_string(),
            PathBuf::from("/tmp/worktree/claude"),
        );

        assert_eq!(attempt.task_id, task_id);
        assert_eq!(attempt.session_id, session_id);
        assert_eq!(attempt.agent_type, AgentType::Claude);
        assert_eq!(attempt.branch_name, "parallel-abc/claude");
        assert_eq!(attempt.worktree_path, PathBuf::from("/tmp/worktree/claude"));
        assert_eq!(attempt.status, AttemptStatus::Running);
        assert!(attempt.report_content.is_none());
        assert!(!attempt.prompt_sent); // Critical: prompt_sent starts as false
    }

    #[test]
    fn test_attempt_prompt_sent_tracking() {
        let mut attempt = create_test_attempt(Uuid::new_v4(), AgentType::Gemini);

        // Initially not sent
        assert!(!attempt.prompt_sent);

        // Mark as sent
        attempt.prompt_sent = true;
        assert!(attempt.prompt_sent);

        // Verify it persists
        let serialized = serde_json::to_string(&attempt).unwrap();
        let deserialized: ParallelTaskAttempt = serde_json::from_str(&serialized).unwrap();
        assert!(deserialized.prompt_sent);
    }

    #[test]
    fn test_attempt_prompt_sent_default_on_deserialize() {
        // Simulate old data without prompt_sent field
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "task_id": "550e8400-e29b-41d4-a716-446655440001",
            "session_id": "550e8400-e29b-41d4-a716-446655440002",
            "agent_type": "Claude",
            "branch_name": "test-branch",
            "worktree_path": "/tmp/test",
            "status": "Running",
            "report_content": null
        }"#;

        let attempt: ParallelTaskAttempt = serde_json::from_str(json).unwrap();
        // Should default to false when missing
        assert!(!attempt.prompt_sent);
    }

    #[test]
    fn test_attempt_report_content() {
        let mut attempt = create_test_attempt(Uuid::new_v4(), AgentType::Gemini);
        assert!(attempt.report_content.is_none());
        assert!(attempt.report_preview().is_none());

        attempt.set_report("This is the agent's report about the changes made.".to_string());
        assert!(attempt.report_content.is_some());
        assert_eq!(attempt.report_preview().unwrap(), "This is the agent's report about the changes made.");
    }

    #[test]
    fn test_attempt_report_preview_truncation() {
        let mut attempt = create_test_attempt(Uuid::new_v4(), AgentType::Claude);
        let long_report = "A".repeat(150);
        attempt.set_report(long_report);

        let preview = attempt.report_preview().unwrap();
        assert!(preview.ends_with("..."));
        assert!(preview.len() <= 103); // 100 chars + "..."
    }

    // ==================== Status Display Tests ====================

    #[test]
    fn test_attempt_status_display() {
        assert_eq!(AttemptStatus::Running.display(), "Running");
        assert_eq!(AttemptStatus::Completed.display(), "Completed");
        assert_eq!(AttemptStatus::Failed.display(), "Failed");
    }

    // ==================== Reports Feature Tests ====================
    // These test the data model aspects of the reports feature

    #[test]
    fn test_reports_navigation_data() {
        let mut task = create_test_task();

        // Add multiple attempts (these appear in the Reports tab)
        task.add_attempt(create_test_attempt(task.id, AgentType::Claude));
        task.add_attempt(create_test_attempt(task.id, AgentType::Gemini));
        task.add_attempt(create_test_attempt(task.id, AgentType::Codex));

        // Verify we can navigate through attempts by index
        assert_eq!(task.attempts.len(), 3);

        let idx = 0;
        assert_eq!(task.attempts.get(idx).unwrap().agent_type, AgentType::Claude);

        let idx = 1;
        assert_eq!(task.attempts.get(idx).unwrap().agent_type, AgentType::Gemini);

        let idx = 2;
        assert_eq!(task.attempts.get(idx).unwrap().agent_type, AgentType::Codex);

        // Out of bounds returns None
        assert!(task.attempts.get(3).is_none());
    }

    #[test]
    fn test_reports_view_data() {
        let mut task = create_test_task();
        let attempt = create_test_attempt(task.id, AgentType::Claude);
        let session_id = attempt.session_id;
        task.add_attempt(attempt);

        // ViewReport uses the session_id to set active_session_id
        let selected_idx = 0;
        let attempt = task.attempts.get(selected_idx).unwrap();
        assert_eq!(attempt.session_id, session_id);
    }

    #[test]
    fn test_reports_merge_selects_winner() {
        let mut task = create_test_task();

        task.add_attempt(create_test_attempt(task.id, AgentType::Claude));
        task.add_attempt(create_test_attempt(task.id, AgentType::Gemini));

        let selected_idx = 1; // Select Gemini's attempt
        let winner_id = task.attempts.get(selected_idx).unwrap().id;

        // Simulate merge action
        task.mark_completed(winner_id);

        assert_eq!(task.status, ParallelTaskStatus::Completed);
        assert_eq!(task.winner_attempt_id, Some(winner_id));
    }

    #[test]
    fn test_reports_with_completed_attempts() {
        let mut task = create_test_task();

        task.add_attempt(create_test_attempt(task.id, AgentType::Claude));
        task.add_attempt(create_test_attempt(task.id, AgentType::Gemini));
        task.add_attempt(create_test_attempt(task.id, AgentType::Codex));

        // Mark attempts as completed with reports
        task.attempts[0].status = AttemptStatus::Completed;
        task.attempts[0].set_report("Claude's solution: refactored the login module.".to_string());

        task.attempts[1].status = AttemptStatus::Completed;
        task.attempts[1].set_report("Gemini's approach: added new authentication layer.".to_string());

        task.attempts[2].status = AttemptStatus::Failed;

        // Verify reports are accessible
        assert!(task.attempts[0].report_content.is_some());
        assert!(task.attempts[1].report_content.is_some());
        assert!(task.attempts[2].report_content.is_none());

        // Task should transition to awaiting selection when all done
        assert!(task.all_attempts_finished());
        task.mark_awaiting_selection();
        assert_eq!(task.status, ParallelTaskStatus::AwaitingSelection);
    }

    // ==================== Edge Cases ====================

    #[test]
    fn test_serialization_roundtrip() {
        let mut task = create_test_task();
        task.add_attempt(create_test_attempt(task.id, AgentType::Claude));
        task.attempts[0].prompt_sent = true;
        task.attempts[0].set_report("Test report".to_string());

        let json = serde_json::to_string(&task).unwrap();
        let restored: ParallelTask = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.prompt, task.prompt);
        assert_eq!(restored.attempts.len(), 1);
        assert!(restored.attempts[0].prompt_sent);
        assert_eq!(restored.attempts[0].report_content, Some("Test report".to_string()));
    }

    // ==================== Request Report Tests ====================

    #[test]
    fn test_request_report_flag_default() {
        // Default test helper doesn't request report
        let task = create_test_task();
        assert!(!task.request_report);
    }

    #[test]
    fn test_request_report_flag_true() {
        let task = ParallelTask::new(
            Uuid::new_v4(),
            "Fix the bug".to_string(),
            "main".to_string(),
            "abc123".to_string(),
            true, // Request report
        );
        assert!(task.request_report);
    }

    #[test]
    fn test_full_prompt_without_report() {
        let task = ParallelTask::new(
            Uuid::new_v4(),
            "Fix the login bug".to_string(),
            "main".to_string(),
            "abc123".to_string(),
            false,
        );

        let full = task.full_prompt();
        assert_eq!(full, "Fix the login bug");
        // Should not contain report instructions
        assert!(!full.contains("PARALLEL_REPORT.md"));
    }

    #[test]
    fn test_full_prompt_with_report() {
        let task = ParallelTask::new(
            Uuid::new_v4(),
            "Fix the login bug".to_string(),
            "main".to_string(),
            "abc123".to_string(),
            true,
        );

        let full = task.full_prompt();
        // Should contain original prompt
        assert!(full.contains("Fix the login bug"));
        // Should contain report instructions
        assert!(full.contains("PARALLEL_REPORT.md"));
        assert!(full.contains("What approach you took"));
        assert!(full.contains("Key changes made"));
    }

    #[test]
    fn test_full_prompt_multiline_with_report() {
        let task = ParallelTask::new(
            Uuid::new_v4(),
            "Fix the bug\n\nPlease also add tests".to_string(),
            "main".to_string(),
            "abc123".to_string(),
            true,
        );

        let full = task.full_prompt();
        // Should preserve original multiline prompt
        assert!(full.contains("Fix the bug"));
        assert!(full.contains("Please also add tests"));
        // Should add separator
        assert!(full.contains("---"));
        // Should add report instructions
        assert!(full.contains("PARALLEL_REPORT.md"));
    }

    #[test]
    fn test_request_report_serialization() {
        let task = ParallelTask::new(
            Uuid::new_v4(),
            "Test task".to_string(),
            "main".to_string(),
            "abc123".to_string(),
            true,
        );

        let json = serde_json::to_string(&task).unwrap();
        let restored: ParallelTask = serde_json::from_str(&json).unwrap();

        assert!(restored.request_report);
        // Verify full_prompt still works after deserialization
        assert!(restored.full_prompt().contains("PARALLEL_REPORT.md"));
    }

    #[test]
    fn test_request_report_backward_compatibility() {
        // Simulate old data without request_report field
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "workspace_id": "550e8400-e29b-41d4-a716-446655440001",
            "prompt": "Test prompt",
            "source_branch": "main",
            "source_commit": "abc123",
            "status": "Running",
            "created_at": "2024-01-01T00:00:00Z",
            "completed_at": null,
            "winner_attempt_id": null,
            "attempts": []
        }"#;

        let task: ParallelTask = serde_json::from_str(json).unwrap();
        // Should default to false when missing
        assert!(!task.request_report);
        // full_prompt should just return the prompt
        assert_eq!(task.full_prompt(), "Test prompt");
    }
}
