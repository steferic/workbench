use crate::app::{Action, AppState, InputMode};
use crate::persistence;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers};
use tui_textarea::{Input, Key};

pub fn handle_input_action(state: &mut AppState, action: Action) -> Result<()> {
    match action {
        Action::EnterHelpMode => {
            state.ui.input_mode = InputMode::Help;
        }
        Action::EnterWorkspaceActionMode => {
            state.ui.input_mode = InputMode::SelectWorkspaceAction;
            state.ui.selected_workspace_action = crate::app::WorkspaceAction::default();
        }
        Action::EnterWorkspaceNameMode => {
            state.ui.input_mode = InputMode::EnterWorkspaceName;
            state.ui.input_buffer.clear();
        }
        Action::EnterCreateSessionMode => {
            if state.selected_workspace().is_some() {
                state.ui.input_mode = InputMode::CreateSession;
            }
        }
        Action::ExitMode => {
            state.ui.input_mode = InputMode::Normal;
            state.ui.input_buffer.clear();
            state.ui.file_browser_query.clear();
            state.ui.editing_session_id = None;
        }
        Action::EnterSetStartCommandMode => {
            let session_info = state.selected_session()
                .filter(|s| s.agent_type.is_terminal())
                .map(|s| (s.id, s.start_command.clone()));

            if let Some((session_id, existing_cmd)) = session_info {
                state.ui.editing_session_id = Some(session_id);
                state.ui.input_buffer = existing_cmd.unwrap_or_default();
                state.ui.input_mode = InputMode::SetStartCommand;
            }
        }
        Action::SetStartCommand(session_id, command) => {
            if let Some(session) = state.get_session_mut(session_id) {
                session.start_command = if command.is_empty() {
                    None
                } else {
                    Some(command)
                };
            }
            state.ui.input_mode = InputMode::Normal;
            state.ui.input_buffer.clear();
            state.ui.editing_session_id = None;
            let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
        }
        Action::InputChar(c) => {
            // Handle input based on current mode
            if state.ui.input_mode == InputMode::CreateWorkspace && !state.ui.workspace_create_mode {
                state.ui.file_browser_query.push(c);
                state.apply_file_browser_filter();
            } else if state.ui.input_mode == InputMode::CreateParallelTask {
                state.ui.parallel_task_prompt.push(c);
            } else {
                state.ui.input_buffer.push(c);
            }
        }
        Action::InputBackspace => {
            // Handle backspace based on current mode
            if state.ui.input_mode == InputMode::CreateWorkspace && !state.ui.workspace_create_mode {
                state.ui.file_browser_query.pop();
                state.apply_file_browser_filter();
            } else if state.ui.input_mode == InputMode::CreateParallelTask {
                state.ui.parallel_task_prompt.pop();
            } else {
                state.ui.input_buffer.pop();
            }
        }
        Action::NotepadInput(key) => {
            let tui_key = match key.code {
                KeyCode::Char(c) => Key::Char(c),
                KeyCode::Backspace => Key::Backspace,
                KeyCode::Enter => Key::Enter,
                KeyCode::Left => Key::Left,
                KeyCode::Right => Key::Right,
                KeyCode::Up => Key::Up,
                KeyCode::Down => Key::Down,
                KeyCode::Tab => Key::Tab,
                KeyCode::Delete => Key::Delete,
                KeyCode::Home => Key::Home,
                KeyCode::End => Key::End,
                KeyCode::PageUp => Key::PageUp,
                KeyCode::PageDown => Key::PageDown,
                KeyCode::Esc => Key::Esc,
                KeyCode::F(n) => Key::F(n),
                _ => Key::Null,
            };

            let input = Input {
                key: tui_key,
                ctrl: key.modifiers.contains(KeyModifiers::CONTROL),
                alt: key.modifiers.contains(KeyModifiers::ALT),
                shift: key.modifiers.contains(KeyModifiers::SHIFT),
            };

            if let Some(textarea) = state.current_notepad() {
                textarea.input(input);
            }
            let notepad_contents = state.notepad_content_for_persistence();
            let _ = persistence::save_with_notepad(&state.data.workspaces, &state.data.sessions, &notepad_contents);
        }
        Action::FileBrowserUp => {
            if state.ui.file_browser_selected > 0 {
                state.ui.file_browser_selected -= 1;
                if state.ui.file_browser_selected < state.ui.file_browser_scroll {
                    state.ui.file_browser_scroll = state.ui.file_browser_selected;
                }
            }
        }
        Action::FileBrowserDown => {
            if state.ui.file_browser_selected < state.ui.file_browser_entries.len().saturating_sub(1) {
                state.ui.file_browser_selected += 1;
                let visible_height = 15;
                if state.ui.file_browser_selected >= state.ui.file_browser_scroll + visible_height {
                    state.ui.file_browser_scroll = state.ui.file_browser_selected - visible_height + 1;
                }
            }
        }
        Action::FileBrowserEnter => {
            state.file_browser_enter_selected();
        }
        Action::FileBrowserBack => {
            state.file_browser_go_up();
        }
        Action::FileBrowserSelect => {
            let path = if let Some(selected) = state.ui.file_browser_entries.get(state.ui.file_browser_selected) {
                selected.clone()
            } else {
                state.ui.file_browser_path.clone()
            };
            if path.exists() && path.is_dir() {
                let workspace = crate::models::Workspace::from_path(path);
                state.add_workspace(workspace);
                state.ui.file_browser_query.clear();
                state.ui.input_mode = InputMode::Normal;
                let _ = persistence::save(&state.data.workspaces, &state.data.sessions);
            }
        }
        Action::EnterCreateTodoMode => {
            state.ui.input_mode = InputMode::CreateTodo;
            state.ui.input_buffer.clear();
        }
        Action::EnterParallelTaskMode => {
            // Only enter if we have a workspace selected
            if state.selected_workspace().is_some() {
                state.ui.input_mode = InputMode::CreateParallelTask;
                state.ui.parallel_task_prompt.clear();
                state.ui.parallel_task_agent_idx = 0;
                // Pre-select agents that have running sessions in the workspace
                let ws_id = state.selected_workspace().map(|w| w.id);
                if let Some(workspace_id) = ws_id {
                    let running_agents: Vec<_> = state.data.sessions.get(&workspace_id)
                        .map(|sessions| {
                            sessions.iter()
                                .filter(|s| s.agent_type.is_agent() && s.status == crate::models::SessionStatus::Running)
                                .map(|s| s.agent_type.clone())
                                .collect()
                        })
                        .unwrap_or_default();

                    // Update selection based on running agents
                    for (agent_type, selected) in state.ui.parallel_task_agents.iter_mut() {
                        *selected = running_agents.contains(agent_type);
                    }
                }
            }
        }
        Action::NextParallelAgent => {
            let agent_count = state.ui.parallel_task_agents.len();
            // Total items = agents + 1 (report checkbox)
            let total_items = agent_count + 1;
            if total_items > 0 {
                state.ui.parallel_task_agent_idx = (state.ui.parallel_task_agent_idx + 1) % total_items;
            }
        }
        Action::PrevParallelAgent => {
            let agent_count = state.ui.parallel_task_agents.len();
            // Total items = agents + 1 (report checkbox)
            let total_items = agent_count + 1;
            if total_items > 0 {
                if state.ui.parallel_task_agent_idx == 0 {
                    state.ui.parallel_task_agent_idx = total_items - 1;
                } else {
                    state.ui.parallel_task_agent_idx -= 1;
                }
            }
        }
        Action::ToggleParallelAgent(idx) => {
            let agent_count = state.ui.parallel_task_agents.len();
            if idx == agent_count {
                // Toggle the report checkbox
                state.ui.parallel_task_request_report = !state.ui.parallel_task_request_report;
            } else if let Some((_, selected)) = state.ui.parallel_task_agents.get_mut(idx) {
                *selected = !*selected;
            }
        }
        _ => {}
    }
    Ok(())
}
