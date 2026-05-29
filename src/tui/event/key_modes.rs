use crate::app::{Action, AppState, ConfigTab, InputMode};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::shortcuts::agent_shortcut;

pub(super) fn handle_input_mode_key(key: &KeyEvent, state: &AppState) -> Option<Action> {
    let action = match state.ui.input_mode {
        InputMode::SelectWorkspaceAction => match key.code {
            KeyCode::Esc => Action::ExitMode,
            KeyCode::Char('j') | KeyCode::Down => Action::NextWorkspaceChoice,
            KeyCode::Char('k') | KeyCode::Up => Action::PrevWorkspaceChoice,
            KeyCode::Enter => Action::ConfirmWorkspaceChoice,
            _ => Action::Tick,
        },
        InputMode::EnterWorkspaceName => match key.code {
            KeyCode::Esc => Action::ExitMode,
            KeyCode::Enter => {
                let name = state.ui.input_buffer.clone();
                if name.is_empty() {
                    Action::Tick
                } else {
                    Action::CreateNewWorkspace(name)
                }
            }
            KeyCode::Backspace => Action::InputBackspace,
            KeyCode::Char(c) => Action::InputChar(c),
            _ => Action::Tick,
        },
        InputMode::CreateWorkspace => {
            if state.ui.workspace_create_mode {
                match key.code {
                    KeyCode::Esc => Action::ExitMode,
                    KeyCode::Char('j') | KeyCode::Down => Action::FileBrowserDown,
                    KeyCode::Char('k') | KeyCode::Up => Action::FileBrowserUp,
                    KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter => {
                        Action::FileBrowserEnter
                    }
                    KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace => {
                        Action::FileBrowserBack
                    }
                    KeyCode::Char(' ') | KeyCode::Tab => Action::EnterWorkspaceNameMode,
                    _ => Action::Tick,
                }
            } else {
                match key.code {
                    KeyCode::Esc => Action::ExitMode,
                    KeyCode::Down => Action::FileBrowserDown,
                    KeyCode::Up => Action::FileBrowserUp,
                    KeyCode::Right | KeyCode::Enter => Action::FileBrowserEnter,
                    KeyCode::Left => Action::FileBrowserBack,
                    KeyCode::Char(' ') | KeyCode::Tab => Action::FileBrowserSelect,
                    KeyCode::Backspace => {
                        if state.ui.file_browser.query.is_empty() {
                            Action::FileBrowserBack
                        } else {
                            Action::InputBackspace
                        }
                    }
                    KeyCode::Char(c)
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        Action::InputChar(c)
                    }
                    _ => Action::Tick,
                }
            }
        }
        InputMode::CreateSession => {
            if let Some((agent_type, dangerously_skip_permissions, with_worktree)) =
                agent_shortcut(key, &state.system.user_config.agents)
            {
                Action::CreateSession(agent_type, dangerously_skip_permissions, with_worktree)
            } else {
                match key.code {
                    KeyCode::Esc => Action::ExitMode,
                    KeyCode::Char('t') => Action::CreateTerminal,
                    _ => Action::Tick,
                }
            }
        }
        InputMode::SetStartCommand => match key.code {
            KeyCode::Esc => Action::ExitMode,
            KeyCode::Enter => {
                if let Some(session_id) = state.ui.editing_session_id {
                    let cmd = state.ui.input_buffer.clone();
                    Action::SetStartCommand(session_id, cmd)
                } else {
                    Action::ExitMode
                }
            }
            KeyCode::Backspace => Action::InputBackspace,
            KeyCode::Char(c) => Action::InputChar(c),
            _ => Action::Tick,
        },
        InputMode::CreateTodo => match key.code {
            KeyCode::Esc => Action::ExitMode,
            KeyCode::Enter => {
                let desc = state.ui.input_buffer.clone();
                if desc.is_empty() {
                    Action::ExitMode
                } else {
                    Action::CreateTodo(desc)
                }
            }
            KeyCode::Backspace => Action::InputBackspace,
            KeyCode::Char(c) => Action::InputChar(c),
            _ => Action::Tick,
        },
        InputMode::CreateParallelTask => match key.code {
            KeyCode::Esc => Action::ExitMode,
            KeyCode::Tab => Action::NextParallelAgent,
            KeyCode::BackTab => Action::PrevParallelAgent,
            KeyCode::Char('x') => Action::ToggleParallelAgent(state.ui.parallel_task.agent_idx),
            KeyCode::Enter => Action::StartParallelTask,
            KeyCode::Backspace => Action::InputBackspace,
            KeyCode::Char(c) => Action::InputChar(c),
            _ => Action::Tick,
        },
        InputMode::ConfirmMergeWorktree => match key.code {
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => Action::CancelMerge,
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                Action::ConfirmMergeWithCommit
            }
            _ => Action::Tick,
        },
        InputMode::ConfirmParallelMerge => match key.code {
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => Action::CancelParallelMerge,
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                Action::ConfirmParallelMerge
            }
            _ => Action::Tick,
        },
        InputMode::CommandPalette => match key.code {
            KeyCode::Esc => Action::ExitCommandPalette,
            KeyCode::Enter => Action::CommandPaletteExecute,
            KeyCode::Down => Action::CommandPaletteDown,
            KeyCode::Up => Action::CommandPaletteUp,
            KeyCode::Backspace => Action::CommandPaletteBackspace,
            KeyCode::Char(c) => Action::CommandPaletteInput(c),
            _ => Action::Tick,
        },
        InputMode::ConfigWindow => {
            if state.ui.config.rebinding {
                Action::ConfigRebindKey(*key)
            } else if state.ui.config.editing {
                match key.code {
                    KeyCode::Esc => Action::ConfigCancelEdit,
                    KeyCode::Enter => Action::ConfigFinishEdit,
                    KeyCode::Backspace => Action::ConfigInputBackspace,
                    KeyCode::Char(c) => Action::ConfigInputChar(c),
                    _ => Action::Tick,
                }
            } else {
                match key.code {
                    KeyCode::Esc => Action::ExitConfigWindow,
                    KeyCode::Char('1') => Action::ConfigSwitchTab(ConfigTab::QuickRef),
                    KeyCode::Char('2') => Action::ConfigSwitchTab(ConfigTab::Agents),
                    KeyCode::Char('3') => Action::ConfigSwitchTab(ConfigTab::Hotkeys),
                    KeyCode::Char('4') => Action::ConfigSwitchTab(ConfigTab::Scrollback),
                    KeyCode::Tab => {
                        let next = match state.ui.config.tab {
                            ConfigTab::QuickRef => ConfigTab::Agents,
                            ConfigTab::Agents => ConfigTab::Hotkeys,
                            ConfigTab::Hotkeys => ConfigTab::Scrollback,
                            ConfigTab::Scrollback => ConfigTab::QuickRef,
                        };
                        Action::ConfigSwitchTab(next)
                    }
                    KeyCode::Char('j') | KeyCode::Down => Action::ConfigMoveDown,
                    KeyCode::Char('k') | KeyCode::Up => Action::ConfigMoveUp,
                    KeyCode::Char('h') | KeyCode::Left => Action::ConfigMoveLeft,
                    KeyCode::Char('l') | KeyCode::Right => Action::ConfigMoveRight,
                    KeyCode::Enter => Action::ConfigStartEdit,
                    KeyCode::Char('a') => Action::ConfigAddAgent,
                    KeyCode::Char('d') => Action::ConfigDeleteAgent,
                    KeyCode::Char('J') => Action::ConfigReorderDown,
                    KeyCode::Char('K') => Action::ConfigReorderUp,
                    KeyCode::Char('r') => Action::ConfigResetDefault,
                    _ => Action::Tick,
                }
            }
        }
        InputMode::Normal => return None,
    };

    Some(action)
}
