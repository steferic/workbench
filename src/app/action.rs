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

    // Session operations
    CreateSession(AgentType),
    SelectSession(usize),
    ActivateSession(Uuid),
    RestartSession(Uuid),
    StopSession(Uuid),
    KillSession(Uuid),
    DeleteSession(Uuid),

    // PTY interaction
    SendInput(Uuid, Vec<u8>),
    PtyOutput(Uuid, Vec<u8>),
    SessionExited(Uuid, i32),

    // UI modes
    EnterCreateWorkspaceMode,
    EnterCreateSessionMode,
    EnterCreateTerminalMode,
    EnterSetStartCommandMode,
    EnterHelpMode,
    ExitMode,

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
    CreateTerminal(String), // Terminal with name

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

    // Mouse
    MouseClick(u16, u16), // (x, y) coordinates

    // App control
    Quit,
    Tick,
    Resize(u16, u16),
}
