use crate::models::AgentType;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum Action {
    // Navigation
    MoveUp,
    MoveDown,
    FocusLeft,
    FocusRight,
    FocusUtilitiesPane,
    ScrollOutputUp,
    ScrollOutputDown,
    ScrollOutputToBottom,
    JumpToNextIdle,

    // Workspace operations
    CreateWorkspace(PathBuf),
    SelectWorkspace(usize),
    DeleteWorkspace(Uuid),
    ToggleWorkspaceStatus,
    InitiateDeleteWorkspace(Uuid, String),  // (id, name) - first 'd' press
    ConfirmDeleteWorkspace,                  // second 'd' press

    // Session operations
    CreateSession(AgentType),
    SelectSession(usize),
    ActivateSession(Uuid),
    RestartSession(Uuid),
    StopSession(Uuid),
    KillSession(Uuid),
    DeleteSession(Uuid),
    InitiateDeleteSession(Uuid, String),    // (id, name) - first 'd' press
    ConfirmDeleteSession,                    // second 'd' press

    // PTY interaction
    SendInput(Uuid, Vec<u8>),
    PtyOutput(Uuid, Vec<u8>),
    SessionExited(Uuid, i32),

    // UI modes
    EnterWorkspaceActionMode,    // Opens the Create/Open workspace selector
    EnterCreateWorkspaceMode,    // File browser for opening existing workspace
    EnterWorkspaceNameMode,      // Text input for naming new workspace
    EnterCreateSessionMode,
    EnterSetStartCommandMode,
    EnterHelpMode,
    ExitMode,

    // Workspace action selection
    SelectNextWorkspaceAction,
    SelectPrevWorkspaceAction,
    ConfirmWorkspaceAction,      // Confirm selected action (Create New or Open Existing)
    CreateNewWorkspace(String),  // Create new workspace with given name in current dir

    // Start command
    SetStartCommand(Uuid, String),

    // Mouse selection
    MouseDrag(u16, u16),    // (x, y) coordinates during drag
    MouseUp(u16, u16),      // (x, y) coordinates on release
    CopySelection,          // Copy selected text to clipboard
    ClearSelection,         // Clear current selection

    // Split pane / pinned terminals (up to 4)
    PinSession(Uuid),         // Pin a terminal to the workspace's pinned pane area
    UnpinSession(Uuid),       // Remove a specific terminal from pinned list
    UnpinFocusedSession,      // Remove the currently focused pinned terminal
    ToggleSplitView,          // Toggle between split and full-width view
    FocusPinnedPane(usize),   // Focus a specific pinned pane by index
    NextPinnedPane,           // Move focus to next pinned pane
    PrevPinnedPane,           // Move focus to previous pinned pane

    // Terminal creation
    CreateTerminal, // Auto-named terminal

    // Input handling
    InputChar(char),
    InputBackspace,
    InputSubmit,

    // File browser
    FileBrowserUp,
    FileBrowserDown,
    FileBrowserEnter,
    FileBrowserBack,
    FileBrowserSelect, // Select current directory as workspace

    // Utilities pane
    SelectNextUtility,
    SelectPrevUtility,
    ActivateUtility,    // Load and display utility content in output pane
    ToggleUtilitySection, // Switch between Utilities and GlobalConfig sections
    ToggleConfigItem,   // Toggle the selected config item (e.g., banner visibility)
    ToggleBrownNoise,   // Toggle brown noise player on/off

    // Notepad operations
    NotepadChar(char),      // Insert character at cursor
    NotepadBackspace,       // Delete character before cursor
    NotepadDelete,          // Delete character at cursor
    NotepadNewline,         // Insert newline
    NotepadCursorLeft,      // Move cursor left
    NotepadCursorRight,     // Move cursor right
    NotepadCursorHome,      // Move cursor to start of line
    NotepadCursorEnd,       // Move cursor to end of line
    NotepadPaste,           // Paste from clipboard
    NotepadDeleteWord,      // Delete word before cursor (Option+Backspace)
    NotepadDeleteLine,      // Delete to start of line (Cmd+Backspace)
    NotepadDeleteWordForward, // Delete word after cursor (Option+Delete)
    NotepadDeleteToEnd,     // Delete to end of line (Cmd+Delete / Ctrl+K)
    NotepadWordLeft,        // Move cursor to previous word (Option+Left)
    NotepadWordRight,       // Move cursor to next word (Option+Right)

    // Todo operations
    SelectNextTodo,
    SelectPrevTodo,
    EnterCreateTodoMode,       // Enter mode to type a new todo
    CreateTodo(String),        // Create a new todo with description
    MarkTodoDone,              // Mark selected todo as done
    RunSelectedTodo,           // Dispatch selected todo to active session
    ToggleTodoPaneMode,        // Toggle between Write and Autorun modes
    InitiateDeleteTodo(Uuid, String),  // First 'd' press on todo
    ConfirmDeleteTodo,                  // Second 'd' press

    // Auto-dispatch todos
    DispatchTodoToSession(Uuid, Uuid, String),  // (session_id, todo_id, description)
    MarkTodoReadyForReview(Uuid),               // (todo_id) - agent went idle after dispatch

    // Todo suggestion
    TriggerTodoSuggestion,                      // Spawn analyzer to suggest todos
    AddSuggestedTodo(String),                   // Add a suggested todo (from analyzer)
    ApproveSuggestedTodo(Uuid),                 // Approve suggested todo -> becomes Pending
    ApproveAllSuggestedTodos,                   // Approve all suggested todos at once
    ArchiveTodo(Uuid),                          // Archive a todo (hide from main list)
    ToggleTodosTab,                             // Switch between Active and Archived tabs

    // Mouse
    MouseClick(u16, u16), // (x, y) coordinates

    // Delete confirmation
    CancelPendingDelete,

    // App control
    Quit,
    Tick,
    Resize(u16, u16),
}
