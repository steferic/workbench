use crate::app::{Action, AppState, FocusPanel, InputMode, PendingDelete, TodosTab};
use crate::models::AgentType;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
use std::time::Duration;
use tokio::sync::mpsc;

/// Internal event type for terminal events
enum TerminalEvent {
    Key(KeyEvent),
    Paste(String),
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
                        Ok(Event::Paste(data)) => TerminalEvent::Paste(data),
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
                    TerminalEvent::Paste(data) => Ok(Action::Paste(data)),
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
    match state.ui.input_mode {
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
                    let name = state.ui.input_buffer.clone();
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
                    if state.ui.workspace_create_mode {
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
            if let Some((agent_type, dangerously_skip_permissions)) = Self::agent_shortcut(&key)
            {
                return Action::CreateSession(agent_type, dangerously_skip_permissions);
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
        InputMode::Normal => {}
    }

    // Handle pending delete confirmation
    if state.ui.pending_delete.is_some() {
        return match key.code {
            KeyCode::Char('d') => {
                // Second 'd' confirms the delete
                match &state.ui.pending_delete {
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

        if let Some((agent_type, dangerously_skip_permissions)) = Self::agent_shortcut(&key) {
            return Action::CreateSession(agent_type, dangerously_skip_permissions);
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
                    .filter(|t| match state.ui.selected_todos_tab {
                        TodosTab::Active => !t.is_archived(),
                        TodosTab::Archived => t.is_archived(),
                    })
                    .nth(state.ui.selected_todo_idx)
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
            KeyCode::Char('n') if state.ui.selected_todos_tab == TodosTab::Active => {
                Action::EnterCreateTodoMode
            }
            KeyCode::Enter if state.ui.selected_todos_tab == TodosTab::Active => {
                Action::RunSelectedTodo
            }
            KeyCode::Char('y') if state.ui.selected_todos_tab == TodosTab::Active => {
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
            KeyCode::Char('Y') if state.ui.selected_todos_tab == TodosTab::Active => {
                Action::ApproveAllSuggestedTodos
            }
            KeyCode::Char('x') if state.ui.selected_todos_tab == TodosTab::Active => {
                Action::MarkTodoDone
            }
            KeyCode::Char('X') if state.ui.selected_todos_tab == TodosTab::Active => {
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
            KeyCode::Char('a') if state.ui.selected_todos_tab == TodosTab::Active => {
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

        // Special handling for Notepad section - pass keys to TextArea
        if state.ui.utility_section == UtilitySection::Notepad {
            // Tab switches sections
            if key.code == KeyCode::Tab {
                return Action::ToggleUtilitySection;
            }
            // Escape exits notepad focus
            if key.code == KeyCode::Esc {
                return Action::ToggleUtilitySection;
            }
            // Ctrl+Q quits
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
                return Action::Quit;
            }
            // Pass all other keys to TextArea (handles Ctrl+K, word nav, undo/redo, etc.)
            return Action::NotepadInput(key);
        }

        match key.code {
            // Navigation within current section
            KeyCode::Char('j') | KeyCode::Down => Action::SelectNextUtility,
            KeyCode::Char('k') | KeyCode::Up => Action::SelectPrevUtility,
            KeyCode::Char('h') => Action::FocusLeft,

            // Activate/toggle based on section
            KeyCode::Char('l') | KeyCode::Enter => {
                match state.ui.utility_section {
                    UtilitySection::Utilities => {
                        // Check if selected utility is a toggle
                        if state.ui.selected_utility == UtilityItem::BrownNoise {
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
                _ => {} // Fall through to normal handling
            }
        }

        // If there's an active session, send input to PTY
        if let Some(session_id) = state.ui.active_session_id {
            match key.code {
                // Escape sends to PTY (for interrupting Claude Code, etc.)
                KeyCode::Esc => Action::SendInput(session_id, vec![0x1b]),

                // Scrolling with Shift+Up/Down or PageUp/PageDown
                KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    Action::ScrollOutputUp
                }
                KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    Action::ScrollOutputDown
                }
                KeyCode::PageUp => Action::ScrollOutputUp,
                KeyCode::PageDown => Action::ScrollOutputDown,

                // Panel navigation with Ctrl+H (Ctrl+Esc to leave)
                KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Action::FocusLeft
                }

                // Send to PTY with proper escape sequences for modifiers
                KeyCode::Char(c) => {
                    let data = if key.modifiers.contains(KeyModifiers::CONTROL) {
                        // Convert to control character
                        vec![(c as u8) & 0x1f]
                    } else if key.modifiers.contains(KeyModifiers::ALT) {
                        // Alt+char sends ESC followed by char (meta key)
                        vec![0x1b, c as u8]
                    } else {
                        c.to_string().into_bytes()
                    };
                    Action::SendInput(session_id, data)
                }
                KeyCode::Enter => Action::SendInput(session_id, vec![b'\r']),
                KeyCode::Backspace => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        // Alt+Backspace: delete word backward (ESC + DEL)
                        Action::SendInput(session_id, vec![0x1b, 0x7f])
                    } else if key.modifiers.contains(KeyModifiers::SUPER) {
                        // Cmd+Backspace: delete to start of line (Ctrl+U)
                        Action::SendInput(session_id, vec![0x15])
                    } else {
                        Action::SendInput(session_id, vec![0x7f])
                    }
                }
                KeyCode::Delete => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        // Alt+Delete: delete word forward (ESC + d)
                        Action::SendInput(session_id, vec![0x1b, b'd'])
                    } else {
                        // Delete forward
                        Action::SendInput(session_id, b"\x1b[3~".to_vec())
                    }
                }
                KeyCode::Tab => Action::SendInput(session_id, vec![b'\t']),
                KeyCode::Up => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        // Alt+Up: scroll up or command history (CSI 1;3 A)
                        Action::SendInput(session_id, b"\x1b[1;3A".to_vec())
                    } else {
                        Action::SendInput(session_id, b"\x1b[A".to_vec())
                    }
                }
                KeyCode::Down => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        // Alt+Down (CSI 1;3 B)
                        Action::SendInput(session_id, b"\x1b[1;3B".to_vec())
                    } else {
                        Action::SendInput(session_id, b"\x1b[B".to_vec())
                    }
                }
                KeyCode::Right => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        // Alt+Right: move word forward (ESC f or CSI 1;3 C)
                        Action::SendInput(session_id, vec![0x1b, b'f'])
                    } else if key.modifiers.contains(KeyModifiers::SUPER) {
                        // Cmd+Right: end of line (Ctrl+E)
                        Action::SendInput(session_id, vec![0x05])
                    } else {
                        Action::SendInput(session_id, b"\x1b[C".to_vec())
                    }
                }
                KeyCode::Left => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        // Alt+Left: move word backward (ESC b or CSI 1;3 D)
                        Action::SendInput(session_id, vec![0x1b, b'b'])
                    } else if key.modifiers.contains(KeyModifiers::SUPER) {
                        // Cmd+Left: start of line (Ctrl+A)
                        Action::SendInput(session_id, vec![0x01])
                    } else {
                        Action::SendInput(session_id, b"\x1b[D".to_vec())
                    }
                }
                KeyCode::Home => {
                    // Home: start of line
                    Action::SendInput(session_id, vec![0x01])
                }
                KeyCode::End => {
                    // End: end of line
                    Action::SendInput(session_id, vec![0x05])
                }
                // Function keys
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
                // Insert key
                KeyCode::Insert => Action::SendInput(session_id, b"\x1b[2~".to_vec()),

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
                _ => {} // Fall through to normal handling
            }
        }

        // Get the pinned terminal ID for this pane
        if let Some(session_id) = state.pinned_terminal_id_at(pane_idx) {
            match key.code {
                // Escape sends to PTY (for interrupting Claude Code, etc.)
                KeyCode::Esc => Action::SendInput(session_id, vec![0x1b]),
                // Ctrl+H to leave pinned pane
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

                // Send to pinned terminal PTY with proper escape sequences
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
                // Function keys
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
                // Insert key
                KeyCode::Insert => Action::SendInput(session_id, b"\x1b[2~".to_vec()),

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

    fn agent_shortcut(key: &KeyEvent) -> Option<(AgentType, bool)> {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            || key.modifiers.contains(KeyModifiers::ALT)
        {
            return None;
        }

        let shifted = key.modifiers.contains(KeyModifiers::SHIFT);

        match key.code {
            KeyCode::Char('1') => Some((AgentType::Claude, shifted)),
            KeyCode::Char('2') => Some((AgentType::Gemini, shifted)),
            KeyCode::Char('3') => Some((AgentType::Codex, shifted)),
            KeyCode::Char('4') => Some((AgentType::Grok, shifted)),
            KeyCode::Char('!') => Some((AgentType::Claude, true)),
            KeyCode::Char('@') => Some((AgentType::Gemini, true)),
            KeyCode::Char('#') => Some((AgentType::Codex, true)),
            KeyCode::Char('$') => Some((AgentType::Grok, true)),
            _ => None,
        }
    }
}

impl Default for EventHandler {
    fn default() -> Self {
        Self::new()
    }
}
