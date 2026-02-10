use crate::app::{Action, AppState, FocusPanel, InputMode, ParallelMergePlan, ParallelWorktreeSpec, PARSER_BUFFER_ROWS, TERMINAL_SCROLLBACK_LIMIT};
use crate::git;
use crate::models::{AttemptStatus, ParallelTask, ParallelTaskAttempt, ParallelTaskStatus, Session};
use crate::persistence;
use crate::pty::{PtyManager, SessionSpawnConfig};
use anyhow::{anyhow, Result};
use tokio::sync::mpsc;
use tokio::task;
use uuid::Uuid;

pub fn handle_parallel_action(
    state: &mut AppState,
    action: Action,
    pty_manager: &PtyManager,
    action_tx: &mpsc::UnboundedSender<Action>,
    pty_tx: &mpsc::Sender<Action>,
) -> Result<()> {
    match action {
        Action::StartParallelTask => {
            start_parallel_task(state, action_tx)?;
        }
        Action::CancelParallelTask(task_id) => {
            cancel_parallel_task(state, task_id, pty_manager)?;
        }
        Action::SelectNextReport | Action::SelectPrevReport => {
            handle_report_navigation(state, &action);
        }
        Action::ViewReport => {
            view_selected_report(state);
        }
        Action::MergeSelectedReport => {
            merge_selected_report(state)?;
        }
        Action::ConfirmParallelMerge => {
            confirm_parallel_merge(state, action_tx)?;
        }
        Action::CancelParallelMerge => {
            state.ui.merging_parallel_attempt_id = None;
            state.ui.input_mode = InputMode::Normal;
        }
        Action::ParallelAttemptCompleted(session_id) => {
            mark_attempt_completed(state, session_id)?;
        }
        Action::ParallelWorktreesReady {
            request_id,
            task_id,
            workspace_id,
            prompt,
            request_report,
            dangerously_skip_permissions,
            source_branch,
            source_commit,
            worktrees,
        } => {
            handle_parallel_worktrees_ready(
                state,
                pty_manager,
                pty_tx,
                request_id,
                task_id,
                workspace_id,
                prompt,
                request_report,
                dangerously_skip_permissions,
                source_branch,
                source_commit,
                worktrees,
            )?;
        }
        Action::ParallelWorktreesFailed { request_id, error: _error } => {
            if request_id == state.ui.parallel_task_request_id {
                // Don't use eprintln! in TUI - it corrupts the display
                // TODO: Add proper notification system for user feedback
            }
        }
        Action::ParallelMergeFinished { plan, error } => {
            handle_parallel_merge_finished(state, plan, error)?;
        }
        _ => {}
    }
    Ok(())
}

