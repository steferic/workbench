use crate::app::{Action, AppState, ConfigTab, FocusPanel, InputMode, PendingDelete, TodosTab};
use crate::models::AgentType;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::EventHandler;

impl EventHandler {
    /// Check for global keybindings that work in any panel (driven by UserConfig)
    fn check_global_keys(key: &KeyEvent, user_config: &crate::config::user_config::UserConfig) -> Option<Action> {
        use crate::config::keybindings::KeyCombo;

        let pressed = KeyCombo::new(key.code, key.modifiers);
        let pressed_str = pressed.display();

        for (action_name, key_str) in &user_config.global_hotkeys {
            if key_str == &pressed_str {
                return match action_name.as_str() {
                    "CycleNextWorkspace" => Some(Action::CycleNextWorkspace),
                    "CycleNextSession" => Some(Action::CycleNextSession),
                    "InitiateQuit" => Some(Action::InitiateQuit),
                    "EnterHelpMode" => Some(Action::EnterConfigWindow),
                    "ToggleDebugOverlay" => Some(Action::ToggleDebugOverlay),
                    "EnterConfigWindow" => Some(Action::EnterConfigWindow),
                    "TestToast" => Some(Action::TestToast),
                    _ => None,
                };
            }
        }
        None
    }

    pub(super) fn handle_key_event(&self, key: KeyEvent, state: &AppState) -> Action {
        // Handle input mode first
        match state.ui.input_mode {
            InputMode::SelectWorkspaceAction => {
                return match key.code {
                    KeyCode::Esc => Action::ExitMode,
                    KeyCode::Char('j') | KeyCode::Down => Action::NextWorkspaceChoice,
                    KeyCode::Char('k') | KeyCode::Up => Action::PrevWorkspaceChoice,
                    KeyCode::Enter => Action::ConfirmWorkspaceChoice,
                    _ => Action::Tick,
                };
            }
            InputMode::EnterWorkspaceName => {
                return match key.code {
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
                };
            }
            InputMode::CreateWorkspace => {
                if state.ui.workspace_create_mode {
                    return match key.code {
                        KeyCode::Esc => Action::ExitMode,
                        KeyCode::Char('j') | KeyCode::Down => Action::FileBrowserDown,
                        KeyCode::Char('k') | KeyCode::Up => Action::FileBrowserUp,
                        KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter => Action::FileBrowserEnter,
                        KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace => Action::FileBrowserBack,
                        KeyCode::Char(' ') | KeyCode::Tab => Action::EnterWorkspaceNameMode,
                        _ => Action::Tick,
                    };
                }

                return match key.code {
                    KeyCode::Esc => Action::ExitMode,
                    KeyCode::Down => Action::FileBrowserDown,
                    KeyCode::Up => Action::FileBrowserUp,
                    KeyCode::Right | KeyCode::Enter => Action::FileBrowserEnter,
                    KeyCode::Left => Action::FileBrowserBack,
                    KeyCode::Char(' ') | KeyCode::Tab => Action::FileBrowserSelect,
                    KeyCode::Backspace => {
                        if state.ui.file_browser_query.is_empty() {
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
                };
            }
            InputMode::CreateSession => {
                if let Some((agent_type, dangerously_skip_permissions, with_worktree)) = Self::agent_shortcut(&key, &state.system.user_config.agents)
                {
                    return Action::CreateSession(agent_type, dangerously_skip_permissions, with_worktree);
                }
                return match key.code {
                    KeyCode::Esc => Action::ExitMode,
                    KeyCode::Char('t') => Action::CreateTerminal,
                    _ => Action::Tick,
                };
            }
            InputMode::SetStartCommand => {
                return match key.code {
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
                };
            }
            InputMode::CreateTodo => {
                return match key.code {
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
                };
            }
            InputMode::CreateParallelTask => {
                return match key.code {
                    KeyCode::Esc => Action::ExitMode,
                    KeyCode::Tab => Action::NextParallelAgent,
                    KeyCode::BackTab => Action::PrevParallelAgent,
                    KeyCode::Char('x') => {
                        Action::ToggleParallelAgent(state.ui.parallel_task_agent_idx)
                    }
                    KeyCode::Enter => Action::StartParallelTask,
                    KeyCode::Backspace => Action::InputBackspace,
                    KeyCode::Char(c) => Action::InputChar(c),
                    _ => Action::Tick,
                };
            }
            InputMode::ConfirmMergeWorktree => {
                return match key.code {
                    KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => Action::CancelMerge,
                    KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => Action::ConfirmMergeWithCommit,
                    _ => Action::Tick,
                };
            }
            InputMode::ConfirmParallelMerge => {
                return match key.code {
                    KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => Action::CancelParallelMerge,
                    KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => Action::ConfirmParallelMerge,
                    _ => Action::Tick,
                };
            }
            InputMode::CommandPalette => {
                return match key.code {
                    KeyCode::Esc => Action::ExitCommandPalette,
                    KeyCode::Enter => Action::CommandPaletteExecute,
                    KeyCode::Down => Action::CommandPaletteDown,
                    KeyCode::Up => Action::CommandPaletteUp,
                    KeyCode::Backspace => Action::CommandPaletteBackspace,
                    KeyCode::Char(c) => Action::CommandPaletteInput(c),
                    _ => Action::Tick,
                };
            }
            InputMode::ConfigWindow => {
                // If rebinding a hotkey, capture any key as the new binding
                if state.ui.config_rebinding {
                    return Action::ConfigRebindKey(key);
                }
                // If editing a field, handle text input
                if state.ui.config_editing {
                    return match key.code {
                        KeyCode::Esc => Action::ConfigCancelEdit,
                        KeyCode::Enter => Action::ConfigFinishEdit,
                        KeyCode::Backspace => Action::ConfigInputBackspace,
                        KeyCode::Char(c) => Action::ConfigInputChar(c),
                        _ => Action::Tick,
                    };
                }
                // Normal config navigation
                return match key.code {
                    KeyCode::Esc => Action::ExitConfigWindow,
                    KeyCode::Char('1') => Action::ConfigSwitchTab(ConfigTab::QuickRef),
                    KeyCode::Char('2') => Action::ConfigSwitchTab(ConfigTab::Agents),
                    KeyCode::Char('3') => Action::ConfigSwitchTab(ConfigTab::Hotkeys),
                    KeyCode::Char('4') => Action::ConfigSwitchTab(ConfigTab::Scrollback),
                    KeyCode::Tab => {
                        let next = match state.ui.config_tab {
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
                };
            }
            InputMode::Normal => {}
        }

        // Handle pending delete confirmation
        if state.ui.pending_delete.is_some() {
            return match key.code {
                KeyCode::Char('d') => {
                    match &state.ui.pending_delete {
                        Some(PendingDelete::Session(_, _)) => Action::ConfirmDeleteSession,
                        Some(PendingDelete::Workspace(_, _)) => Action::ConfirmDeleteWorkspace,
                        Some(PendingDelete::Todo(_, _)) => Action::ConfirmDeleteTodo,
                        None => Action::Tick,
                    }
                }
                KeyCode::Esc => Action::CancelPendingDelete,
                _ => Action::CancelPendingDelete,
            };
        }

        // Handle pending quit confirmation
        if state.ui.pending_quit {
            return match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('y') | KeyCode::Char('Y') => Action::ConfirmQuit,
                _ => Action::CancelQuit,
            };
        }

        // Global Ctrl+P - command palette
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('p') {
            return Action::EnterCommandPalette;
        }

        // Global window navigation with Shift+Left/Right arrows
        if key.modifiers.contains(KeyModifiers::SHIFT) {
            match key.code {
                KeyCode::Left => return Action::FocusLeft,
                KeyCode::Right => return Action::FocusRight,
                _ => {}
            }
        }

        // Normal mode key handling based on focused panel
        match state.ui.focus {
            FocusPanel::WorkspaceList => self.handle_workspace_list_keys(key, state),
            FocusPanel::SessionList => self.handle_session_list_keys(key, state),
            FocusPanel::TodosPane => self.handle_todos_pane_keys(key, state),
            FocusPanel::UtilitiesPane => self.handle_utilities_pane_keys(key, state),
            FocusPanel::OutputPane => self.handle_output_pane_keys(key, state),
            FocusPanel::PinnedTerminalPane(idx) => self.handle_pinned_terminal_keys(key, state, idx),
        }
    }

    fn handle_workspace_list_keys(&self, key: KeyEvent, state: &AppState) -> Action {
        if let Some(action) = Self::check_global_keys(&key, &state.system.user_config) {
            return action;
        }

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => Action::MoveDown,
            KeyCode::Char('k') | KeyCode::Up => Action::MoveUp,
            KeyCode::Char('l') => Action::FocusRight,
            KeyCode::Char('n') => Action::EnterWorkspaceActionMode,
            KeyCode::Char('w') => Action::ToggleWorkspaceStatus,
            KeyCode::Enter => Action::FocusRight,
            KeyCode::Char('d') => {
                if let Some(workspace) = state.selected_workspace() {
                    Action::InitiateDeleteWorkspace(workspace.id, workspace.name.clone())
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('h') => Action::EnterConfigWindow,
            KeyCode::Char('?') => Action::EnterConfigWindow,
            KeyCode::Char('q') | KeyCode::Esc => Action::InitiateQuit,
            _ => Action::Tick,
        }
    }

    fn handle_session_list_keys(&self, key: KeyEvent, state: &AppState) -> Action {
        if let Some(action) = Self::check_global_keys(&key, &state.system.user_config) {
            return action;
        }

        if let Some((agent_type, dangerously_skip_permissions, with_worktree)) = Self::agent_shortcut(&key, &state.system.user_config.agents) {
            return Action::CreateSession(agent_type, dangerously_skip_permissions, with_worktree);
        }

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => Action::MoveDown,
            KeyCode::Char('k') | KeyCode::Up => Action::MoveUp,
            KeyCode::Char('l') => Action::FocusRight,
            KeyCode::Char('n') => Action::EnterCreateSessionMode,
            KeyCode::Enter => {
                if let Some(session) = state.selected_session() {
                    if matches!(
                        session.status,
                        crate::models::SessionStatus::Stopped | crate::models::SessionStatus::Errored
                    ) {
                        Action::RestartSession(session.id)
                    } else {
                        Action::ActivateSession(session.id)
                    }
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('r') => {
                if let Some(session) = state.selected_session() {
                    if matches!(
                        session.status,
                        crate::models::SessionStatus::Stopped | crate::models::SessionStatus::Errored
                    ) {
                        Action::RestartSession(session.id)
                    } else {
                        Action::Tick
                    }
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('s') => {
                if let Some(session) = state.selected_session() {
                    Action::StopSession(session.id)
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('x') => {
                if let Some(session) = state.selected_session() {
                    Action::KillSession(session.id)
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('d') => {
                if let Some(session) = state.selected_session() {
                    Action::InitiateDeleteSession(session.id, session.display_name())
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('t') => Action::CreateTerminal,
            KeyCode::Char('c') => {
                if let Some(session) = state.selected_session() {
                    if session.agent_type.is_terminal() {
                        Action::EnterSetStartCommandMode
                    } else {
                        Action::Tick
                    }
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('p') => {
                if let Some(session) = state.selected_session() {
                    if state.pinned_terminal_ids().contains(&session.id) {
                        Action::UnpinSession(session.id)
                    } else {
                        Action::PinSession(session.id)
                    }
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('u') => {
                if let Some(session) = state.selected_session() {
                    if state.pinned_terminal_ids().contains(&session.id) {
                        Action::UnpinSession(session.id)
                    } else {
                        Action::Tick
                    }
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('\\') | KeyCode::Char('/') => Action::ToggleSplitView,
            KeyCode::Char('P') => Action::EnterParallelTaskMode,
            KeyCode::Char('X') => {
                if let Some(session) = state.selected_session() {
                    if let Some(task_id) = state.selected_workspace()
                        .and_then(|ws| {
                            ws.parallel_tasks.iter()
                                .find(|t| t.attempts.iter().any(|a| a.session_id == session.id))
                                .map(|t| t.id)
                        })
                    {
                        Action::CancelParallelTask(task_id)
                    } else {
                        Action::Tick
                    }
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('m') => {
                if let Some(session) = state.selected_session() {
                    if session.has_worktree() {
                        Action::MergeSessionWorktree(session.id)
                    } else {
                        Action::Tick
                    }
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('w') => {
                if let Some(session) = state.selected_session() {
                    if session.has_worktree() {
                        let is_active = state.selected_workspace()
                            .and_then(|ws| ws.active_worktree_session_id)
                            .map(|id| id == session.id)
                            .unwrap_or(false);

                        if is_active {
                            Action::SwitchToWorktree(None)
                        } else {
                            Action::SwitchToWorktree(Some(session.id))
                        }
                    } else {
                        Action::Tick
                    }
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('h') => Action::EnterConfigWindow,
            KeyCode::Char('?') => Action::EnterConfigWindow,
            KeyCode::Char('q') => Action::Quit,
            _ => Action::Tick,
        }
    }

    fn handle_todos_pane_keys(&self, key: KeyEvent, state: &AppState) -> Action {
        if let Some(action) = Self::check_global_keys(&key, &state.system.user_config) {
            return action;
        }

        let get_selected_todo = || -> Option<&crate::models::Todo> {
            state.selected_workspace().and_then(|ws| {
                ws.todos.iter()
                    .filter(|t| match state.ui.selected_todos_tab {
                        TodosTab::Active => !t.is_archived(),
                        TodosTab::Archived => t.is_archived(),
                        TodosTab::Reports => false,
                    })
                    .nth(state.ui.selected_todo_idx)
            })
        };

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if state.ui.selected_todos_tab == TodosTab::Reports {
                    Action::SelectNextReport
                } else {
                    Action::SelectNextTodo
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if state.ui.selected_todos_tab == TodosTab::Reports {
                    Action::SelectPrevReport
                } else {
                    Action::SelectPrevTodo
                }
            }
            KeyCode::Char('l') => Action::FocusRight,
            KeyCode::Tab => Action::ToggleTodosTab,
            KeyCode::Char('v') | KeyCode::Enter if state.ui.selected_todos_tab == TodosTab::Reports => {
                Action::ViewReport
            }
            KeyCode::Char('m') if state.ui.selected_todos_tab == TodosTab::Reports => {
                Action::MergeSelectedReport
            }
            KeyCode::Char('d') if state.ui.selected_todos_tab == TodosTab::Reports => {
                if let Some(task_id) = state.selected_workspace()
                    .and_then(|ws| ws.active_parallel_task())
                    .map(|t| t.id)
                {
                    Action::CancelParallelTask(task_id)
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('n') if state.ui.selected_todos_tab == TodosTab::Active => {
                Action::EnterCreateTodoMode
            }
            KeyCode::Enter if state.ui.selected_todos_tab == TodosTab::Active => {
                Action::RunSelectedTodo
            }
            KeyCode::Char('y') if state.ui.selected_todos_tab == TodosTab::Active => {
                if let Some(todo) = get_selected_todo() {
                    if todo.is_suggested() {
                        Action::ApproveSuggestedTodo(todo.id)
                    } else {
                        Action::Tick
                    }
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('Y') if state.ui.selected_todos_tab == TodosTab::Active => {
                Action::ApproveAllSuggestedTodos
            }
            KeyCode::Char('x') if state.ui.selected_todos_tab == TodosTab::Active => {
                Action::MarkTodoDone
            }
            KeyCode::Char('X') if state.ui.selected_todos_tab == TodosTab::Active => {
                if let Some(todo) = get_selected_todo() {
                    if todo.is_ready_for_review() || todo.is_done() {
                        Action::ArchiveTodo(todo.id)
                    } else {
                        Action::Tick
                    }
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('a') if state.ui.selected_todos_tab == TodosTab::Active => {
                Action::ToggleTodoPaneMode
            }
            KeyCode::Char('d') => {
                if let Some(todo) = get_selected_todo() {
                    let desc = if todo.description.len() > 30 {
                        format!("{}...", &todo.description[..30])
                    } else {
                        todo.description.clone()
                    };
                    Action::InitiateDeleteTodo(todo.id, desc)
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('h') => Action::EnterConfigWindow,
            KeyCode::Char('?') => Action::EnterConfigWindow,
            KeyCode::Char('q') => Action::Quit,
            _ => Action::Tick,
        }
    }

    fn handle_utilities_pane_keys(&self, key: KeyEvent, state: &AppState) -> Action {
        use crate::app::{UtilityItem, UtilitySection};

        if let Some(action) = Self::check_global_keys(&key, &state.system.user_config) {
            return action;
        }

        if state.ui.utility_section == UtilitySection::Notepad {
            if key.code == KeyCode::Tab {
                return Action::ToggleUtilitySection;
            }
            if key.code == KeyCode::Esc {
                return Action::ToggleUtilitySection;
            }
            return Action::NotepadInput(key);
        }

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => Action::SelectNextUtility,
            KeyCode::Char('k') | KeyCode::Up => Action::SelectPrevUtility,
            KeyCode::Char('l') | KeyCode::Enter => {
                match state.ui.utility_section {
                    UtilitySection::Utilities => Action::ActivateUtility,
                    UtilitySection::Sounds => {
                        match state.ui.selected_sound {
                            UtilityItem::BrownNoise => Action::ToggleBrownNoise,
                            UtilityItem::ClassicalRadio => Action::ToggleClassicalRadio,
                            UtilityItem::OceanWaves => Action::ToggleOceanWaves,
                            UtilityItem::WindChimes => Action::ToggleWindChimes,
                            UtilityItem::RainforestRain => Action::ToggleRainforestRain,
                            _ => Action::Tick,
                        }
                    }
                    UtilitySection::GlobalConfig => Action::ToggleConfigItem,
                    UtilitySection::Notepad => Action::Tick,
                }
            }
            KeyCode::Tab => Action::ToggleUtilitySection,
            KeyCode::Char('h') => Action::EnterConfigWindow,
            KeyCode::Char('?') => Action::EnterConfigWindow,
            KeyCode::Char('q') => Action::Quit,
            _ => Action::Tick,
        }
    }

    fn handle_output_pane_keys(&self, key: KeyEvent, state: &AppState) -> Action {
        if let Some(action) = Self::check_global_keys(&key, &state.system.user_config) {
            return action;
        }

        if state.ui.text_selection.start.is_some() {
            match key.code {
                KeyCode::Char('y') => return Action::CopySelection,
                KeyCode::Char('c') | KeyCode::Char('C')
                    if key.modifiers.contains(KeyModifiers::SUPER)
                        || (key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.modifiers.contains(KeyModifiers::SHIFT)) =>
                {
                    return Action::CopySelection;
                }
                KeyCode::Esc => return Action::ClearSelection,
                _ => {}
            }
        }

        if let Some(session_id) = state.ui.active_session_id {
            match key.code {
                KeyCode::Esc => Action::SendInput(session_id, vec![0x1b]),
                KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => Action::ScrollOutputUp,
                KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => Action::ScrollOutputDown,
                KeyCode::PageUp => Action::ScrollOutputUp,
                KeyCode::PageDown => Action::ScrollOutputDown,
                KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::FocusLeft,
                KeyCode::BackTab => Action::SendInput(session_id, b"\x1b[Z".to_vec()),
                KeyCode::Char(c) => {
                    let data = if key.modifiers.contains(KeyModifiers::CONTROL) {
                        vec![(c as u8) & 0x1f]
                    } else if key.modifiers.contains(KeyModifiers::ALT) {
                        vec![0x1b, c as u8]
                    } else {
                        c.to_string().into_bytes()
                    };
                    Action::SendInput(session_id, data)
                }
                KeyCode::Enter => Action::SendInput(session_id, vec![b'\r']),
                KeyCode::Backspace => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, vec![0x1b, 0x7f])
                    } else if key.modifiers.contains(KeyModifiers::SUPER) {
                        Action::SendInput(session_id, vec![0x15])
                    } else {
                        Action::SendInput(session_id, vec![0x7f])
                    }
                }
                KeyCode::Delete => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, vec![0x1b, b'd'])
                    } else {
                        Action::SendInput(session_id, b"\x1b[3~".to_vec())
                    }
                }
                KeyCode::Tab => Action::SendInput(session_id, vec![b'\t']),
                KeyCode::Up => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, b"\x1b[1;3A".to_vec())
                    } else {
                        Action::SendInput(session_id, b"\x1b[A".to_vec())
                    }
                }
                KeyCode::Down => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, b"\x1b[1;3B".to_vec())
                    } else {
                        Action::SendInput(session_id, b"\x1b[B".to_vec())
                    }
                }
                KeyCode::Right => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, vec![0x1b, b'f'])
                    } else if key.modifiers.contains(KeyModifiers::SUPER) {
                        Action::SendInput(session_id, vec![0x05])
                    } else {
                        Action::SendInput(session_id, b"\x1b[C".to_vec())
                    }
                }
                KeyCode::Left => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, vec![0x1b, b'b'])
                    } else if key.modifiers.contains(KeyModifiers::SUPER) {
                        Action::SendInput(session_id, vec![0x01])
                    } else {
                        Action::SendInput(session_id, b"\x1b[D".to_vec())
                    }
                }
                KeyCode::Home => Action::SendInput(session_id, vec![0x01]),
                KeyCode::End => Action::SendInput(session_id, vec![0x05]),
                KeyCode::F(n) => {
                    let seq = match n {
                        1 => b"\x1bOP".to_vec(),
                        2 => b"\x1bOQ".to_vec(),
                        3 => b"\x1bOR".to_vec(),
                        4 => b"\x1bOS".to_vec(),
                        5 => b"\x1b[15~".to_vec(),
                        6 => b"\x1b[17~".to_vec(),
                        7 => b"\x1b[18~".to_vec(),
                        8 => b"\x1b[19~".to_vec(),
                        9 => b"\x1b[20~".to_vec(),
                        10 => b"\x1b[21~".to_vec(),
                        11 => b"\x1b[23~".to_vec(),
                        12 => b"\x1b[24~".to_vec(),
                        _ => vec![],
                    };
                    if seq.is_empty() {
                        Action::Tick
                    } else {
                        Action::SendInput(session_id, seq)
                    }
                }
                KeyCode::Insert => Action::SendInput(session_id, b"\x1b[2~".to_vec()),
                _ => Action::Tick,
            }
        } else {
            match key.code {
                KeyCode::Char('h') | KeyCode::Esc => Action::FocusLeft,
                KeyCode::Char('?') => Action::EnterConfigWindow,
                KeyCode::Char('q') => Action::Quit,
                _ => Action::Tick,
            }
        }
    }

    fn handle_pinned_terminal_keys(&self, key: KeyEvent, state: &AppState, pane_idx: usize) -> Action {
        if let Some(action) = Self::check_global_keys(&key, &state.system.user_config) {
            return action;
        }

        if state.ui.pinned_text_selections.get(pane_idx).map(|s| s.start.is_some()).unwrap_or(false) {
            match key.code {
                KeyCode::Char('y') => return Action::CopySelection,
                KeyCode::Char('c') | KeyCode::Char('C')
                    if key.modifiers.contains(KeyModifiers::SUPER)
                        || (key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.modifiers.contains(KeyModifiers::SHIFT)) =>
                {
                    return Action::CopySelection;
                }
                KeyCode::Esc => return Action::ClearSelection,
                _ => {}
            }
        }

        if let Some(session_id) = state.pinned_terminal_id_at(pane_idx) {
            match key.code {
                KeyCode::Esc => Action::SendInput(session_id, vec![0x1b]),
                KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::FocusLeft,
                KeyCode::BackTab => Action::SendInput(session_id, b"\x1b[Z".to_vec()),
                KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::NextPinnedPane,
                KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::PrevPinnedPane,
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::UnpinFocusedSession,
                KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => Action::ScrollOutputUp,
                KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => Action::ScrollOutputDown,
                KeyCode::PageUp => Action::ScrollOutputUp,
                KeyCode::PageDown => Action::ScrollOutputDown,
                KeyCode::Char(c) => {
                    let data = if key.modifiers.contains(KeyModifiers::CONTROL) {
                        vec![(c as u8) & 0x1f]
                    } else if key.modifiers.contains(KeyModifiers::ALT) {
                        vec![0x1b, c as u8]
                    } else {
                        c.to_string().into_bytes()
                    };
                    Action::SendInput(session_id, data)
                }
                KeyCode::Enter => Action::SendInput(session_id, vec![b'\r']),
                KeyCode::Backspace => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, vec![0x1b, 0x7f])
                    } else if key.modifiers.contains(KeyModifiers::SUPER) {
                        Action::SendInput(session_id, vec![0x15])
                    } else {
                        Action::SendInput(session_id, vec![0x7f])
                    }
                }
                KeyCode::Delete => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, vec![0x1b, b'd'])
                    } else {
                        Action::SendInput(session_id, b"\x1b[3~".to_vec())
                    }
                }
                KeyCode::Tab => Action::SendInput(session_id, vec![b'\t']),
                KeyCode::Up => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, b"\x1b[1;3A".to_vec())
                    } else {
                        Action::SendInput(session_id, b"\x1b[A".to_vec())
                    }
                }
                KeyCode::Down => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, b"\x1b[1;3B".to_vec())
                    } else {
                        Action::SendInput(session_id, b"\x1b[B".to_vec())
                    }
                }
                KeyCode::Right => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, vec![0x1b, b'f'])
                    } else if key.modifiers.contains(KeyModifiers::SUPER) {
                        Action::SendInput(session_id, vec![0x05])
                    } else {
                        Action::SendInput(session_id, b"\x1b[C".to_vec())
                    }
                }
                KeyCode::Left => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, vec![0x1b, b'b'])
                    } else if key.modifiers.contains(KeyModifiers::SUPER) {
                        Action::SendInput(session_id, vec![0x01])
                    } else {
                        Action::SendInput(session_id, b"\x1b[D".to_vec())
                    }
                }
                KeyCode::Home => Action::SendInput(session_id, vec![0x01]),
                KeyCode::End => Action::SendInput(session_id, vec![0x05]),
                KeyCode::F(n) => {
                    let seq = match n {
                        1 => b"\x1bOP".to_vec(),
                        2 => b"\x1bOQ".to_vec(),
                        3 => b"\x1bOR".to_vec(),
                        4 => b"\x1bOS".to_vec(),
                        5 => b"\x1b[15~".to_vec(),
                        6 => b"\x1b[17~".to_vec(),
                        7 => b"\x1b[18~".to_vec(),
                        8 => b"\x1b[19~".to_vec(),
                        9 => b"\x1b[20~".to_vec(),
                        10 => b"\x1b[21~".to_vec(),
                        11 => b"\x1b[23~".to_vec(),
                        12 => b"\x1b[24~".to_vec(),
                        _ => vec![],
                    };
                    if seq.is_empty() {
                        Action::Tick
                    } else {
                        Action::SendInput(session_id, seq)
                    }
                }
                KeyCode::Insert => Action::SendInput(session_id, b"\x1b[2~".to_vec()),
                _ => Action::Tick,
            }
        } else {
            match key.code {
                KeyCode::Esc | KeyCode::Char('h') => Action::FocusLeft,
                _ => Action::Tick,
            }
        }
    }

    pub(super) fn agent_shortcut(key: &KeyEvent, agents: &[crate::config::user_config::AgentConfig]) -> Option<(AgentType, bool, bool)> {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            || key.modifiers.contains(KeyModifiers::SUPER)
            || key.modifiers.contains(KeyModifiers::META)
        {
            return None;
        }

        let shifted = key.modifiers.contains(KeyModifiers::SHIFT);
        let with_worktree = key.modifiers.contains(KeyModifiers::ALT);

        let key_char = match key.code {
            KeyCode::Char(c) => c.to_string(),
            _ => return None,
        };

        // Map shift+number to the number (e.g. '!' -> "1", '@' -> "2")
        let unshifted = match key_char.as_str() {
            "!" => Some("1"), "@" => Some("2"), "#" => Some("3"), "$" => Some("4"),
            "%" => Some("5"), "^" => Some("6"), "&" => Some("7"), "*" => Some("8"),
            "(" => Some("9"),
            _ => None,
        };

        for agent in agents {
            if !agent.enabled { continue; }
            let matches = agent.hotkey == key_char
                || unshifted.map(|s| s == agent.hotkey).unwrap_or(false);
            if matches {
                let agent_type = config_to_agent_type(agent);
                let skip_perms = shifted || unshifted.is_some();
                return Some((agent_type, skip_perms, with_worktree));
            }
        }
        None
    }
}

fn config_to_agent_type(agent: &crate::config::user_config::AgentConfig) -> AgentType {
    match agent.command.as_str() {
        "claude" => AgentType::Claude,
        "gemini" => AgentType::Gemini,
        "codex" => AgentType::Codex,
        "grok" => AgentType::Grok,
        _ => AgentType::Custom {
            command: agent.command.clone(),
            display_name: agent.display_name.clone(),
            badge: agent.badge.clone(),
        },
    }
}
