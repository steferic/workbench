use crate::app::{Action, AppState};
use crate::git;
use crate::pty::{PtyManager, SessionSpawnConfig};
use anyhow::Result;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;

use super::handlers::{config, input, navigation, parallel, session, todo, workspace};
use super::pty_ops::resize_ptys_to_panes;

const AGENT_DONE_WAV: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/sounds/agent_done.wav");

pub fn process_action(
    state: &mut AppState,
    action: Action,
    pty_manager: &PtyManager,
    action_tx: &mpsc::UnboundedSender<Action>,
    pty_tx: &mpsc::Sender<Action>,
) -> Result<()> {
    match action {
        Action::Quit | Action::ConfirmQuit => {
            // Kill all active sessions before quitting
            let handles: Vec<_> = state.system.pty_handles.drain().collect();
            for (session_id, handle) in handles {
                // Check if this is a terminal session
                let is_terminal = state.data.sessions.values()
                    .flat_map(|sessions| sessions.iter())
                    .find(|s| s.id == session_id)
                    .map(|s| s.agent_type.is_terminal())
                    .unwrap_or(false);

                session::terminate_session_handle(handle, is_terminal);
            }
            state.system.should_quit = true;
        }
        Action::Tick => {
            state.tick_animation();
            navigation::handle_drag_auto_scroll(state);
            let newly_idle = state.update_idle_queue();

            // Play notification sound when agents go idle (debounced: max once per 5s)
            if state.system.agent_done_sound_enabled
                && !newly_idle.is_empty()
                && state.system.last_agent_done_sound.elapsed().as_secs() >= 5
            {
                state.system.last_agent_done_sound = std::time::Instant::now();
                crate::audio::play_sound(AGENT_DONE_WAV);
            }

            // Check if analyzer session went idle
            if let Some(analyzer_id) = state.ui.analyzer_session_id {
                if newly_idle.contains(&analyzer_id) {
                    if let Some(parser) = state.system.output_buffers.get(&analyzer_id) {
                        let screen = parser.screen();
                        let contents = screen.contents();
                        for line in contents.lines() {
                            if let Some(idx) = line.find("TODO: ") {
                                let todo_text = line[idx + 6..].trim();
                                if !todo_text.is_empty() && todo_text.len() > 5 {
                                    let clean_text: String = todo_text.chars().filter(|c| !c.is_control()).collect();
                                    let _ = action_tx.send(Action::AddSuggestedTodo(clean_text));
                                }
                            }
                        }
                    }
                    state.ui.analyzer_session_id = None;
                }
            }

            // Process newly idle sessions
            for session_id in &newly_idle {
                if let Some(workspace_id) = state.workspace_id_for_session(*session_id) {
                    let has_in_progress = state.get_workspace(workspace_id)
                        .and_then(|ws| ws.todo_for_session(*session_id))
                        .map(|t| t.is_in_progress())
                        .unwrap_or(false);

                    if has_in_progress {
                        if let Some(ws) = state.get_workspace_mut(workspace_id) {
                            if let Some(todo) = ws.todo_for_session_mut(*session_id) {
                                let _ = action_tx.send(Action::MarkTodoReadyForReview(todo.id));
                            }
                        }
                    }

                    // Check if this is a parallel task session
                    let parallel_info = state.get_workspace(workspace_id)
                        .and_then(|ws| {
                            ws.parallel_tasks.iter()
                                .find(|t| t.attempts.iter().any(|a| a.session_id == *session_id))
                                .and_then(|t| {
                                    t.attempts.iter()
                                        .find(|a| a.session_id == *session_id)
                                        .map(|a| (t.full_prompt(), a.prompt_sent, a.status))
                                })
                        });

                    if let Some((full_prompt, prompt_sent, attempt_status)) = parallel_info {
                        use crate::models::AttemptStatus;

                        if !prompt_sent {
                            // Send the prompt to the agent
                            let text_bytes: Vec<u8> = full_prompt.bytes().collect();
                            let _ = action_tx.send(Action::SendInput(*session_id, text_bytes));
                            let _ = action_tx.send(Action::SendInput(*session_id, vec![b'\r']));

                            // Mark the prompt as sent
                            if let Some(ws) = state.get_workspace_mut(workspace_id) {
                                for task in ws.parallel_tasks.iter_mut() {
                                    if let Some(attempt) = task.attempts.iter_mut().find(|a| a.session_id == *session_id) {
                                        attempt.prompt_sent = true;
                                    }
                                }
                            }
                        } else if attempt_status == AttemptStatus::Running {
                            // Agent already received prompt and is now idle again - it's done!
                            let _ = action_tx.send(Action::ParallelAttemptCompleted(*session_id));
                        }
                    }
                }
            }

            // Autorun dispatch
            if state.ui.todo_pane_mode == crate::app::TodoPaneMode::Autorun {
                for &session_id in &state.data.idle_queue {
                    if let Some(workspace_id) = state.workspace_id_for_session(session_id) {
                        let has_in_progress = state.get_workspace(workspace_id)
                            .and_then(|ws| ws.todo_for_session(session_id))
                            .map(|t| t.is_in_progress())
                            .unwrap_or(false);

                        if !has_in_progress {
                            let pending = state.get_workspace(workspace_id)
                                .and_then(|ws| ws.next_pending_todo())
                                .map(|t| (t.id, t.description.clone()));

                            if let Some((id, desc)) = pending {
                                let _ = action_tx.send(Action::DispatchTodoToSession(session_id, id, desc));
                                break;
                            }
                        }
                    }
                }
            }

            // Refresh diff stats every 5 seconds
            if state.system.last_diff_refresh.elapsed() >= Duration::from_secs(5) {
                state.system.last_diff_refresh = std::time::Instant::now();

                // Collect unique (path, Option<base>) pairs
                let mut diff_requests: HashMap<std::path::PathBuf, Option<String>> = HashMap::new();

                for ws in &state.data.workspaces {
                    // Workspace path → diff vs HEAD (uncommitted changes)
                    diff_requests.entry(ws.path.clone()).or_insert(None);

                    // Parallel task attempts → diff vs source_branch
                    for task in &ws.parallel_tasks {
                        for attempt in &task.attempts {
                            diff_requests.entry(attempt.worktree_path.clone())
                                .or_insert_with(|| Some(task.source_branch.clone()));
                        }
                    }

                    // Session worktrees → diff vs main workspace branch
                    if let Some(sessions) = state.data.sessions.get(&ws.id) {
                        for session in sessions {
                            if let Some(ref wt_path) = session.worktree_path {
                                if !diff_requests.contains_key(wt_path) {
                                    // Use the workspace's current branch as base
                                    let base = git::get_current_branch_fast(&ws.path);
                                    diff_requests.insert(wt_path.clone(), base);
                                }
                            }
                        }
                    }
                }

                if !diff_requests.is_empty() {
                    let tx = action_tx.clone();
                    tokio::task::spawn_blocking(move || {
                        let mut stats = HashMap::new();
                        for (path, base) in diff_requests {
                            if path.exists() {
                                let stat = git::get_diff_shortstat(&path, base.as_deref());
                                stats.insert(path, stat);
                            }
                        }
                        let _ = tx.send(Action::DiffStatsUpdated(stats));
                    });
                }
            }

            // Handle pending config terminal (from config tree)
            if let Some(config_dir) = state.system.pending_config_terminal.take() {
                // Create a terminal session in the config directory
                if let Some(ws) = state.selected_workspace() {
                    let workspace_id = ws.id;

                    // Get directory name for terminal name
                    let dir_name = config_dir.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("config")
                        .to_string();

                    // Create terminal session
                    let agent_type = crate::models::AgentType::Terminal(dir_name.clone());
                    let mut session = crate::models::Session::new(workspace_id, agent_type.clone(), false);

                    // Set ls as the start command
                    session.start_command = Some("ls".to_string());

                    let session_id = session.id;
                    state.add_session(session);

                    // Spawn the terminal session in the config directory
                    let pty_rows = state.pane_rows();
                    let pty_cols = state.output_pane_cols();

                    match pty_manager.spawn_session(SessionSpawnConfig {
                        session_id,
                        agent_type,
                        working_dir: &config_dir,
                        rows: pty_rows,
                        cols: pty_cols,
                        pty_tx: pty_tx.clone(),
                        resume: false,
                        dangerously_skip_permissions: false,
                    }) {
                        Ok(handle) => {
                            state.system.pty_handles.insert(session_id, handle);
                            state.system.create_session_buffers(session_id, pty_cols, false);

                            // Mark session as running
                            if let Some(s) = state.get_session_mut(session_id) {
                                s.status = crate::models::SessionStatus::Running;
                            }

                            // Activate the session so it shows in the center pane
                            state.ui.active_session_id = Some(session_id);
                            state.ui.output_scroll_offset = 0;
                            state.ui.focus = crate::app::FocusPanel::OutputPane;

                            // Run ls after a short delay to show directory contents
                            let tx = action_tx.clone();
                            let sid = session_id;
                            tokio::spawn(async move {
                                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                                let _ = tx.send(Action::SendInput(sid, b"ls".to_vec()));
                                let _ = tx.send(Action::SendInput(sid, vec![b'\r']));
                            });
                        }
                        Err(_) => {
                            // Failed to spawn, remove the session
                            state.delete_session(session_id);
                        }
                    }
                }
            }
        }
        Action::UtilityContentLoaded(payload) => {
            if payload.request_id == state.ui.utility_request_id {
                state.ui.utility_content = payload.content;
                state.ui.pie_chart_data = payload.pie_chart_data;
                state.ui.show_calendar = payload.show_calendar;
            }
        }
        Action::DiffStatsUpdated(stats) => {
            state.system.diff_stats = stats;
        }
        Action::Resize(w, h) => {
            state.system.terminal_size = (w, h);
            resize_ptys_to_panes(state);
        }
        // Dispatch to specialized handlers
        _ => {
            // Try each handler in turn. They return Ok(()) if they handled it or ignored it.
            // Since Action is consumed, we need to clone it if we were chaining, but here we can just pattern match.
            // Actually, my handlers take Action by value. I need to dispatch based on action variant.
            // But implementing a huge match again here defeats the purpose.
            // The specialized handlers internally match on the actions they care about and ignore others.
            // So I should clone the action? No, Action might not be cloneable (it is derived Clone though).
            // Better: match here and call the right handler.
            
            match action {
                // Workspace actions
                Action::ToggleWorkspaceStatus | Action::InitiateDeleteWorkspace(_, _) |
                Action::ConfirmDeleteWorkspace | Action::EnterWorkspaceActionMode |
                Action::NextWorkspaceChoice | Action::PrevWorkspaceChoice |
                Action::ConfirmWorkspaceChoice | Action::EnterWorkspaceNameMode |
                Action::CreateNewWorkspace(_) => {
                    workspace::handle_workspace_action(state, action, pty_manager, action_tx, pty_tx)?;
                }

                // Session actions
                Action::CreateSession(_, _, _) | Action::CreateTerminal |
                Action::ActivateSession(_) | Action::RestartSession(_) | Action::StopSession(_) |
                Action::KillSession(_) | Action::InitiateDeleteSession(_, _) |
                Action::ConfirmDeleteSession | Action::CancelPendingDelete | Action::EnterCreateSessionMode |
                Action::EnterSetStartCommandMode | Action::SetStartCommand(_, _) | Action::PinSession(_) |
                Action::UnpinSession(_) | Action::UnpinFocusedSession | Action::ToggleSplitView |
                Action::SessionExited(_, _) | Action::PtyOutput(_, _) | Action::SendInput(_, _) |
                Action::MergeSessionWorktree(_) | Action::SwitchToWorktree(_) |
                Action::ConfirmMergeWithCommit | Action::CancelMerge => {
                    session::handle_session_action(state, action, pty_manager, action_tx, pty_tx)?;
                }

                // Todo actions
                Action::SelectNextTodo | Action::SelectPrevTodo | Action::EnterCreateTodoMode |
                Action::CreateTodo(_) | Action::MarkTodoDone | Action::RunSelectedTodo |
                Action::ToggleTodoPaneMode | Action::InitiateDeleteTodo(_, _) | Action::ConfirmDeleteTodo |
                Action::DispatchTodoToSession(_, _, _) | Action::MarkTodoReadyForReview(_) |
                Action::AddSuggestedTodo(_) | Action::ApproveSuggestedTodo(_) |
                Action::ApproveAllSuggestedTodos | Action::ArchiveTodo(_) | Action::ToggleTodosTab |
                Action::ActivateUtility => {
                    todo::handle_todo_action(state, action, action_tx)?;
                }

                // Navigation actions
                Action::MoveUp | Action::MoveDown | Action::FocusLeft | Action::FocusRight |
                Action::NextPinnedPane | Action::PrevPinnedPane | Action::ScrollOutputUp |
                Action::ScrollOutputDown | Action::CycleNextWorkspace | Action::CycleNextSession |
                Action::MouseClick(_, _) |
                Action::MouseDrag(_, _) | Action::MouseUp(_, _) | Action::CopySelection |
                Action::Paste(_) | Action::ClearSelection | Action::SelectNextUtility |
                Action::SelectPrevUtility | Action::ToggleUtilitySection |
                Action::ToggleConfigItem | Action::ToggleBrownNoise | Action::ToggleClassicalRadio |
                Action::ToggleOceanWaves | Action::ToggleWindChimes | Action::ToggleRainforestRain => {
                    navigation::handle_navigation_action(state, action, pty_manager, pty_tx)?;
                }

                // Input actions
                Action::ExitMode | Action::InputChar(_) |
                Action::InputBackspace | Action::NotepadInput(_) |
                Action::FileBrowserUp | Action::FileBrowserDown | Action::FileBrowserEnter |
                Action::FileBrowserBack | Action::FileBrowserSelect |
                // Parallel task modal input actions
                Action::EnterParallelTaskMode | Action::ToggleParallelAgent(_) |
                Action::NextParallelAgent | Action::PrevParallelAgent |
                // Quit confirmation actions
                Action::InitiateQuit | Action::CancelQuit => {
                    input::handle_input_action(state, action)?;
                }

                // Parallel task execution actions
                Action::StartParallelTask | Action::CancelParallelTask(_) |
                Action::ParallelAttemptCompleted(_) |
                Action::ParallelWorktreesReady { .. } | Action::ParallelWorktreesFailed { .. } |
                Action::ParallelMergeFinished { .. } |
                Action::SelectNextReport | Action::SelectPrevReport |
                Action::ViewReport | Action::MergeSelectedReport |
                Action::ConfirmParallelMerge | Action::CancelParallelMerge => {
                    parallel::handle_parallel_action(state, action, pty_manager, action_tx, pty_tx)?;
                }

                // Debug overlay toggle
                Action::ToggleDebugOverlay => {
                    state.ui.show_debug_overlay = !state.ui.show_debug_overlay;
                }

                // Config window actions
                Action::EnterConfigWindow => {
                    state.ui.input_mode = crate::app::InputMode::ConfigWindow;
                    state.ui.config_tab = crate::app::ConfigTab::Agents;
                    state.ui.config_selected_row = 0;
                    state.ui.config_selected_col = 0;
                    state.ui.config_editing = false;
                    state.ui.config_rebinding = false;
                }
                Action::ExitConfigWindow => {
                    state.ui.input_mode = crate::app::InputMode::Normal;
                    state.ui.config_editing = false;
                    state.ui.config_rebinding = false;
                }
                Action::ConfigSwitchTab(_) | Action::ConfigMoveUp | Action::ConfigMoveDown |
                Action::ConfigMoveLeft | Action::ConfigMoveRight | Action::ConfigStartEdit |
                Action::ConfigFinishEdit | Action::ConfigCancelEdit | Action::ConfigAddAgent |
                Action::ConfigDeleteAgent | Action::ConfigReorderUp | Action::ConfigReorderDown |
                Action::ConfigResetDefault | Action::ConfigInputChar(_) |
                Action::ConfigInputBackspace | Action::ConfigRebindKey(_) => {
                    config::handle_config_action(state, action);
                }

                // Global already handled
                Action::Quit | Action::ConfirmQuit | Action::Tick | Action::Resize(_, _) |
                Action::UtilityContentLoaded(_) | Action::DiffStatsUpdated(_) => {}
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::process_action;
    use crate::app::{Action, AppState, UtilityContentPayload};
    use crate::pty::PtyManager;
    use ratatui::style::Color;
    use tokio::sync::mpsc;

    #[test]
    fn utility_content_loaded_ignores_stale_request() {
        let mut state = AppState::default();
        state.ui.utility_request_id = 2;
        state.ui.utility_content = vec!["old".to_string()];
        state.ui.pie_chart_data = vec![("old".to_string(), 1.0, Color::Blue)];
        state.ui.show_calendar = true;

        let payload = UtilityContentPayload {
            request_id: 1,
            content: vec!["new".to_string()],
            pie_chart_data: vec![("new".to_string(), 2.0, Color::Red)],
            show_calendar: false,
        };

        let pty_manager = PtyManager::new();
        let (action_tx, _) = mpsc::unbounded_channel();
        let (pty_tx, _) = mpsc::channel(1);

        process_action(
            &mut state,
            Action::UtilityContentLoaded(payload),
            &pty_manager,
            &action_tx,
            &pty_tx,
        )
        .unwrap();

        assert_eq!(state.ui.utility_content, vec!["old".to_string()]);
        assert_eq!(state.ui.pie_chart_data.len(), 1);
        assert_eq!(state.ui.pie_chart_data[0].0, "old");
        assert!(state.ui.show_calendar);
    }

    #[test]
    fn utility_content_loaded_updates_current_request() {
        let mut state = AppState::default();
        state.ui.utility_request_id = 3;
        state.ui.utility_content = vec!["old".to_string()];
        state.ui.pie_chart_data = vec![("old".to_string(), 1.0, Color::Blue)];
        state.ui.show_calendar = false;

        let payload = UtilityContentPayload {
            request_id: 3,
            content: vec!["new".to_string()],
            pie_chart_data: vec![("new".to_string(), 2.0, Color::Red)],
            show_calendar: true,
        };

        let pty_manager = PtyManager::new();
        let (action_tx, _) = mpsc::unbounded_channel();
        let (pty_tx, _) = mpsc::channel(1);

        process_action(
            &mut state,
            Action::UtilityContentLoaded(payload),
            &pty_manager,
            &action_tx,
            &pty_tx,
        )
        .unwrap();

        assert_eq!(state.ui.utility_content, vec!["new".to_string()]);
        assert_eq!(state.ui.pie_chart_data.len(), 1);
        assert_eq!(state.ui.pie_chart_data[0].0, "new");
        assert!(state.ui.show_calendar);
    }
}
