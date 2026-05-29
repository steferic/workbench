use crate::app::{Action, AppState, InputMode};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers};
use tui_textarea::{Input, Key};

use super::{save_state, save_state_with_notepad};

pub fn handle_input_action(state: &mut AppState, action: Action) -> Result<()> {
    match action {
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
            state.ui.file_browser.query.clear();
            state.ui.editing_session_id = None;
        }
        Action::EnterSetStartCommandMode => {
            let session_info = state
                .selected_session()
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
            save_state(state, "failed to save start command");
        }
        Action::InputChar(c) => {
            // Handle input based on current mode
            if state.ui.input_mode == InputMode::CreateWorkspace && !state.ui.workspace_create_mode
            {
                state.ui.file_browser.query.push(c);
                state.apply_file_browser_filter();
            } else if state.ui.input_mode == InputMode::CreateParallelTask {
                state.ui.parallel_task.prompt.push(c);
            } else {
                state.ui.input_buffer.push(c);
            }
        }
        Action::InputBackspace => {
            // Handle backspace based on current mode
            if state.ui.input_mode == InputMode::CreateWorkspace && !state.ui.workspace_create_mode
            {
                state.ui.file_browser.query.pop();
                state.apply_file_browser_filter();
            } else if state.ui.input_mode == InputMode::CreateParallelTask {
                state.ui.parallel_task.prompt.pop();
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
            save_state_with_notepad(state, "failed to save notepad input");
        }
        Action::FileBrowserUp => {
            if state.ui.file_browser.selected > 0 {
                state.ui.file_browser.selected -= 1;
                if state.ui.file_browser.selected < state.ui.file_browser.scroll {
                    state.ui.file_browser.scroll = state.ui.file_browser.selected;
                }
            }
        }
        Action::FileBrowserDown => {
            if state.ui.file_browser.selected
                < state.ui.file_browser.entries.len().saturating_sub(1)
            {
                state.ui.file_browser.selected += 1;
                let visible_height = 15;
                if state.ui.file_browser.selected >= state.ui.file_browser.scroll + visible_height {
                    state.ui.file_browser.scroll =
                        state.ui.file_browser.selected - visible_height + 1;
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
            let path = if let Some(selected) = state
                .ui
                .file_browser.entries
                .get(state.ui.file_browser.selected)
            {
                selected.clone()
            } else {
                state.ui.file_browser.path.clone()
            };
            if path.exists() && path.is_dir() {
                let workspace = crate::models::Workspace::from_path(path);
                state.add_workspace(workspace);
                state.ui.file_browser.query.clear();
                state.ui.input_mode = InputMode::Normal;
                save_state(state, "failed to save workspace selection");
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
                state.ui.parallel_task.prompt.clear();
                state.ui.parallel_task.agent_idx = 0;
                // Pre-select agents that have running sessions in the workspace
                let ws_id = state.selected_workspace().map(|w| w.id);
                if let Some(workspace_id) = ws_id {
                    let running_agents: Vec<_> = state
                        .data
                        .sessions
                        .get(&workspace_id)
                        .map(|sessions| {
                            sessions
                                .iter()
                                .filter(|s| {
                                    s.agent_type.is_agent()
                                        && s.status == crate::models::SessionStatus::Running
                                })
                                .map(|s| s.agent_type.clone())
                                .collect()
                        })
                        .unwrap_or_default();

                    // Update selection based on running agents
                    for (agent_type, selected) in state.ui.parallel_task.agents.iter_mut() {
                        *selected = running_agents.contains(agent_type);
                    }
                }
            }
        }
        Action::NextParallelAgent => {
            let agent_count = state.ui.parallel_task.agents.len();
            // Total items = agents + 2 (dangerous mode + report checkboxes)
            let total_items = agent_count + 2;
            if total_items > 0 {
                state.ui.parallel_task.agent_idx =
                    (state.ui.parallel_task.agent_idx + 1) % total_items;
            }
        }
        Action::PrevParallelAgent => {
            let agent_count = state.ui.parallel_task.agents.len();
            // Total items = agents + 2 (dangerous mode + report checkboxes)
            let total_items = agent_count + 2;
            if total_items > 0 {
                if state.ui.parallel_task.agent_idx == 0 {
                    state.ui.parallel_task.agent_idx = total_items - 1;
                } else {
                    state.ui.parallel_task.agent_idx -= 1;
                }
            }
        }
        Action::ToggleParallelAgent(idx) => {
            let agent_count = state.ui.parallel_task.agents.len();
            if idx == agent_count {
                // First extra checkbox: dangerous mode
                state.ui.parallel_task.dangerous_mode = !state.ui.parallel_task.dangerous_mode;
            } else if idx == agent_count + 1 {
                // Second extra checkbox: request report
                state.ui.parallel_task.request_report = !state.ui.parallel_task.request_report;
            } else if let Some((_, selected)) = state.ui.parallel_task.agents.get_mut(idx) {
                *selected = !*selected;
            }
        }
        Action::EnterCommandPalette => {
            state.ui.input_mode = InputMode::CommandPalette;
            state.ui.palette.query.clear();
            state.ui.palette.selected = 0;
        }
        Action::ExitCommandPalette => {
            state.ui.input_mode = InputMode::Normal;
            state.ui.palette.query.clear();
            state.ui.palette.selected = 0;
        }
        Action::CommandPaletteInput(c) => {
            state.ui.palette.query.push(c);
            state.ui.palette.selected = 0;
        }
        Action::CommandPaletteBackspace => {
            state.ui.palette.query.pop();
            state.ui.palette.selected = 0;
        }
        Action::CommandPaletteDown => {
            let count =
                crate::tui::components::command_palette::filtered_entries(&state.ui.palette.query)
                    .len();
            if count > 0 && state.ui.palette.selected + 1 < count {
                state.ui.palette.selected += 1;
            }
        }
        Action::CommandPaletteUp => {
            if state.ui.palette.selected > 0 {
                state.ui.palette.selected -= 1;
            }
        }
        Action::CommandPaletteExecute => {
            let entries =
                crate::tui::components::command_palette::filtered_entries(&state.ui.palette.query);
            if let Some(entry) = entries.into_iter().nth(state.ui.palette.selected) {
                state.ui.input_mode = InputMode::Normal;
                state.ui.palette.query.clear();
                state.ui.palette.selected = 0;
                state.ui.palette.pending_action = Some(entry.action);
            }
        }
        Action::InitiateQuit => {
            state.ui.pending_quit = true;
        }
        Action::CancelQuit => {
            state.ui.pending_quit = false;
        }
        // ConfirmQuit is handled in the main handler as it triggers actual quit
        _ => {}
    }
    Ok(())
}
