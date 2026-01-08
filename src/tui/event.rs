use crate::app::{Action, AppState, FocusPanel, InputMode, PendingDelete, TodosTab};
use crate::models::AgentType;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
use std::time::Duration;
use tokio::sync::mpsc;

/// Internal event type for terminal events
enum TerminalEvent {
    Key(KeyEvent),
    MouseDown(u16, u16),
    MouseDrag(u16, u16),
    MouseUp(u16, u16),
    MouseScrollUp,
    MouseScrollDown,
    Resize(u16, u16),
    Tick,
}

pub struct EventHandler {
    action_tx: mpsc::UnboundedSender<Action>,
    action_rx: mpsc::UnboundedReceiver<Action>,
    terminal_rx: mpsc::UnboundedReceiver<TerminalEvent>,
}

impl EventHandler {
    pub fn new() -> Self {
        let (action_tx, action_rx) = mpsc::unbounded_channel();
        let (terminal_tx, terminal_rx) = mpsc::unbounded_channel();

        // Spawn dedicated thread for terminal events
        std::thread::spawn(move || {
            let poll_timeout = Duration::from_millis(50);
            loop {
                let event = if event::poll(poll_timeout).unwrap_or(false) {
                    match event::read() {
                        Ok(Event::Key(key)) => TerminalEvent::Key(key),
                        Ok(Event::Mouse(mouse)) => match mouse.kind {
                            MouseEventKind::Down(MouseButton::Left) => {
                                TerminalEvent::MouseDown(mouse.column, mouse.row)
                            }
                            MouseEventKind::Drag(MouseButton::Left) => {
                                TerminalEvent::MouseDrag(mouse.column, mouse.row)
                            }
                            MouseEventKind::Up(MouseButton::Left) => {
                                TerminalEvent::MouseUp(mouse.column, mouse.row)
                            }
                            MouseEventKind::ScrollUp => TerminalEvent::MouseScrollUp,
                            MouseEventKind::ScrollDown => TerminalEvent::MouseScrollDown,
                            _ => TerminalEvent::Tick,
                        },
                        Ok(Event::Resize(w, h)) => TerminalEvent::Resize(w, h),
                        _ => TerminalEvent::Tick,
                    }
                } else {
                    TerminalEvent::Tick
                };

                if terminal_tx.send(event).is_err() {
                    break; // Channel closed, exit thread
                }
            }
        });

        Self {
            action_tx,
            action_rx,
            terminal_rx,
        }
    }

    pub fn action_sender(&self) -> mpsc::UnboundedSender<Action> {
        self.action_tx.clone()
    }

    pub async fn next(&mut self, state: &AppState) -> Result<Action> {
        tokio::select! {
            // Terminal events (keyboard, mouse, resize)
            Some(event) = self.terminal_rx.recv() => {
                match event {
                    TerminalEvent::Key(key) => Ok(self.handle_key_event(key, state)),
                    TerminalEvent::MouseDown(x, y) => Ok(Action::MouseClick(x, y)),
                    TerminalEvent::MouseDrag(x, y) => Ok(Action::MouseDrag(x, y)),
                    TerminalEvent::MouseUp(x, y) => Ok(Action::MouseUp(x, y)),
                    TerminalEvent::MouseScrollUp => Ok(Action::ScrollOutputUp),
                    TerminalEvent::MouseScrollDown => Ok(Action::ScrollOutputDown),
                    TerminalEvent::Resize(w, h) => Ok(Action::Resize(w, h)),
                    TerminalEvent::Tick => Ok(Action::Tick),
                }
            }
            // PTY output and other actions
            Some(action) = self.action_rx.recv() => {
                Ok(action)
            }
            else => Ok(Action::Tick)
        }
    }

