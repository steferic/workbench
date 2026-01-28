use crate::app::{Action, AppState, FocusPanel, InputMode, PendingDelete, TodoPaneMode, TodosTab, UtilityItem};
use crate::models::SessionStatus;
use crate::persistence;
use crate::app::utilities::load_utility_content;
use anyhow::Result;
use tokio::sync::mpsc;

pub fn handle_todo_action(
    state: &mut AppState,
    action: Action,
    action_tx: &mpsc::UnboundedSender<Action>,
) -> Result<()> {
    match action {
        Action::SelectNextTodo => {
            if let Some(ws) = state.selected_workspace() {
                let count = match state.ui.selected_todos_tab {
                    TodosTab::Active => ws.todos.iter().filter(|t| !t.is_archived()).count(),
                    TodosTab::Archived => ws.todos.iter().filter(|t| t.is_archived()).count(),
                    TodosTab::Reports => {
                        // Count reports from active parallel task
                        ws.active_parallel_task()
                            .map(|t| t.attempts.len())
                            .unwrap_or(0)
                    }
                };
                if count > 0 {
                    state.ui.selected_todo_idx = (state.ui.selected_todo_idx + 1).min(count - 1);
                }
            }
        }
        Action::SelectPrevTodo => {
            if state.ui.selected_todo_idx > 0 {
                state.ui.selected_todo_idx -= 1;
            }
        }
        Action::EnterCreateTodoMode => {
            state.ui.input_mode = InputMode::CreateTodo;
            state.ui.input_buffer.clear();
        }
        Action::CreateTodo(description) => {
            if let Some(ws) = state.data.workspaces.get_mut(state.ui.selected_workspace_idx) {
                ws.add_todo(description);
                let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
            }
            state.ui.input_mode = InputMode::Normal;
            state.ui.input_buffer.clear();
        }
        Action::MarkTodoDone => {
            let todo_id = state.selected_workspace()
                .and_then(|ws| {
                    ws.todos.iter()
                        .filter(|t| !t.is_archived())
                        .nth(state.ui.selected_todo_idx)
                        .map(|t| t.id)
                });

            if let Some(id) = todo_id {
                if let Some(ws) = state.data.workspaces.get_mut(state.ui.selected_workspace_idx) {
                    if let Some(todo) = ws.get_todo_mut(id) {
                        todo.mark_done();
                        let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
                    }
                }
            }
        }
        Action::RunSelectedTodo => {
            let selected_todo = state.selected_workspace()
                .and_then(|ws| {
                    ws.todos.iter()
                        .filter(|t| !t.is_archived())
                        .nth(state.ui.selected_todo_idx)
                        .map(|t| (t.id, t.description.clone(), t.is_pending(), t.is_queued()))
                });

            let (todo_id, description, is_pending, is_queued) = match selected_todo {
                Some(data) => data,
                None => return Ok(()),
            };

            if !is_pending && !is_queued {
                return Ok(())
            }

            let has_in_progress = state.data.workspaces.get(state.ui.selected_workspace_idx)
                .map(|ws| ws.has_in_progress_todo())
                .unwrap_or(false);

            let todo_count = state.selected_workspace()
                .map(|ws| ws.todos.iter().filter(|t| !t.is_archived()).count())
                .unwrap_or(0);

            if state.ui.todo_pane_mode == TodoPaneMode::Autorun && has_in_progress {
                if let Some(ws) = state.data.workspaces.get_mut(state.ui.selected_workspace_idx) {
                    if let Some(todo) = ws.get_todo_mut(todo_id) {
                        if todo.is_pending() {
                            todo.mark_queued();
                            let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
                        }
                    }
                }
                if state.ui.selected_todo_idx + 1 < todo_count {
                    state.ui.selected_todo_idx += 1;
                }
                return Ok(())
            }

            let current_workspace_id = state.data.workspaces.get(state.ui.selected_workspace_idx)
                .map(|ws| ws.id);

            let target_session_id = state.ui.active_session_id
                .filter(|id| state.data.idle_queue.contains(id))
                .or_else(|| {
                    current_workspace_id.and_then(|ws_id| {
                        state.data.sessions.get(&ws_id)
                            .and_then(|sessions| {
                                sessions.iter()
                                    .find(|s| {
                                        s.agent_type.is_agent() &&
                                        s.status == SessionStatus::Running &&
                                        state.data.idle_queue.contains(&s.id)
                                    })
                                    .map(|s| s.id)
                            })
                    })
                });

            if let Some(session_id) = target_session_id {
                if let Some(ws) = state.data.workspaces.get_mut(state.ui.selected_workspace_idx) {
                    if let Some(todo) = ws.get_todo_mut(todo_id) {
                        todo.assign_to(session_id);
                    }
                }
                state.data.idle_queue.retain(|&id| id != session_id);

                let text_bytes: Vec<u8> = description.bytes().collect();
                let _ = action_tx.send(Action::SendInput(session_id, text_bytes));
                let _ = action_tx.send(Action::SendInput(session_id, vec![b'\r']));
                let _ = persistence::save(&state.data.workspaces, &state.data.sessions);

                if state.ui.selected_todo_idx + 1 < todo_count {
                    state.ui.selected_todo_idx += 1;
                }
            } else if state.ui.todo_pane_mode == TodoPaneMode::Autorun {
                if let Some(ws) = state.data.workspaces.get_mut(state.ui.selected_workspace_idx) {
                    if let Some(todo) = ws.get_todo_mut(todo_id) {
                        if todo.is_pending() {
                            todo.mark_queued();
                            let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
                        }
                    }
                }
                if state.ui.selected_todo_idx + 1 < todo_count {
                    state.ui.selected_todo_idx += 1;
                }
            }
        }
        Action::ToggleTodoPaneMode => {
            state.ui.todo_pane_mode = state.ui.todo_pane_mode.toggle();
        }
        Action::InitiateDeleteTodo(id, desc) => {
            state.ui.pending_delete = Some(PendingDelete::Todo(id, desc));
        }
        Action::ConfirmDeleteTodo => {
            if let Some(PendingDelete::Todo(id, _)) = state.ui.pending_delete.take() {
                if let Some(ws) = state.data.workspaces.get_mut(state.ui.selected_workspace_idx) {
                    ws.remove_todo(id);
                    let filtered_count = match state.ui.selected_todos_tab {
                        TodosTab::Active => ws.todos.iter().filter(|t| !t.is_archived()).count(),
                        TodosTab::Archived => ws.todos.iter().filter(|t| t.is_archived()).count(),
                        TodosTab::Reports => ws.active_parallel_task().map(|t| t.attempts.len()).unwrap_or(0),
                    };
                    if filtered_count > 0 && state.ui.selected_todo_idx >= filtered_count {
                        state.ui.selected_todo_idx = filtered_count - 1;
                    } else if filtered_count == 0 {
                        state.ui.selected_todo_idx = 0;
                    }
                    let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
                }
            }
        }
        Action::DispatchTodoToSession(session_id, todo_id, description) => {
            if let Some(workspace_id) = state.workspace_id_for_session(session_id) {
                if let Some(ws) = state.get_workspace_mut(workspace_id) {
                    if let Some(todo) = ws.get_todo_mut(todo_id) {
                        todo.assign_to(session_id);
                    }
                }
                state.data.idle_queue.retain(|&id| id != session_id);
                let text_bytes: Vec<u8> = description.bytes().collect();
                let _ = action_tx.send(Action::SendInput(session_id, text_bytes));
                let _ = action_tx.send(Action::SendInput(session_id, vec![b'\r']));
                let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
            }
        }
        Action::MarkTodoReadyForReview(todo_id) => {
            for ws in state.data.workspaces.iter_mut() {
                if let Some(todo) = ws.get_todo_mut(todo_id) {
                    todo.mark_ready_for_review();
                    let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
                    break;
                }
            }
        }
        Action::AddSuggestedTodo(description) => {
            if let Some(ws) = state.data.workspaces.get_mut(state.ui.selected_workspace_idx) {
                ws.add_suggested_todo(description);
                let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
            }
        }
        Action::ApproveSuggestedTodo(todo_id) => {
            if let Some(ws) = state.data.workspaces.get_mut(state.ui.selected_workspace_idx) {
                if let Some(todo) = ws.get_todo_mut(todo_id) {
                    todo.approve();
                    let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
                }
            }
        }
        Action::ApproveAllSuggestedTodos => {
            if let Some(ws) = state.data.workspaces.get_mut(state.ui.selected_workspace_idx) {
                for todo in ws.todos.iter_mut() {
                    if todo.is_suggested() {
                        todo.approve();
                    }
                }
                let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
            }
        }
        Action::ArchiveTodo(todo_id) => {
            if let Some(ws) = state.data.workspaces.get_mut(state.ui.selected_workspace_idx) {
                if let Some(todo) = ws.get_todo_mut(todo_id) {
                    todo.archive();
                    let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
                }
            }
        }
        Action::ToggleTodosTab => {
            state.ui.selected_todos_tab = state.ui.selected_todos_tab.toggle();
            state.ui.selected_todo_idx = 0;
        }
        Action::ActivateUtility => {
            // Handle ToggleBanner specially - just toggle it without loading content
            if state.ui.selected_utility == UtilityItem::ToggleBanner {
                state.ui.banner_visible = !state.ui.banner_visible;
                // Resize PTYs since pane height changed
                crate::app::pty_ops::resize_ptys_to_panes(state);
                let config = crate::persistence::GlobalConfig {
                    banner_visible: state.ui.banner_visible,
                    left_panel_ratio: state.ui.left_panel_ratio,
                    workspace_ratio: state.ui.workspace_ratio,
                    sessions_ratio: state.ui.sessions_ratio,
                    todos_ratio: state.ui.todos_ratio,
                    output_split_ratio: state.ui.output_split_ratio,
                };
                let _ = crate::persistence::save_config(&config);
            }
            // Special handling for SuggestTodos - trigger analyzer
            else if state.ui.selected_utility == UtilityItem::SuggestTodos {
                let idle_agent = state.selected_workspace().and_then(|ws| {
                    state.data.sessions.get(&ws.id).and_then(|sessions| {
                        sessions.iter()
                            .find(|s| s.agent_type.is_agent() && state.data.idle_queue.contains(&s.id))
                            .map(|s| s.id)
                    })
                });

                if let Some(session_id) = idle_agent {
                    state.ui.analyzer_session_id = Some(session_id);
                    let prompt = r##"Analyze this codebase and suggest 3-5 potential improvements, new features, or refactoring opportunities.

For each suggestion, output it on its own line in this exact format:
TODO: [DIFFICULTY] [IMPORTANCE] <description>

Where:
- DIFFICULTY is one of: EASY, MED, HARD
- IMPORTANCE is one of: LOW, MED, HIGH, CRITICAL

Examples:
TODO: [EASY] [HIGH] Add input validation for user email fields
TODO: [MED] [CRITICAL] Implement rate limiting on API endpoints
TODO: [HARD] [MED] Refactor database layer to use connection pooling

Focus on practical, actionable items. Be specific about what needs to be done."##;

                    let text_bytes: Vec<u8> = prompt.bytes().collect();
                    let _ = action_tx.send(Action::SendInput(session_id, text_bytes));
                    let _ = action_tx.send(Action::SendInput(session_id, vec![b'\r']));

                    state.data.idle_queue.retain(|&id| id != session_id);
                    state.ui.active_session_id = Some(session_id);
                    state.ui.focus = FocusPanel::OutputPane;
                } else {
                    load_utility_content(state, action_tx);
                    state.ui.active_session_id = None;
                }
            } else {
                load_utility_content(state, action_tx);
                state.ui.active_session_id = None;
            }
        }
        _ => {}
    }
    Ok(())
}
