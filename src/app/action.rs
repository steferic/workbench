use crate::app::state::PaneHelp;
use crate::models::AgentType;
use crossterm::event::KeyEvent;
use ratatui::style::Color;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct UtilityContentPayload {
    pub request_id: u64,
    pub content: Vec<String>,
    pub pie_chart_data: Vec<(String, f64, Color)>,
    pub show_calendar: bool,
}

#[derive(Debug, Clone)]
pub struct ParallelWorktreeSpec {
    pub agent_type: AgentType,
    pub branch_name: String,
    pub worktree_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ParallelMergePlan {
    pub workspace_path: PathBuf,
    pub workspace_id: Uuid,
    pub task_id: Uuid,
    pub winner_attempt_id: Uuid,
    pub source_branch: String,
    pub winner_branch: String,
    pub session_ids: Vec<Uuid>,
    pub worktree_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub enum Action {
    // Navigation
    MoveUp,
    MoveDown,
    FocusLeft,
    FocusRight,
    ScrollOutputUp,
    ScrollOutputDown,
    CycleNextWorkspace,  // ` (backtick) - cycle through workspaces
    CycleNextSession,    // ~ (Shift+backtick) - cycle through sessions in current workspace

    // Workspace operations
    ToggleWorkspaceStatus,
    InitiateDeleteWorkspace(Uuid, String),  // (id, name) - first 'd' press
    ConfirmDeleteWorkspace,                  // second 'd' press

    // Session operations
    CreateSession(AgentType, bool, bool), // (agent_type, dangerously_skip_permissions, with_worktree)
    ActivateSession(Uuid),
    RestartSession(Uuid),
    StopSession(Uuid),
    KillSession(Uuid),
    InitiateDeleteSession(Uuid, String),    // (id, name) - first 'd' press
    ConfirmDeleteSession,                    // second 'd' press
    MergeSessionWorktree(Uuid),             // Merge session's worktree branch into main
    SwitchToWorktree(Option<Uuid>),         // Switch to session's worktree (None = back to main)
    ConfirmMergeWithCommit,                 // Commit changes and merge to main
    CancelMerge,                            // Cancel the merge modal

    // PTY interaction
    SendInput(Uuid, Vec<u8>),
    Paste(String),
    PtyOutput(Uuid, Vec<u8>),
    SessionExited(Uuid, i32),

    // UI modes
    EnterWorkspaceActionMode,    // Opens the Create/Open workspace selector
    EnterWorkspaceNameMode,      // Text input for naming new workspace
    EnterCreateSessionMode,
    EnterSetStartCommandMode,
    EnterHelpMode,
    ExitMode,

    // Pane-specific help popups
    ShowPaneHelp(PaneHelp),      // Show help popup for specific pane
    DismissPaneHelp,             // Close pane help popup

    // Workspace action selection
    NextWorkspaceChoice,
    PrevWorkspaceChoice,
    ConfirmWorkspaceChoice,      // Confirm selected action (Create New or Open Existing)
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
    NextPinnedPane,           // Move focus to next pinned pane
    PrevPinnedPane,           // Move focus to previous pinned pane

    // Terminal creation
    CreateTerminal, // Auto-named terminal

    // Input handling
    InputChar(char),
    InputBackspace,

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
    ToggleUtilitySection, // Switch between Util, Sounds, Config, and Notepad sections
    ToggleConfigItem,   // Toggle the selected config item (e.g., banner visibility)
    ToggleBrownNoise,   // Toggle brown noise player on/off
    ToggleClassicalRadio, // Toggle WRTI classical radio stream on/off
    ToggleOceanWaves,   // Toggle ocean waves sound on/off
    ToggleWindChimes,   // Toggle wind chimes sound on/off
    ToggleRainforestRain, // Toggle rainforest rain sound on/off
    UtilityContentLoaded(UtilityContentPayload),

    // Notepad operations (tui-textarea handles all editing)
    NotepadInput(KeyEvent),  // Pass key event to TextArea widget

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
    AddSuggestedTodo(String),                   // Add a suggested todo (from analyzer)
    ApproveSuggestedTodo(Uuid),                 // Approve suggested todo -> becomes Pending
    ApproveAllSuggestedTodos,                   // Approve all suggested todos at once
    ArchiveTodo(Uuid),                          // Archive a todo (hide from main list)
    ToggleTodosTab,                             // Switch between Active and Archived tabs

    // Parallel task operations
    EnterParallelTaskMode,                      // Open parallel task modal (P key)
    ToggleParallelAgent(usize),                 // Toggle agent selection in modal
    NextParallelAgent,                          // Move to next agent in selection
    PrevParallelAgent,                          // Move to previous agent in selection
    StartParallelTask,                          // Confirm and start the parallel task
    CancelParallelTask(Uuid),                   // Cancel a running parallel task
    ParallelAttemptCompleted(Uuid),             // An agent finished its attempt
    ParallelWorktreesReady {
        request_id: u64,
        task_id: Uuid,
        workspace_id: Uuid,
        prompt: String,
        request_report: bool,
        source_branch: String,
        source_commit: String,
        worktrees: Vec<ParallelWorktreeSpec>,
    },
    ParallelWorktreesFailed {
        request_id: u64,
        error: String,
    },
    ParallelMergeFinished {
        plan: ParallelMergePlan,
        error: Option<String>,
    },

    // Reports tab
    SelectNextReport,
    SelectPrevReport,
    ViewReport,                                 // View full report in output pane
    MergeSelectedReport,                        // Merge winner from reports tab

    // Mouse
    MouseClick(u16, u16), // (x, y) coordinates

    // Delete confirmation
    CancelPendingDelete,

    // Quit confirmation
    InitiateQuit,      // First Esc/q press - show confirmation
    ConfirmQuit,       // Second Esc/q press - actually quit
    CancelQuit,        // Any other key - cancel quit

    // App control
    Quit,
    Tick,
    Resize(u16, u16),
}
