use crate::app::state::ConfigTab;
use crate::app::state::TaskEdit;
use crate::app::state::ToastLevel;
use crate::git::DiffStat;
use crate::models::AgentType;
use crossterm::event::KeyEvent;
use ratatui::style::Color;
use std::collections::HashMap;
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

/// Result of a background session-worktree merge (see `session_worktree.rs`).
#[derive(Debug, Clone)]
pub enum WorktreeMergeOutcome {
    Merged { worktree_removed: bool },
    WorkspaceDirty,
    CommitFailed,
    MergeFailed,
}

#[derive(Debug, Clone)]
pub struct ParallelMergePlan {
    pub workspace_path: PathBuf,
    pub workspace_id: Uuid,
    pub task_id: Uuid,
    pub winner_attempt_id: Uuid,
    pub source_branch: String,
    pub winner_branch: String,
    pub winner_worktree_path: PathBuf,
    pub session_ids: Vec<Uuid>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Action {
    // Navigation
    MoveUp,
    MoveDown,
    FocusLeft,
    FocusRight,
    ScrollOutputUp,
    ScrollOutputDown,
    MouseScrollUp(u16, u16),   // (x, y) coordinates
    MouseScrollDown(u16, u16), // (x, y) coordinates
    CycleNextWorkspace,        // F7 - cycle forward through workspaces
    CyclePrevWorkspace,        // F6 - cycle backward through workspaces
    CycleNextSession,          // F9 - cycle forward through sessions
    CyclePrevSession,          // F8 - cycle backward through sessions

    // Workspace operations
    /// Open the selected repository in the local infinite-canvas file map.
    OpenRepositoryMap,
    InitiateDeleteWorkspace(Uuid, String), // (id, name) - first 'd' press
    ConfirmDeleteWorkspace,                // second 'd' press

    // Session operations
    CreateSession(AgentType, bool, bool), // (agent_type, dangerously_skip_permissions, with_worktree)
    /// Same, but in a named workspace — used when the request did not come
    /// from the cursor (the phone).
    CreateSessionIn(Uuid, AgentType, bool, bool),
    ActivateSession(Uuid),
    RestartSession(Uuid),
    StopSession(Uuid),
    KillSession(Uuid),
    InitiateDeleteSession(Uuid, String), // (id, name) - first 'd' press
    ConfirmDeleteSession,                // second 'd' press
    MergeSessionWorktree(Uuid),          // Merge session's worktree branch into main
    SwitchToWorktree(Option<Uuid>),      // Switch to session's worktree (None = back to main)
    ConfirmMergeWithCommit,              // Commit changes and merge to main
    CancelMerge,                         // Cancel the merge modal
    // Background git results (work runs on blocking threads, never the event loop)
    SessionWorktreeMergeChecked {
        session_id: Uuid,
        has_changes: bool,
        workspace_clean: bool,
    },
    SessionWorktreeMergeFinished {
        session_id: Uuid,
        committed: bool,
        outcome: WorktreeMergeOutcome,
    },
    SessionWorktreeCreated {
        workspace_id: Uuid,
        session_id: Uuid,
        agent_type: AgentType,
        dangerously_skip_permissions: bool,
        worktree: Option<(PathBuf, String)>, // (worktree_path, branch); None = run in workspace
        failed: bool,                        // worktree creation failed (warn) vs. skipped
    },

    // PTY interaction
    SendInput(Uuid, Vec<u8>),
    Paste(String),
    PtyOutput(Uuid, Vec<u8>),
    SessionExited(Uuid, i32),

    // UI modes
    EnterWorkspaceActionMode, // Opens the Create/Open workspace selector
    EnterWorkspaceNameMode,   // Text input for naming new workspace
    EnterCreateSessionMode,
    /// Choose which CLI a new manager runs.
    EnterCreateManagerMode,
    EnterSetStartCommandMode,
    ExitMode,

    // Workspace action selection
    NextWorkspaceChoice,
    PrevWorkspaceChoice,
    ConfirmWorkspaceChoice, // Confirm selected action (Create New or Open Existing)
    CreateNewWorkspace(String), // Create new workspace with given name in current dir

    // Start command
    SetStartCommand(Uuid, String),

    // Mouse selection
    MouseDrag(u16, u16), // (x, y) coordinates during drag
    MouseUp(u16, u16),   // (x, y) coordinates on release
    CopySelection,       // Copy selected text to clipboard
    ClearSelection,      // Clear current selection

    // Split pane / pinned terminals (up to 4)
    PinSession(Uuid),    // Pin a terminal to the workspace's pinned pane area
    UnpinSession(Uuid),  // Remove a specific terminal from pinned list
    UnpinFocusedSession, // Remove the currently focused pinned terminal
    ToggleSplitView,     // Toggle between split and full-width view
    NextPinnedPane,      // Move focus to next pinned pane
    PrevPinnedPane,      // Move focus to previous pinned pane

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
    ActivateUtility,      // Open a utility or apply the selected theme
    ToggleUtilitySection, // Switch between the utility pane tabs
    ToggleBrownNoise,     // Toggle brown noise player on/off
    ToggleClassicalRadio, // Toggle WRTI classical radio stream on/off
    ToggleOceanWaves,     // Toggle ocean waves sound on/off
    ToggleWindChimes,     // Toggle wind chimes sound on/off
    ToggleRainforestRain, // Toggle rainforest rain sound on/off
    UtilityContentLoaded(UtilityContentPayload),

    // Notepad operations (tui-textarea handles all editing)
    NotepadInput(KeyEvent), // Pass key event to TextArea widget

    // Tasks pane: a live mirror of each agent's own task list
    SelectNextTask,
    SelectPrevTask,
    ToggleTasksTab,         // Switch between Tasks and Reports
    FocusSelectedTaskAgent, // Jump to the selected agent's terminal
    /// Start composing a queued item (new, or a rewrite of the selected one).
    EnterTaskEditMode(TaskEdit),
    /// Commit the composed text to the queue.
    SendTaskMessage(String),
    /// Remove the selected item from the queue.
    DeleteSelectedTodo,
    /// Move the selected item one place earlier (-1) or later (+1).
    MoveSelectedTodo(isize),
    /// Hold the queue, or let it run again.
    ToggleTodoQueuePaused,

    // Objectives — the project's standing priorities (see models::objective).
    /// Start composing a new objective, or rewrite the selected one.
    EditObjective(bool),
    /// Remove the selected objective.
    DeleteObjective,
    /// Cycle the selected objective: active → held → met → active.
    CycleObjectiveState,
    /// Move the selected objective up (-1) or down (+1) in priority.
    MoveObjective(isize),
    /// Turn the selected proposal into queued work for the agent it names.
    ApproveProposal,
    /// Say no to the selected proposal.
    DeclineProposal,
    /// Drop the items that have already run.
    ClearCompletedTodos,
    /// Off-thread scan for listening dev servers finished.
    PortsScanned(Vec<crate::ports::DevServer>),
    /// Durable scrollback parsed from an agent's session log (off-thread).
    ScrollbackLoaded {
        session_id: Uuid,
        lines: Vec<crate::app::TranscriptLine>,
        log_size: u64,
        cols: u16,
    },
    /// Off-thread re-read of the agent session logs finished.
    AgentTasksRefreshed(HashMap<Uuid, crate::agent_tasks::TaskTracker>),

    // Parallel task operations
    EnterParallelTaskMode,          // Open parallel task modal (P key)
    ToggleParallelAgent(usize),     // Toggle agent selection in modal
    NextParallelAgent,              // Move to next agent in selection
    PrevParallelAgent,              // Move to previous agent in selection
    StartParallelTask,              // Confirm and start the parallel task
    CancelParallelTask(Uuid),       // Cancel a running parallel task
    ParallelAttemptCompleted(Uuid), // An agent finished its attempt
    ParallelWorktreesReady {
        request_id: u64,
        task_id: Uuid,
        workspace_id: Uuid,
        prompt: String,
        request_report: bool,
        dangerously_skip_permissions: bool,
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
    ViewReport,           // View full report in output pane
    MergeSelectedReport,  // Merge winner from reports tab
    ConfirmParallelMerge, // Confirm parallel merge after seeing uncommitted changes
    CancelParallelMerge,  // Cancel parallel merge

    // Mouse
    MouseClick(u16, u16), // (x, y) coordinates

    // Delete confirmation
    CancelPendingDelete,

    // Quit confirmation
    InitiateQuit, // First Esc/q press - show confirmation
    ConfirmQuit,  // Second Esc/q press - actually quit
    CancelQuit,   // Any other key - cancel quit

    // App control
    Quit,
    Tick,
    Resize(u16, u16),

    // Diff stats
    DiffStatsUpdated(HashMap<PathBuf, DiffStat>),

    // Debug
    ToggleDebugOverlay, // F11 - show terminal dimension debug info

    // Repaint every cell on the next draw. Recovers from an emulator-level
    // screen clear (e.g. Ghostty's Cmd+K on the primary screen), which wipes
    // cells ratatui still believes are painted.
    ForceRedraw,

    // Config window
    EnterConfigWindow,
    ExitConfigWindow,
    ConfigSwitchTab(ConfigTab),
    ConfigMoveUp,
    ConfigMoveDown,
    ConfigMoveLeft,
    ConfigMoveRight,
    ConfigStartEdit,
    ConfigFinishEdit,
    ConfigCancelEdit,
    ConfigAddAgent,
    ConfigDeleteAgent,
    ConfigReorderUp,
    ConfigReorderDown,
    ConfigResetDefault,
    ConfigInputChar(char),
    ConfigInputBackspace,
    ConfigRebindKey(KeyEvent),

    // Command palette
    EnterCommandPalette,
    ExitCommandPalette,
    CommandPaletteExecute,
    CommandPaletteDown,
    CommandPaletteUp,
    CommandPaletteInput(char),
    CommandPaletteBackspace,

    // Toast notifications
    ShowToast(String, ToastLevel),
    TestToast,
}
