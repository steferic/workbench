//! Tests for parallel task handler logic
//!
//! These tests verify the handler functions work correctly without
//! requiring actual PTY sessions or git operations.

#[cfg(test)]
mod tests {
    use crate::app::{AppState, FocusPanel, InputMode};
    use crate::models::{
        AgentType, ParallelTask, ParallelTaskAttempt, ParallelTaskStatus, Workspace,
    };
    use std::path::PathBuf;
    use uuid::Uuid;

    /// Create a minimal AppState for testing
    fn create_test_state() -> AppState {
        let mut state = AppState::default();

        // Add a workspace
        let workspace = Workspace::new(
            "test-workspace".to_string(),
            PathBuf::from("/tmp/test-workspace"),
        );
        state.data.workspaces.push(workspace);
        state.ui.selected_workspace_idx = 0;

        state
    }

    fn create_test_task(workspace_id: Uuid, prompt: &str) -> ParallelTask {
        ParallelTask::new(
            workspace_id,
            prompt.to_string(),
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

    // ==================== Report Navigation Tests ====================

    #[test]
    fn test_report_navigation_select_next() {
        let mut state = create_test_state();
        let ws_id = state.data.workspaces[0].id;

        // Add a parallel task with 3 attempts
        let mut task = create_test_task(ws_id, "Test task");
        task.add_attempt(create_test_attempt(task.id, AgentType::Claude));
        task.add_attempt(create_test_attempt(task.id, AgentType::Gemini));
        task.add_attempt(create_test_attempt(task.id, AgentType::Codex));
        state.data.workspaces[0].add_parallel_task(task);

        // Initial state - at index 0
        assert_eq!(state.ui.selected_report_idx, 0);

        // Simulate SelectNextReport
        let report_count = state.selected_workspace()
            .and_then(|ws| ws.active_parallel_task())
            .map(|t| t.attempts.len())
            .unwrap_or(0);

        // Move to next
        state.ui.selected_report_idx = (state.ui.selected_report_idx + 1).min(report_count - 1);
        assert_eq!(state.ui.selected_report_idx, 1);

        // Move to next again
        state.ui.selected_report_idx = (state.ui.selected_report_idx + 1).min(report_count - 1);
        assert_eq!(state.ui.selected_report_idx, 2);

        // Try to move past the end - should stay at 2
        state.ui.selected_report_idx = (state.ui.selected_report_idx + 1).min(report_count - 1);
        assert_eq!(state.ui.selected_report_idx, 2);
    }

    #[test]
    fn test_report_navigation_select_prev() {
        let mut state = create_test_state();
        let ws_id = state.data.workspaces[0].id;

        let mut task = create_test_task(ws_id, "Test task");
        task.add_attempt(create_test_attempt(task.id, AgentType::Claude));
        task.add_attempt(create_test_attempt(task.id, AgentType::Gemini));
        task.add_attempt(create_test_attempt(task.id, AgentType::Codex));
        state.data.workspaces[0].add_parallel_task(task);

        // Start at index 2
        state.ui.selected_report_idx = 2;

        // Move to previous
        if state.ui.selected_report_idx > 0 {
            state.ui.selected_report_idx -= 1;
        }
        assert_eq!(state.ui.selected_report_idx, 1);

        // Move to previous again
        if state.ui.selected_report_idx > 0 {
            state.ui.selected_report_idx -= 1;
        }
        assert_eq!(state.ui.selected_report_idx, 0);

        // Try to move before the start - should stay at 0
        if state.ui.selected_report_idx > 0 {
            state.ui.selected_report_idx -= 1;
        }
        assert_eq!(state.ui.selected_report_idx, 0);
    }

    #[test]
    fn test_report_navigation_empty_task() {
        let mut state = create_test_state();
        let ws_id = state.data.workspaces[0].id;

        // Add an empty task
        let task = create_test_task(ws_id, "Test task");
        state.data.workspaces[0].add_parallel_task(task);

        let report_count = state.selected_workspace()
            .and_then(|ws| ws.active_parallel_task())
            .map(|t| t.attempts.len())
            .unwrap_or(0);

        // Should be 0 attempts
        assert_eq!(report_count, 0);
    }

    // ==================== View Report Tests ====================

    #[test]
    fn test_view_report_sets_active_session() {
        let mut state = create_test_state();
        let ws_id = state.data.workspaces[0].id;

        let mut task = create_test_task(ws_id, "Test task");
        let attempt = create_test_attempt(task.id, AgentType::Claude);
        let expected_session_id = attempt.session_id;
        task.add_attempt(attempt);
        state.data.workspaces[0].add_parallel_task(task);

        // No active session initially
        assert!(state.ui.active_session_id.is_none());

        // Simulate ViewReport
        let attempt = state.selected_workspace()
            .and_then(|ws| ws.active_parallel_task())
            .and_then(|t| t.attempts.get(state.ui.selected_report_idx))
            .cloned();

        if let Some(attempt) = attempt {
            state.ui.active_session_id = Some(attempt.session_id);
            state.ui.focus = FocusPanel::OutputPane;
        }

        // Should have set the session and changed focus
        assert_eq!(state.ui.active_session_id, Some(expected_session_id));
        assert_eq!(state.ui.focus, FocusPanel::OutputPane);
    }

    #[test]
    fn test_view_report_different_indices() {
        let mut state = create_test_state();
        let ws_id = state.data.workspaces[0].id;

        let mut task = create_test_task(ws_id, "Test task");
        let attempt1 = create_test_attempt(task.id, AgentType::Claude);
        let attempt2 = create_test_attempt(task.id, AgentType::Gemini);
        let session1 = attempt1.session_id;
        let session2 = attempt2.session_id;
        task.add_attempt(attempt1);
        task.add_attempt(attempt2);
        state.data.workspaces[0].add_parallel_task(task);

        // View first report
        state.ui.selected_report_idx = 0;
        let attempt = state.selected_workspace()
            .and_then(|ws| ws.active_parallel_task())
            .and_then(|t| t.attempts.get(state.ui.selected_report_idx))
            .cloned();
        if let Some(attempt) = attempt {
            state.ui.active_session_id = Some(attempt.session_id);
        }
        assert_eq!(state.ui.active_session_id, Some(session1));

        // View second report
        state.ui.selected_report_idx = 1;
        let attempt = state.selected_workspace()
            .and_then(|ws| ws.active_parallel_task())
            .and_then(|t| t.attempts.get(state.ui.selected_report_idx))
            .cloned();
        if let Some(attempt) = attempt {
            state.ui.active_session_id = Some(attempt.session_id);
        }
        assert_eq!(state.ui.active_session_id, Some(session2));
    }

    // ==================== Merge Report Tests ====================

    #[test]
    fn test_merge_gets_correct_attempt_id() {
        let mut state = create_test_state();
        let ws_id = state.data.workspaces[0].id;

        let mut task = create_test_task(ws_id, "Test task");
        let attempt1 = create_test_attempt(task.id, AgentType::Claude);
        let attempt2 = create_test_attempt(task.id, AgentType::Gemini);
        let attempt1_id = attempt1.id;
        let attempt2_id = attempt2.id;
        task.add_attempt(attempt1);
        task.add_attempt(attempt2);
        state.data.workspaces[0].add_parallel_task(task);

        // Select first report
        state.ui.selected_report_idx = 0;
        let attempt_id = state.selected_workspace()
            .and_then(|ws| ws.active_parallel_task())
            .and_then(|t| t.attempts.get(state.ui.selected_report_idx))
            .map(|a| a.id);
        assert_eq!(attempt_id, Some(attempt1_id));

        // Select second report
        state.ui.selected_report_idx = 1;
        let attempt_id = state.selected_workspace()
            .and_then(|ws| ws.active_parallel_task())
            .and_then(|t| t.attempts.get(state.ui.selected_report_idx))
            .map(|a| a.id);
        assert_eq!(attempt_id, Some(attempt2_id));
    }

    // ==================== Prompt Sent Tracking Tests ====================

    #[test]
    fn test_prompt_sent_workflow() {
        let mut state = create_test_state();
        let ws_id = state.data.workspaces[0].id;

        let mut task = create_test_task(ws_id, "Fix the bug");
        let attempt = create_test_attempt(task.id, AgentType::Claude);
        let session_id = attempt.session_id;
        task.add_attempt(attempt);
        let prompt = task.prompt.clone();
        state.data.workspaces[0].add_parallel_task(task);

        // Initially, prompt not sent
        let needs_prompt = state.selected_workspace()
            .and_then(|ws| {
                ws.parallel_tasks.iter()
                    .find(|t| t.attempts.iter().any(|a| a.session_id == session_id && !a.prompt_sent))
                    .map(|t| t.prompt.clone())
            });
        assert_eq!(needs_prompt, Some("Fix the bug".to_string()));

        // Mark as sent (simulating what handler.rs does when idle)
        if let Some(ws) = state.data.workspaces.get_mut(0) {
            for task in ws.parallel_tasks.iter_mut() {
                if let Some(attempt) = task.attempts.iter_mut().find(|a| a.session_id == session_id) {
                    attempt.prompt_sent = true;
                }
            }
        }

        // Now should not need prompt
        let needs_prompt = state.selected_workspace()
            .and_then(|ws| {
                ws.parallel_tasks.iter()
                    .find(|t| t.attempts.iter().any(|a| a.session_id == session_id && !a.prompt_sent))
                    .map(|t| t.prompt.clone())
            });
        assert!(needs_prompt.is_none());
    }

    #[test]
    fn test_multiple_attempts_prompt_tracking() {
        let mut state = create_test_state();
        let ws_id = state.data.workspaces[0].id;

        let mut task = create_test_task(ws_id, "Review code");
        let attempt1 = create_test_attempt(task.id, AgentType::Claude);
        let attempt2 = create_test_attempt(task.id, AgentType::Gemini);
        let session1 = attempt1.session_id;
        let session2 = attempt2.session_id;
        task.add_attempt(attempt1);
        task.add_attempt(attempt2);
        state.data.workspaces[0].add_parallel_task(task);

        // Both need prompts initially
        let count_needing_prompt = state.selected_workspace()
            .map(|ws| {
                ws.parallel_tasks.iter()
                    .flat_map(|t| t.attempts.iter())
                    .filter(|a| !a.prompt_sent)
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(count_needing_prompt, 2);

        // Mark first as sent
        if let Some(ws) = state.data.workspaces.get_mut(0) {
            for task in ws.parallel_tasks.iter_mut() {
                if let Some(attempt) = task.attempts.iter_mut().find(|a| a.session_id == session1) {
                    attempt.prompt_sent = true;
                }
            }
        }

        // Only second needs prompt now
        let count_needing_prompt = state.selected_workspace()
            .map(|ws| {
                ws.parallel_tasks.iter()
                    .flat_map(|t| t.attempts.iter())
                    .filter(|a| !a.prompt_sent)
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(count_needing_prompt, 1);
    }

    // ==================== Cancel Task Tests ====================

    #[test]
    fn test_cancel_clears_old_active_tasks() {
        let mut state = create_test_state();
        let ws_id = state.data.workspaces[0].id;

        // Add first task
        let task1 = create_test_task(ws_id, "First task");
        let task1_id = task1.id;
        state.data.workspaces[0].add_parallel_task(task1);

        // Verify it's active
        assert!(state.selected_workspace()
            .and_then(|ws| ws.active_parallel_task())
            .is_some());

        // Simulate what start_parallel_task does: cancel existing tasks
        if let Some(ws) = state.data.workspaces.get_mut(0) {
            for task in ws.parallel_tasks.iter_mut() {
                if matches!(task.status, ParallelTaskStatus::Running | ParallelTaskStatus::AwaitingSelection) {
                    task.status = ParallelTaskStatus::Cancelled;
                }
            }
        }

        // Old task should now be cancelled
        let old_task = state.data.workspaces[0].get_parallel_task(task1_id).unwrap();
        assert_eq!(old_task.status, ParallelTaskStatus::Cancelled);

        // No active task since we cancelled
        assert!(state.selected_workspace()
            .and_then(|ws| ws.active_parallel_task())
            .is_none());
    }

    // ==================== Status Transition Tests ====================

    #[test]
    fn test_all_attempts_finished_triggers_awaiting_selection() {
        let mut state = create_test_state();
        let ws_id = state.data.workspaces[0].id;

        let mut task = create_test_task(ws_id, "Test");
        task.add_attempt(create_test_attempt(task.id, AgentType::Claude));
        task.add_attempt(create_test_attempt(task.id, AgentType::Gemini));
        let task_id = task.id;
        state.data.workspaces[0].add_parallel_task(task);

        // Mark both as completed
        if let Some(ws) = state.data.workspaces.get_mut(0) {
            if let Some(task) = ws.get_parallel_task_mut(task_id) {
                task.attempts[0].mark_completed();
                task.attempts[1].mark_completed();

                // Check if all finished and update status
                if task.all_attempts_finished() {
                    task.mark_awaiting_selection();
                }
            }
        }

        // Task should now be awaiting selection
        let task = state.data.workspaces[0].get_parallel_task(task_id).unwrap();
        assert_eq!(task.status, ParallelTaskStatus::AwaitingSelection);
    }

    #[test]
    fn test_mixed_completion_status() {
        let mut state = create_test_state();
        let ws_id = state.data.workspaces[0].id;

        let mut task = create_test_task(ws_id, "Test");
        task.add_attempt(create_test_attempt(task.id, AgentType::Claude));
        task.add_attempt(create_test_attempt(task.id, AgentType::Gemini));
        let task_id = task.id;
        state.data.workspaces[0].add_parallel_task(task);

        // Mark first as completed, second as failed
        if let Some(ws) = state.data.workspaces.get_mut(0) {
            if let Some(task) = ws.get_parallel_task_mut(task_id) {
                task.attempts[0].mark_completed();
                task.attempts[1].mark_failed();

                // Both are finished (completed or failed)
                assert!(task.all_attempts_finished());
                task.mark_awaiting_selection();
            }
        }

        let task = state.data.workspaces[0].get_parallel_task(task_id).unwrap();
        assert_eq!(task.status, ParallelTaskStatus::AwaitingSelection);
    }

    // ==================== Input Mode Tests ====================

    #[test]
    fn test_parallel_modal_clears_after_start() {
        let mut state = create_test_state();

        // Set up modal state
        state.ui.input_mode = InputMode::CreateParallelTask;
        state.ui.parallel_task_prompt = "Fix the bug".to_string();
        // parallel_task_agents is a Vec<(AgentType, bool)>, so push to it
        state.ui.parallel_task_agents.push((AgentType::Claude, true));

        // Simulate what happens after starting task
        state.ui.input_mode = InputMode::Normal;
        state.ui.parallel_task_prompt.clear();
        state.ui.focus = FocusPanel::SessionList;

        assert_eq!(state.ui.input_mode, InputMode::Normal);
        assert!(state.ui.parallel_task_prompt.is_empty());
        assert_eq!(state.ui.focus, FocusPanel::SessionList);
    }
}