fn start_parallel_task(
    state: &mut AppState,
    action_tx: &mpsc::UnboundedSender<Action>,
) -> Result<()> {
    // Get selected agents
    let selected_agents: Vec<_> = state.ui.parallel_task_agents
        .iter()
        .filter(|(_, selected)| *selected)
        .map(|(agent_type, _)| agent_type.clone())
        .collect();

    // Validate at least 1 agent selected (allow single agent for testing)
    if selected_agents.is_empty() {
        state.ui.input_mode = InputMode::Normal;
        return Ok(());
    }

    // Validate prompt is not empty
    let prompt = state.ui.parallel_task_prompt.trim().to_string();
    if prompt.is_empty() {
        state.ui.input_mode = InputMode::Normal;
        return Ok(());
    }

    // Get workspace info
    let workspace = state.selected_workspace()
        .ok_or_else(|| anyhow!("No workspace selected"))?;
    let workspace_id = workspace.id;
    let workspace_path = workspace.path.clone();

    // Get settings from UI state
    let request_report = state.ui.parallel_task_request_report;
    let dangerously_skip_permissions = state.ui.parallel_task_dangerous_mode;

    let task_id = Uuid::new_v4();
    let task_short_id = task_id.to_string()[..8].to_string();

    let request_id = state.ui.parallel_task_request_id.wrapping_add(1);
    state.ui.parallel_task_request_id = request_id;

    // Cancel any existing active parallel tasks before creating a new one.
    if let Some(ws) = state.data.workspaces.get_mut(state.ui.selected_workspace_idx) {
        for task in ws.parallel_tasks.iter_mut() {
            if matches!(task.status, ParallelTaskStatus::Running | ParallelTaskStatus::AwaitingSelection) {
                task.status = ParallelTaskStatus::Cancelled;
            }
        }
    }

    // Reset modal state and switch to normal mode
    state.ui.input_mode = InputMode::Normal;
    state.ui.parallel_task_prompt.clear();
    state.ui.focus = FocusPanel::SessionList;

    let _ = persistence::save(&state.data.workspaces, &state.data.sessions);

    let action_tx = action_tx.clone();
    task::spawn_blocking(move || {
        if !git::is_git_repo(&workspace_path) {
            let _ = action_tx.send(Action::ParallelWorktreesFailed {
                request_id,
                error: "Not a git repository".to_string(),
            });
            return;
        }

        let source_branch = git::get_current_branch(&workspace_path)
            .unwrap_or_else(|_| "main".to_string());
        let source_commit = git::get_head_commit(&workspace_path)
            .unwrap_or_else(|_| "unknown".to_string());

        let mut worktrees = Vec::new();
        for agent_type in selected_agents {
            let agent_name = agent_type.badge().to_lowercase();
            let branch_name = format!("parallel-{}/{}", task_short_id, agent_name);
            let worktree_path = git::get_attempt_worktree_path(&workspace_path, &task_short_id, &agent_name);

            if git::create_worktree(&workspace_path, &branch_name, &worktree_path).is_err() {
                continue;
            }

            worktrees.push(ParallelWorktreeSpec {
                agent_type,
                branch_name,
                worktree_path,
            });
        }

        let _ = action_tx.send(Action::ParallelWorktreesReady {
            request_id,
            task_id,
            workspace_id,
            prompt,
            request_report,
            dangerously_skip_permissions,
            source_branch,
            source_commit,
            worktrees,
        });
    });

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_parallel_worktrees_ready(
    state: &mut AppState,
    pty_manager: &PtyManager,
    pty_tx: &mpsc::Sender<Action>,
    request_id: u64,
    task_id: Uuid,
    workspace_id: Uuid,
    prompt: String,
    request_report: bool,
    dangerously_skip_permissions: bool,
    source_branch: String,
    source_commit: String,
    worktrees: Vec<ParallelWorktreeSpec>,
) -> Result<()> {
    if request_id != state.ui.parallel_task_request_id {
        return Ok(());
    }

    let workspace_idx = match state.data.workspaces.iter().position(|ws| ws.id == workspace_id) {
        Some(idx) => idx,
        None => return Ok(()),
    };

    // Cancel any existing active parallel tasks before creating a new one.
    if let Some(ws) = state.data.workspaces.get_mut(workspace_idx) {
        for task in ws.parallel_tasks.iter_mut() {
            if matches!(task.status, ParallelTaskStatus::Running | ParallelTaskStatus::AwaitingSelection) {
                task.status = ParallelTaskStatus::Cancelled;
            }
        }
    }

    if worktrees.is_empty() {
        return Ok(());
    }

    let workspace_path = state.data.workspaces[workspace_idx].path.clone();

    let mut task = ParallelTask::new(
        workspace_id,
        prompt,
        source_branch,
        source_commit,
        request_report,
    );
    task.id = task_id;

    // Store the task.
    if let Some(ws) = state.data.workspaces.get_mut(workspace_idx) {
        ws.add_parallel_task(task);
    }

    // Create sessions for each prepared worktree.
    for spec in worktrees {
        let attempt = ParallelTaskAttempt::new(
            task_id,
            Uuid::nil(), // Will be updated after session creation
            spec.agent_type.clone(),
            spec.branch_name.clone(),
            spec.worktree_path.clone(),
        );
        let attempt_id = attempt.id;

        let session = Session::new_parallel(
            workspace_id,
            spec.agent_type.clone(),
            dangerously_skip_permissions,
            attempt_id,
        );
        let session_id = session.id;

        if let Some(ws) = state.data.workspaces.get_mut(workspace_idx) {
            if let Some(task) = ws.get_parallel_task_mut(task_id) {
                let mut updated_attempt = attempt;
                updated_attempt.session_id = session_id;
                task.add_attempt(updated_attempt);
            }
        }

        let pty_rows = state.pane_rows();
        let cols = state.output_pane_cols();
        let parser = vt100::Parser::new(PARSER_BUFFER_ROWS, cols, TERMINAL_SCROLLBACK_LIMIT);
        state.system.output_buffers.insert(session_id, parser);

        match pty_manager.spawn_session(SessionSpawnConfig {
            session_id,
            agent_type: spec.agent_type.clone(),
            working_dir: &spec.worktree_path,
            rows: pty_rows,
            cols,
            pty_tx: pty_tx.clone(),
            resume: false,
            dangerously_skip_permissions,
        }) {
            Ok(handle) => {
                state.system.pty_handles.insert(session_id, handle);
                state.add_session(session);
                state.data.last_activity.insert(session_id, std::time::Instant::now());
            }
            Err(_) => {
                state.system.output_buffers.remove(&session_id);
                let workspace_path = workspace_path.clone();
                let worktree_path = spec.worktree_path.clone();
                task::spawn_blocking(move || {
                    let _ = git::remove_worktree(&workspace_path, &worktree_path, true);
                });
            }
        }
    }

    let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
    Ok(())
}

fn cancel_parallel_task(state: &mut AppState, task_id: Uuid, _pty_manager: &PtyManager) -> Result<()> {
    let workspace_path = state.selected_workspace()
        .map(|w| w.path.clone());

    // First, collect all the info we need from the task
    let (workspace_id, session_ids, worktree_paths): (Option<Uuid>, Vec<Uuid>, Vec<std::path::PathBuf>) = {
        let ws = state.data.workspaces.get(state.ui.selected_workspace_idx);
        if let Some(ws) = ws {
            if let Some(task) = ws.get_parallel_task(task_id) {
                let ids: Vec<Uuid> = task.attempts.iter().map(|a| a.session_id).collect();
                let paths: Vec<std::path::PathBuf> = task.attempts.iter().map(|a| a.worktree_path.clone()).collect();
                (Some(ws.id), ids, paths)
            } else {
                (None, vec![], vec![])
            }
        } else {
            (None, vec![], vec![])
        }
    };

    // Kill all sessions and cleanup worktrees
    for session_id in &session_ids {
        if let Some(mut handle) = state.system.pty_handles.remove(session_id) {
            let _ = handle.kill();
        }
        state.system.output_buffers.remove(session_id);
    }

    // Remove worktrees
    if let Some(ref ws_path) = workspace_path {
        for worktree_path in &worktree_paths {
            let ws_path = ws_path.clone();
            let worktree_path = worktree_path.clone();
            task::spawn_blocking(move || {
                let _ = git::remove_worktree(&ws_path, &worktree_path, true);
            });
        }
    }

    // Remove sessions from state
    if let Some(ws_id) = workspace_id {
        if let Some(sessions) = state.data.sessions.get_mut(&ws_id) {
            sessions.retain(|s| !session_ids.contains(&s.id));
        }
    }

    // Mark task as cancelled and remove it
    if let Some(ws) = state.data.workspaces.get_mut(state.ui.selected_workspace_idx) {
        if let Some(task) = ws.get_parallel_task_mut(task_id) {
            task.mark_cancelled();
        }
        ws.remove_parallel_task(task_id);
    }

    let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
    Ok(())
}

fn select_parallel_winner(
    state: &mut AppState,
    attempt_id: Uuid,
    action_tx: &mpsc::UnboundedSender<Action>,
) -> Result<()> {
    // Find the task and attempt info — only collect the winner's session/worktree
    let (workspace_path, workspace_id, task_id, source_branch, winner_branch, winner_worktree_path, winner_session_id) = {
        let ws = state.selected_workspace()
            .ok_or_else(|| anyhow!("No workspace selected"))?;

        let task = ws.parallel_tasks.iter()
            .find(|t| t.attempts.iter().any(|a| a.id == attempt_id))
            .ok_or_else(|| anyhow!("Task not found"))?;

        let attempt = task.get_attempt(attempt_id)
            .ok_or_else(|| anyhow!("Attempt not found"))?;

        (
            ws.path.clone(),
            ws.id,
            task.id,
            task.source_branch.clone(),
            attempt.branch_name.clone(),
            attempt.worktree_path.clone(),
            attempt.session_id,
        )
    };

    let plan = ParallelMergePlan {
        workspace_path,
        workspace_id,
        task_id,
        winner_attempt_id: attempt_id,
        source_branch,
        winner_branch,
        winner_worktree_path,
        session_ids: vec![winner_session_id],
    };

    let action_tx = action_tx.clone();
    task::spawn_blocking(move || {
        // Auto-commit any uncommitted changes in the winner's worktree before merging.
        // Agents often edit files without committing — without this, those changes
        // would be lost when the worktree is force-removed during cleanup.
        if git::worktree_has_changes(&plan.winner_worktree_path) {
            let _ = git::commit_all_changes(
                &plan.winner_worktree_path,
                "parallel task: auto-commit uncommitted changes",
            );
        }

        let result = git::checkout_branch(&plan.workspace_path, &plan.source_branch)
            .and_then(|_| git::merge_branch(&plan.workspace_path, &plan.winner_branch));
        let error = result.err().map(|e| e.to_string());
        let _ = action_tx.send(Action::ParallelMergeFinished { plan, error });
    });

    Ok(())
}

fn handle_parallel_merge_finished(
    state: &mut AppState,
    plan: ParallelMergePlan,
    error: Option<String>,
) -> Result<()> {
    if error.is_some() {
        // Don't use eprintln! in TUI - it corrupts the display
        // TODO: Add proper notification system for user feedback
        return Ok(());
    }

    // Kill only the merged attempt's session
    for session_id in &plan.session_ids {
        if let Some(mut handle) = state.system.pty_handles.remove(session_id) {
            let _ = handle.kill();
        }
        state.system.output_buffers.remove(session_id);
    }

    // Remove the merged attempt's worktree
    {
        let workspace_path = plan.workspace_path.clone();
        let worktree_path = plan.winner_worktree_path.clone();
        task::spawn_blocking(move || {
            let _ = git::remove_worktree(&workspace_path, &worktree_path, true);
        });
    }

    // Remove the merged session from state
    if let Some(sessions) = state.data.sessions.get_mut(&plan.workspace_id) {
        sessions.retain(|s| !plan.session_ids.contains(&s.id));
    }

    // Remove only the merged attempt from the task
    if let Some(ws) = state.data.workspaces.iter_mut().find(|ws| ws.id == plan.workspace_id) {
        if let Some(task) = ws.get_parallel_task_mut(plan.task_id) {
            task.attempts.retain(|a| a.id != plan.winner_attempt_id);

            // If no attempts remain, mark completed and remove the task
            if task.attempts.is_empty() {
                task.mark_completed(plan.winner_attempt_id);
                ws.remove_parallel_task(plan.task_id);
            }
        }
    }

    // Adjust selected report index if it's now out of bounds
    let report_count = state.selected_workspace()
        .and_then(|ws| ws.active_parallel_task())
        .map(|t| t.attempts.len())
        .unwrap_or(0);
    if state.ui.selected_report_idx >= report_count && report_count > 0 {
        state.ui.selected_report_idx = report_count - 1;
    }

    let _ = persistence::save(&state.data.workspaces, &state.data.sessions);

    Ok(())
}

fn mark_attempt_completed(state: &mut AppState, session_id: Uuid) -> Result<()> {
    // Find and update the attempt
    for ws in state.data.workspaces.iter_mut() {
        for task in ws.parallel_tasks.iter_mut() {
            if let Some(attempt) = task.attempts.iter_mut()
                .find(|a| a.session_id == session_id)
            {
                attempt.status = AttemptStatus::Completed;

                // Try to read the report file from the worktree
                if task.request_report {
                    let report_path = attempt.worktree_path.join("PARALLEL_REPORT.md");
                    if report_path.exists() {
                        if let Ok(content) = std::fs::read_to_string(&report_path) {
                            attempt.set_report(content);
                        }
                    }
                }

                // Check if all attempts are done
                if task.all_attempts_finished() {
                    task.mark_awaiting_selection();
                }

                let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
                return Ok(());
            }
        }
    }
    Ok(())
}

fn handle_report_navigation(state: &mut AppState, action: &Action) {
    let report_count = state.selected_workspace()
        .and_then(|ws| ws.active_parallel_task())
        .map(|t| t.attempts.len())
        .unwrap_or(0);

    if report_count == 0 {
        return;
    }

    match action {
        Action::SelectNextReport => {
            state.ui.selected_report_idx = (state.ui.selected_report_idx + 1).min(report_count - 1);
        }
        Action::SelectPrevReport => {
            if state.ui.selected_report_idx > 0 {
                state.ui.selected_report_idx -= 1;
            }
        }
        _ => {}
    }
}

fn view_selected_report(state: &mut AppState) {
    // Get the selected attempt
    let attempt = state.selected_workspace()
        .and_then(|ws| ws.active_parallel_task())
        .and_then(|t| t.attempts.get(state.ui.selected_report_idx))
        .cloned();

    if let Some(attempt) = attempt {
        // Set the active session to view the output
        state.ui.active_session_id = Some(attempt.session_id);
        state.ui.focus = FocusPanel::OutputPane;
    }
}

fn merge_selected_report(state: &mut AppState) -> Result<()> {
    // Get the selected attempt ID
    let attempt_id = state.selected_workspace()
        .and_then(|ws| ws.active_parallel_task())
        .and_then(|t| t.attempts.get(state.ui.selected_report_idx))
        .map(|a| a.id);

    if let Some(attempt_id) = attempt_id {
        // Show confirmation modal instead of merging directly
        state.ui.merging_parallel_attempt_id = Some(attempt_id);
        state.ui.input_mode = InputMode::ConfirmParallelMerge;
    }

    Ok(())
}

fn confirm_parallel_merge(
    state: &mut AppState,
    action_tx: &mpsc::UnboundedSender<Action>,
) -> Result<()> {
    let attempt_id = state.ui.merging_parallel_attempt_id.take();
    state.ui.input_mode = InputMode::Normal;

    if let Some(attempt_id) = attempt_id {
        select_parallel_winner(state, attempt_id, action_tx)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::app::{Action, AppState, FocusPanel, InputMode, ParallelWorktreeSpec};
    use super::handle_parallel_action;
    use crate::models::{
        AgentType, AttemptStatus, ParallelTask, ParallelTaskAttempt, ParallelTaskStatus, Workspace,
    };
    use crate::pty::PtyManager;
    use std::path::PathBuf;
    use tokio::sync::mpsc;
    use uuid::Uuid;

    fn create_test_state() -> AppState {
        let mut state = AppState::default();
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
            false,
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

        let mut task = create_test_task(ws_id, "Test task");
        task.add_attempt(create_test_attempt(task.id, AgentType::Claude));
        task.add_attempt(create_test_attempt(task.id, AgentType::Gemini));
        task.add_attempt(create_test_attempt(task.id, AgentType::Codex));
        state.data.workspaces[0].add_parallel_task(task);

        assert_eq!(state.ui.selected_report_idx, 0);

        let report_count = state.selected_workspace()
            .and_then(|ws| ws.active_parallel_task())
            .map(|t| t.attempts.len())
            .unwrap_or(0);

        state.ui.selected_report_idx = (state.ui.selected_report_idx + 1).min(report_count - 1);
        assert_eq!(state.ui.selected_report_idx, 1);

        state.ui.selected_report_idx = (state.ui.selected_report_idx + 1).min(report_count - 1);
        assert_eq!(state.ui.selected_report_idx, 2);

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

        state.ui.selected_report_idx = 2;

        if state.ui.selected_report_idx > 0 {
            state.ui.selected_report_idx -= 1;
        }
        assert_eq!(state.ui.selected_report_idx, 1);

        if state.ui.selected_report_idx > 0 {
            state.ui.selected_report_idx -= 1;
        }
        assert_eq!(state.ui.selected_report_idx, 0);

        if state.ui.selected_report_idx > 0 {
            state.ui.selected_report_idx -= 1;
        }
        assert_eq!(state.ui.selected_report_idx, 0);
    }

    #[test]
    fn test_report_navigation_empty_task() {
        let mut state = create_test_state();
        let ws_id = state.data.workspaces[0].id;

        let task = create_test_task(ws_id, "Test task");
        state.data.workspaces[0].add_parallel_task(task);

        let report_count = state.selected_workspace()
            .and_then(|ws| ws.active_parallel_task())
            .map(|t| t.attempts.len())
            .unwrap_or(0);

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

        assert!(state.ui.active_session_id.is_none());

        let attempt = state.selected_workspace()
            .and_then(|ws| ws.active_parallel_task())
            .and_then(|t| t.attempts.get(state.ui.selected_report_idx))
            .cloned();

        if let Some(attempt) = attempt {
            state.ui.active_session_id = Some(attempt.session_id);
            state.ui.focus = FocusPanel::OutputPane;
        }

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

        state.ui.selected_report_idx = 0;
        let attempt = state.selected_workspace()
            .and_then(|ws| ws.active_parallel_task())
            .and_then(|t| t.attempts.get(state.ui.selected_report_idx))
            .cloned();
        if let Some(attempt) = attempt {
            state.ui.active_session_id = Some(attempt.session_id);
        }
        assert_eq!(state.ui.active_session_id, Some(session1));

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

        state.ui.selected_report_idx = 0;
        let attempt_id = state.selected_workspace()
            .and_then(|ws| ws.active_parallel_task())
            .and_then(|t| t.attempts.get(state.ui.selected_report_idx))
            .map(|a| a.id);
        assert_eq!(attempt_id, Some(attempt1_id));

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
        let _prompt = task.prompt.clone();
        state.data.workspaces[0].add_parallel_task(task);

        let needs_prompt = state.selected_workspace()
            .and_then(|ws| {
                ws.parallel_tasks.iter()
                    .find(|t| t.attempts.iter().any(|a| a.session_id == session_id && !a.prompt_sent))
                    .map(|t| t.prompt.clone())
            });
        assert_eq!(needs_prompt, Some("Fix the bug".to_string()));

        if let Some(ws) = state.data.workspaces.get_mut(0) {
            for task in ws.parallel_tasks.iter_mut() {
                if let Some(attempt) = task.attempts.iter_mut().find(|a| a.session_id == session_id) {
                    attempt.prompt_sent = true;
                }
            }
        }

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
        let _session2 = attempt2.session_id;
        task.add_attempt(attempt1);
        task.add_attempt(attempt2);
        state.data.workspaces[0].add_parallel_task(task);

        let count_needing_prompt = state.selected_workspace()
            .map(|ws| {
                ws.parallel_tasks.iter()
                    .flat_map(|t| t.attempts.iter())
                    .filter(|a| !a.prompt_sent)
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(count_needing_prompt, 2);

        if let Some(ws) = state.data.workspaces.get_mut(0) {
            for task in ws.parallel_tasks.iter_mut() {
                if let Some(attempt) = task.attempts.iter_mut().find(|a| a.session_id == session1) {
                    attempt.prompt_sent = true;
                }
            }
        }

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

        let task1 = create_test_task(ws_id, "First task");
        let task1_id = task1.id;
        state.data.workspaces[0].add_parallel_task(task1);

        assert!(state.selected_workspace()
            .and_then(|ws| ws.active_parallel_task())
            .is_some());

        if let Some(ws) = state.data.workspaces.get_mut(0) {
            for task in ws.parallel_tasks.iter_mut() {
                if matches!(task.status, ParallelTaskStatus::Running | ParallelTaskStatus::AwaitingSelection) {
                    task.status = ParallelTaskStatus::Cancelled;
                }
            }
        }

        let old_task = state.data.workspaces[0].get_parallel_task(task1_id).unwrap();
        assert_eq!(old_task.status, ParallelTaskStatus::Cancelled);

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

        if let Some(ws) = state.data.workspaces.get_mut(0) {
            if let Some(task) = ws.get_parallel_task_mut(task_id) {
                task.attempts[0].status = AttemptStatus::Completed;
                task.attempts[1].status = AttemptStatus::Completed;

                if task.all_attempts_finished() {
                    task.mark_awaiting_selection();
                }
            }
        }

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

        if let Some(ws) = state.data.workspaces.get_mut(0) {
            if let Some(task) = ws.get_parallel_task_mut(task_id) {
                task.attempts[0].status = AttemptStatus::Completed;
                task.attempts[1].status = AttemptStatus::Failed;

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

        state.ui.input_mode = InputMode::CreateParallelTask;
        state.ui.parallel_task_prompt = "Fix the bug".to_string();
        state.ui.parallel_task_agents.push((AgentType::Claude, true));

        state.ui.input_mode = InputMode::Normal;
        state.ui.parallel_task_prompt.clear();
        state.ui.focus = FocusPanel::SessionList;

        assert_eq!(state.ui.input_mode, InputMode::Normal);
        assert!(state.ui.parallel_task_prompt.is_empty());
        assert_eq!(state.ui.focus, FocusPanel::SessionList);
    }

    #[test]
    fn test_parallel_worktrees_ready_ignores_stale_request() {
        let mut state = create_test_state();
        let ws_id = state.data.workspaces[0].id;

        let task = create_test_task(ws_id, "Test task");
        state.data.workspaces[0].add_parallel_task(task);

        state.ui.parallel_task_request_id = 2;

        let action = Action::ParallelWorktreesReady {
            request_id: 1,
            task_id: Uuid::new_v4(),
            workspace_id: ws_id,
            prompt: "Prompt".to_string(),
            request_report: false,
            dangerously_skip_permissions: false,
            source_branch: "main".to_string(),
            source_commit: "abc123".to_string(),
            worktrees: Vec::new(),
        };

        let pty_manager = PtyManager::new();
        let (action_tx, _) = mpsc::unbounded_channel();
        let (pty_tx, _) = mpsc::channel(1);

        handle_parallel_action(&mut state, action, &pty_manager, &action_tx, &pty_tx).unwrap();

        assert_eq!(
            state.data.workspaces[0].parallel_tasks[0].status,
            ParallelTaskStatus::Running
        );
    }

    #[test]
    fn test_parallel_worktrees_ready_cancels_active_task_on_match() {
        let mut state = create_test_state();
        let ws_id = state.data.workspaces[0].id;

        let task = create_test_task(ws_id, "Test task");
        state.data.workspaces[0].add_parallel_task(task);

        state.ui.parallel_task_request_id = 2;

        let action = Action::ParallelWorktreesReady {
            request_id: 2,
            task_id: Uuid::new_v4(),
            workspace_id: ws_id,
            prompt: "Prompt".to_string(),
            request_report: false,
            dangerously_skip_permissions: false,
            source_branch: "main".to_string(),
            source_commit: "abc123".to_string(),
            worktrees: Vec::<ParallelWorktreeSpec>::new(),
        };

        let pty_manager = PtyManager::new();
        let (action_tx, _) = mpsc::unbounded_channel();
        let (pty_tx, _) = mpsc::channel(1);

        handle_parallel_action(&mut state, action, &pty_manager, &action_tx, &pty_tx).unwrap();

        assert_eq!(
            state.data.workspaces[0].parallel_tasks[0].status,
            ParallelTaskStatus::Cancelled
        );
    }
}