    fn handle_key_event(&self, key: KeyEvent, state: &AppState) -> Action {
        // Handle input mode first
        match state.input_mode {
            InputMode::Help => {
                return match key.code {
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') | KeyCode::Enter => {
                        Action::ExitMode
                    }
                    _ => Action::Tick,
                };
            }
            InputMode::SelectWorkspaceAction => {
                return match key.code {
                    KeyCode::Esc => Action::ExitMode,
                    KeyCode::Char('j') | KeyCode::Down => Action::SelectNextWorkspaceAction,
                    KeyCode::Char('k') | KeyCode::Up => Action::SelectPrevWorkspaceAction,
                    KeyCode::Enter => Action::ConfirmWorkspaceAction,
                    _ => Action::Tick,
                };
            }
            InputMode::EnterWorkspaceName => {
                return match key.code {
                    KeyCode::Esc => Action::ExitMode,
                    KeyCode::Enter => {
                        let name = state.input_buffer.clone();
                        if name.is_empty() {
                            Action::Tick // Don't allow empty names
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
                return match key.code {
                    KeyCode::Esc => Action::ExitMode,
                    KeyCode::Char('j') | KeyCode::Down => Action::FileBrowserDown,
                    KeyCode::Char('k') | KeyCode::Up => Action::FileBrowserUp,
                    KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter => Action::FileBrowserEnter,
                    KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace => Action::FileBrowserBack,
                    KeyCode::Char(' ') | KeyCode::Tab => {
                        if state.workspace_create_mode {
                            // In "Create New" mode, go to name input
                            Action::EnterWorkspaceNameMode
                        } else {
                            // In "Open Existing" mode, select directory
                            Action::FileBrowserSelect
                        }
                    }
                    _ => Action::Tick,
                };
            }
            InputMode::CreateSession => {
                return match key.code {
                    KeyCode::Esc => Action::ExitMode,
                    KeyCode::Char('1') => Action::CreateSession(AgentType::Claude),
                    KeyCode::Char('2') => Action::CreateSession(AgentType::Gemini),
                    KeyCode::Char('3') => Action::CreateSession(AgentType::Codex),
                    KeyCode::Char('4') => Action::CreateSession(AgentType::Grok),
                    KeyCode::Char('t') => Action::CreateTerminal,
                    _ => Action::Tick,
                };
            }
            InputMode::SetStartCommand => {
                return match key.code {
                    KeyCode::Esc => Action::ExitMode,
                    KeyCode::Enter => {
                        if let Some(session_id) = state.editing_session_id {
                            let cmd = state.input_buffer.clone();
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
                        let desc = state.input_buffer.clone();
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
            InputMode::Normal => {}
        }

        // Handle pending delete confirmation
        if state.pending_delete.is_some() {
            return match key.code {
                KeyCode::Char('d') => {
                    // Second 'd' confirms the delete
                    match &state.pending_delete {
                        Some(PendingDelete::Session(_, _)) => Action::ConfirmDeleteSession,
                        Some(PendingDelete::Workspace(_, _)) => Action::ConfirmDeleteWorkspace,
                        Some(PendingDelete::Todo(_, _)) => Action::ConfirmDeleteTodo,
                        None => Action::Tick,
                    }
                }
                KeyCode::Esc => Action::CancelPendingDelete,
                _ => Action::CancelPendingDelete, // Any other key cancels
            };
        }

        // Note: ` (backtick) for JumpToNextIdle is handled in each focus handler
        // to ensure it's caught before the catch-all PTY input handlers

        // Global window navigation with Shift+Left/Right arrows
        if key.modifiers.contains(KeyModifiers::SHIFT) {
            match key.code {
                KeyCode::Left => return Action::FocusLeft,
                KeyCode::Right => return Action::FocusRight,
                _ => {}
            }
        }

        // Normal mode key handling based on focused panel
        match state.focus {
            FocusPanel::WorkspaceList => self.handle_workspace_list_keys(key, state),
            FocusPanel::SessionList => self.handle_session_list_keys(key, state),
            FocusPanel::TodosPane => self.handle_todos_pane_keys(key, state),
            FocusPanel::UtilitiesPane => self.handle_utilities_pane_keys(key, state),
            FocusPanel::OutputPane => self.handle_output_pane_keys(key, state),
            FocusPanel::PinnedTerminalPane(idx) => self.handle_pinned_terminal_keys(key, state, idx),
        }
    }

    fn handle_workspace_list_keys(&self, key: KeyEvent, state: &AppState) -> Action {
        // ` = Jump to next idle session (global shortcut)
        if key.code == KeyCode::Char('`') {
            return Action::JumpToNextIdle;
        }

        match key.code {
            // Navigation
            KeyCode::Char('j') | KeyCode::Down => Action::MoveDown,
            KeyCode::Char('k') | KeyCode::Up => Action::MoveUp,
            KeyCode::Char('l') => Action::FocusRight,

            // Actions
            KeyCode::Char('n') => Action::EnterWorkspaceActionMode,
            KeyCode::Char('w') => Action::ToggleWorkspaceStatus,
            KeyCode::Enter => Action::FocusRight,

            // Delete workspace (requires confirmation)
            KeyCode::Char('d') => {
                if let Some(workspace) = state.selected_workspace() {
                    Action::InitiateDeleteWorkspace(workspace.id, workspace.name.clone())
                } else {
                    Action::Tick
                }
            }

            // Global
            KeyCode::Char('?') => Action::EnterHelpMode,
            KeyCode::Char('q') => Action::Quit,
            KeyCode::Esc => Action::Quit,

            _ => Action::Tick,
        }
    }

    fn handle_session_list_keys(&self, key: KeyEvent, state: &AppState) -> Action {
        // ` = Jump to next idle session (global shortcut)
        if key.code == KeyCode::Char('`') {
            return Action::JumpToNextIdle;
        }

        match key.code {
            // Navigation
            KeyCode::Char('j') | KeyCode::Down => Action::MoveDown,
            KeyCode::Char('k') | KeyCode::Up => Action::MoveUp,
            KeyCode::Char('h') => Action::FocusLeft,
            KeyCode::Char('l') => Action::FocusRight,

            // Actions
            KeyCode::Char('n') => Action::EnterCreateSessionMode,
            KeyCode::Enter => {
                if let Some(session) = state.selected_session() {
                    if session.status == crate::models::SessionStatus::Stopped {
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
                    if session.status == crate::models::SessionStatus::Stopped {
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

            // Agent shortcuts
            KeyCode::Char('1') => Action::CreateSession(AgentType::Claude),
            KeyCode::Char('2') => Action::CreateSession(AgentType::Gemini),
            KeyCode::Char('3') => Action::CreateSession(AgentType::Codex),
            KeyCode::Char('4') => Action::CreateSession(AgentType::Grok),

            // Terminal shortcut (auto-named)
            KeyCode::Char('t') => Action::CreateTerminal,

            // Set start command for terminal sessions
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

            // Pin session to workspace (any session type)
            KeyCode::Char('p') => {
                if let Some(session) = state.selected_session() {
                    Action::PinSession(session.id)
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

            // Toggle split view
            KeyCode::Char('\\') | KeyCode::Char('/') => Action::ToggleSplitView,

            // Global
            KeyCode::Char('?') => Action::EnterHelpMode,
            KeyCode::Char('q') => Action::Quit,

            _ => Action::Tick,
        }
    }

    fn handle_todos_pane_keys(&self, key: KeyEvent, state: &AppState) -> Action {
        // ` = Jump to next idle session (global shortcut)
        if key.code == KeyCode::Char('`') {
            return Action::JumpToNextIdle;
        }

        // Helper to get the selected todo based on current tab
        let get_selected_todo = || -> Option<&crate::models::Todo> {
            state.selected_workspace().and_then(|ws| {
                ws.todos.iter()
                    .filter(|t| match state.selected_todos_tab {
                        TodosTab::Active => !t.is_archived(),
                        TodosTab::Archived => t.is_archived(),
                    })
                    .nth(state.selected_todo_idx)
            })
        };

        match key.code {
            // Navigation
            KeyCode::Char('j') | KeyCode::Down => Action::SelectNextTodo,
            KeyCode::Char('k') | KeyCode::Up => Action::SelectPrevTodo,
            KeyCode::Char('h') => Action::FocusLeft,
            KeyCode::Char('l') => Action::FocusRight,

            // Tab switching
            KeyCode::Tab => Action::ToggleTodosTab,

            // Actions (only in Active tab)
            KeyCode::Char('n') if state.selected_todos_tab == TodosTab::Active => {
                Action::EnterCreateTodoMode
            }
            KeyCode::Enter if state.selected_todos_tab == TodosTab::Active => {
                Action::RunSelectedTodo
            }
            KeyCode::Char('y') if state.selected_todos_tab == TodosTab::Active => {
                // Approve suggested todo (convert to Pending)
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
            KeyCode::Char('Y') if state.selected_todos_tab == TodosTab::Active => {
                Action::ApproveAllSuggestedTodos
            }
            KeyCode::Char('x') if state.selected_todos_tab == TodosTab::Active => {
                Action::MarkTodoDone
            }
            KeyCode::Char('X') if state.selected_todos_tab == TodosTab::Active => {
                // Archive todo (for review/done items)
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
            KeyCode::Char('a') if state.selected_todos_tab == TodosTab::Active => {
                Action::ToggleTodoPaneMode
            }

            // Delete works in both tabs
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

            // Global
            KeyCode::Char('?') => Action::EnterHelpMode,
            KeyCode::Char('q') => Action::Quit,

            _ => Action::Tick,
        }
    }

    fn handle_utilities_pane_keys(&self, key: KeyEvent, state: &AppState) -> Action {
        use crate::app::{UtilityItem, UtilitySection};

        // ` = Jump to next idle session (global shortcut)
        if key.code == KeyCode::Char('`') {
            return Action::JumpToNextIdle;
        }

        // Special handling for Notepad section - capture all input
        if state.utility_section == UtilitySection::Notepad {
            // Tab still switches sections
            if key.code == KeyCode::Tab {
                return Action::ToggleUtilitySection;
            }
            // Escape exits notepad focus
            if key.code == KeyCode::Esc {
                return Action::ToggleUtilitySection;
            }

            // Handle modifier key combinations first
            let has_alt = key.modifiers.contains(KeyModifiers::ALT);
            let has_super = key.modifiers.contains(KeyModifiers::SUPER);
            let has_ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

            // Word/line deletion shortcuts
            if key.code == KeyCode::Backspace {
                if has_super {
                    return Action::NotepadDeleteLine; // Cmd+Backspace: delete to start of line
                } else if has_alt {
                    return Action::NotepadDeleteWord; // Option+Backspace: delete word before cursor
                }
                return Action::NotepadBackspace;
            }

            if key.code == KeyCode::Delete {
                if has_super {
                    return Action::NotepadDeleteToEnd; // Cmd+Delete: delete to end of line
                } else if has_alt {
                    return Action::NotepadDeleteWordForward; // Option+Delete: delete word after cursor
                }
                return Action::NotepadDelete;
            }

            // Word navigation shortcuts
            if key.code == KeyCode::Left {
                if has_alt {
                    return Action::NotepadWordLeft; // Option+Left: move to previous word
                }
                return Action::NotepadCursorLeft;
            }

            if key.code == KeyCode::Right {
                if has_alt {
                    return Action::NotepadWordRight; // Option+Right: move to next word
                }
                return Action::NotepadCursorRight;
            }

            // Ctrl+K for delete to end of line (emacs style)
            if has_ctrl {
                if let KeyCode::Char('k') = key.code {
                    return Action::NotepadDeleteToEnd;
                }
            }

            // Handle notepad input
            return match key.code {
                KeyCode::Char(c) => {
                    // Ctrl+V for paste
                    if has_ctrl && c == 'v' {
                        Action::NotepadPaste
                    } else if has_ctrl && c == 'q' {
                        Action::Quit
                    } else {
                        Action::NotepadChar(c)
                    }
                }
                KeyCode::Enter => Action::NotepadNewline,
                KeyCode::Home => Action::NotepadCursorHome,
                KeyCode::End => Action::NotepadCursorEnd,
                _ => Action::Tick,
            };
        }

        match key.code {
            // Navigation within current section
            KeyCode::Char('j') | KeyCode::Down => Action::SelectNextUtility,
            KeyCode::Char('k') | KeyCode::Up => Action::SelectPrevUtility,
            KeyCode::Char('h') => Action::FocusLeft,

            // Activate/toggle based on section
            KeyCode::Char('l') | KeyCode::Enter => {
                match state.utility_section {
                    UtilitySection::Utilities => {
                        // Check if selected utility is a toggle
                        if state.selected_utility == UtilityItem::BrownNoise {
                            Action::ToggleBrownNoise
                        } else {
                            Action::ActivateUtility
                        }
                    }
                    UtilitySection::GlobalConfig => Action::ToggleConfigItem,
                    UtilitySection::Notepad => Action::Tick, // Handled above
                }
            }

            // Tab to switch between sections
            KeyCode::Tab => Action::ToggleUtilitySection,

            // Global
            KeyCode::Char('?') => Action::EnterHelpMode,
            KeyCode::Char('q') => Action::Quit,

            _ => Action::Tick,
        }
    }

    fn handle_output_pane_keys(&self, key: KeyEvent, state: &AppState) -> Action {
        // ` = Jump to next idle session (global shortcut, checked first)
        if key.code == KeyCode::Char('`') {
            return Action::JumpToNextIdle;
        }

        // Check if there's a text selection - 'y' copies, Esc clears
        if state.text_selection.start.is_some() {
            match key.code {
                KeyCode::Char('y') => return Action::CopySelection,
                KeyCode::Esc => return Action::ClearSelection,
                _ => {} // Fall through to normal handling
            }
        }

        // If there's an active session, send input to PTY
        if let Some(session_id) = state.active_session_id {
            match key.code {
                // Escape to leave output pane (only if no selection)
                KeyCode::Esc => Action::FocusLeft,

                // Scrolling with Shift+Up/Down or PageUp/PageDown
                KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    Action::ScrollOutputUp
                }
                KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    Action::ScrollOutputDown
                }
                KeyCode::PageUp => Action::ScrollOutputUp,
                KeyCode::PageDown => Action::ScrollOutputDown,

                // Panel navigation with Ctrl+H
                KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Action::FocusLeft
                }

                // Send to PTY
                KeyCode::Char(c) => {
                    let data = if key.modifiers.contains(KeyModifiers::CONTROL) {
                        // Convert to control character
                        vec![(c as u8) & 0x1f]
                    } else {
                        c.to_string().into_bytes()
                    };
                    Action::SendInput(session_id, data)
                }
                KeyCode::Enter => Action::SendInput(session_id, vec![b'\r']),
                KeyCode::Backspace => Action::SendInput(session_id, vec![0x7f]),
                KeyCode::Tab => Action::SendInput(session_id, vec![b'\t']),
                KeyCode::Up => Action::SendInput(session_id, vec![0x1b, b'[', b'A']),
                KeyCode::Down => Action::SendInput(session_id, vec![0x1b, b'[', b'B']),
                KeyCode::Right => Action::SendInput(session_id, vec![0x1b, b'[', b'C']),
                KeyCode::Left => Action::SendInput(session_id, vec![0x1b, b'[', b'D']),

                _ => Action::Tick,
            }
        } else {
            // No active session - allow navigation
            match key.code {
                KeyCode::Char('h') | KeyCode::Esc => Action::FocusLeft,
                KeyCode::Char('?') => Action::EnterHelpMode,
                KeyCode::Char('q') => Action::Quit,
                _ => Action::Tick,
            }
        }
    }

    fn handle_pinned_terminal_keys(&self, key: KeyEvent, state: &AppState, pane_idx: usize) -> Action {
        // ` = Jump to next idle session (global shortcut, checked first)
        if key.code == KeyCode::Char('`') {
            return Action::JumpToNextIdle;
        }

        // Check if there's a text selection - 'y' copies, Esc clears
        if state.pinned_text_selections.get(pane_idx).map(|s| s.start.is_some()).unwrap_or(false) {
            match key.code {
                KeyCode::Char('y') => return Action::CopySelection,
                KeyCode::Esc => return Action::ClearSelection,
                _ => {} // Fall through to normal handling
            }
        }

        // Get the pinned terminal ID for this pane
        if let Some(session_id) = state.pinned_terminal_id_at(pane_idx) {
            match key.code {
                // Escape or Ctrl+H to leave pinned pane
                KeyCode::Esc => Action::FocusLeft,
                KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Action::FocusLeft
                }

                // Navigate between pinned panes with Ctrl+J/K
                KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Action::NextPinnedPane
                }
                KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Action::PrevPinnedPane
                }

                // Unpin current pane with Ctrl+U
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Action::UnpinFocusedSession
                }

                // Scrolling with Shift+Up/Down or PageUp/PageDown
                KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    Action::ScrollOutputUp
                }
                KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    Action::ScrollOutputDown
                }
                KeyCode::PageUp => Action::ScrollOutputUp,
                KeyCode::PageDown => Action::ScrollOutputDown,

                // Send to pinned terminal PTY
                KeyCode::Char(c) => {
                    let data = if key.modifiers.contains(KeyModifiers::CONTROL) {
                        vec![(c as u8) & 0x1f]
                    } else {
                        c.to_string().into_bytes()
                    };
                    Action::SendInput(session_id, data)
                }
                KeyCode::Enter => Action::SendInput(session_id, vec![b'\r']),
                KeyCode::Backspace => Action::SendInput(session_id, vec![0x7f]),
                KeyCode::Tab => Action::SendInput(session_id, vec![b'\t']),
                KeyCode::Up => Action::SendInput(session_id, vec![0x1b, b'[', b'A']),
                KeyCode::Down => Action::SendInput(session_id, vec![0x1b, b'[', b'B']),
                KeyCode::Right => Action::SendInput(session_id, vec![0x1b, b'[', b'C']),
                KeyCode::Left => Action::SendInput(session_id, vec![0x1b, b'[', b'D']),

                _ => Action::Tick,
            }
        } else {
            // No pinned terminal in this slot - go back
            match key.code {
                KeyCode::Esc | KeyCode::Char('h') => Action::FocusLeft,
                _ => Action::Tick,
            }
        }
    }
}

impl Default for EventHandler {
    fn default() -> Self {
        Self::new()
    }
}
