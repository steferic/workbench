#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPanel {
    WorkspaceList,
    SessionList,
    TasksPane,
    UtilitiesPane,
    OutputPane,
    PinnedTerminalPane(usize), // Index of focused pinned pane (0-3)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    SelectWorkspaceAction, // Choose between Create New or Open Existing
    CreateWorkspace,       // Browse to select existing directory (Open Existing)
    EnterWorkspaceName,    // Enter name for new workspace (Create New)
    CreateSession,
    /// Composing a message that steers an agent's task list.
    ComposeTaskMessage,
    SetStartCommand,
    CreateParallelTask,   // Modal for starting a parallel task
    ConfirmMergeWorktree, // Confirm commit and merge worktree
    ConfirmParallelMerge, // Confirm commit and merge parallel task worktree
    ConfigWindow,         // F1 configuration window
    CommandPalette,       // Ctrl+P command palette
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConfigTab {
    #[default]
    QuickRef,
    Agents,
    Hotkeys,
    Scrollback,
}

/// Workspace action selection (when pressing 'n' in workspace list)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkspaceAction {
    #[default]
    CreateNew,
    OpenExisting,
}

impl WorkspaceAction {
    pub fn all() -> &'static [WorkspaceAction] {
        &[WorkspaceAction::CreateNew, WorkspaceAction::OpenExisting]
    }

    pub fn name(&self) -> &'static str {
        match self {
            WorkspaceAction::CreateNew => "Create New Project",
            WorkspaceAction::OpenExisting => "Open Existing Project",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            WorkspaceAction::CreateNew => "Create a new project directory",
            WorkspaceAction::OpenExisting => "Add an existing directory as workspace",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            WorkspaceAction::CreateNew => "+",
            WorkspaceAction::OpenExisting => "\u{1F4C2}",
        }
    }
}

/// Pending delete confirmation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingDelete {
    Session(uuid::Uuid, String),   // Session ID and name for display
    Workspace(uuid::Uuid, String), // Workspace ID and name for display
}

/// Sections in the utilities pane
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UtilitySection {
    #[default]
    Utilities,
    Themes,
    Sounds,
    Notepad,
}

impl UtilitySection {
    pub fn toggle(&self) -> Self {
        match self {
            UtilitySection::Utilities => UtilitySection::Themes,
            UtilitySection::Themes => UtilitySection::Sounds,
            UtilitySection::Sounds => UtilitySection::Notepad,
            UtilitySection::Notepad => UtilitySection::Utilities,
        }
    }
}

/// What the text being composed will do to the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskEdit {
    /// Append a new item.
    Add,
    /// Replace the selected item's text.
    Rewrite,
}

/// Tab selection for the tasks pane
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TasksTab {
    /// Live mirror of every agent's own task list.
    #[default]
    Tasks,
    /// The project's standing priorities (see `models::objective`). What a
    /// manager will eventually work toward; useful on its own before then.
    Objectives,
    /// Reports from parallel task agents.
    Reports,
}

impl TasksTab {
    /// Tab cycles rather than toggles now that there are three.
    ///
    /// Reports is slated to fold into the manager view once one exists —
    /// judging what agents did is the manager's job, and two tabs read better
    /// than three. Doing it now would mean building a merged row model for a
    /// view that is about to be rewritten, so it waits for that rewrite.
    pub fn toggle(&self) -> Self {
        match self {
            TasksTab::Tasks => TasksTab::Objectives,
            TasksTab::Objectives => TasksTab::Reports,
            TasksTab::Reports => TasksTab::Tasks,
        }
    }
}

/// Utility items available in the utilities pane
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UtilityItem {
    // Tools
    #[default]
    TopFiles,
    PromptLog,
    Calendar,
    GitHistory,
    Keybindings,
    PhoneQr,
    ToggleBanner,
    // Sounds
    BrownNoise,
    ClassicalRadio,
    OceanWaves,
    WindChimes,
    RainforestRain,
}

impl UtilityItem {
    /// Tools shown in the Util tab
    pub fn tools() -> &'static [UtilityItem] {
        &[
            UtilityItem::TopFiles,
            UtilityItem::PromptLog,
            UtilityItem::Calendar,
            UtilityItem::GitHistory,
            UtilityItem::Keybindings,
            UtilityItem::PhoneQr,
            UtilityItem::ToggleBanner,
        ]
    }

    /// Sounds shown in the Sounds tab
    pub fn sounds() -> &'static [UtilityItem] {
        &[
            UtilityItem::BrownNoise,
            UtilityItem::ClassicalRadio,
            UtilityItem::OceanWaves,
            UtilityItem::WindChimes,
            UtilityItem::RainforestRain,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            UtilityItem::BrownNoise => "Brown Noise",
            UtilityItem::ClassicalRadio => "Classical Radio",
            UtilityItem::OceanWaves => "Ocean",
            UtilityItem::WindChimes => "Chimes",
            UtilityItem::RainforestRain => "Rain",
            UtilityItem::TopFiles => "Top Files (LOC)",
            UtilityItem::PromptLog => "Prompt Log",
            UtilityItem::Calendar => "Calendar",
            UtilityItem::GitHistory => "Git History",
            UtilityItem::Keybindings => "Keybindings",
            UtilityItem::PhoneQr => "Phone QR",
            UtilityItem::ToggleBanner => "Banner Bar",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            UtilityItem::BrownNoise => "\u{1F50A}",
            UtilityItem::ClassicalRadio => "\u{1F3BB}",
            UtilityItem::OceanWaves => "\u{1F30A}",
            UtilityItem::WindChimes => "\u{1F390}",
            UtilityItem::RainforestRain => "\u{1F327}\u{FE0F}",
            UtilityItem::TopFiles => "\u{1F4CA}",
            UtilityItem::PromptLog => "\u{270E}",
            UtilityItem::Calendar => "\u{1F4C5}",
            UtilityItem::GitHistory => "\u{1F4DC}",
            UtilityItem::Keybindings => "\u{2328}",
            UtilityItem::PhoneQr => "\u{25A6}",
            UtilityItem::ToggleBanner => "\u{1F4E2}",
        }
    }
}

/// Mouse text selection state
#[derive(Debug, Clone, Copy, Default)]
pub struct TextSelection {
    /// Start position (row, col) - where mouse was pressed
    pub start: Option<(usize, usize)>,
    /// End position (row, col) - where mouse currently is or was released
    pub end: Option<(usize, usize)>,
    /// Whether we're actively dragging
    pub is_dragging: bool,
}

/// Toast notification level
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
}

/// A toast notification message. The renderer is currently disabled, but the
/// type stays so call sites that push status messages don't need to change.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub level: ToastLevel,
    pub created_at: std::time::Instant,
    pub duration: std::time::Duration,
}

impl Toast {
    pub fn new(message: String, level: ToastLevel, duration: std::time::Duration) -> Self {
        Self {
            message,
            level,
            created_at: std::time::Instant::now(),
            duration,
        }
    }

    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() >= self.duration
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Divider {
    LeftRight,          // Between left panel and right panel
    WorkspaceSession,   // Between workspace list and session list (horizontal)
    SessionsTasks,      // Between sessions and tasks in lower-left (horizontal)
    TasksUtilities,     // Between tasks and utilities in lower-left (horizontal)
    OutputPinned,       // Between output pane and pinned terminal
    PinnedPanes(usize), // Between pinned panes (index is the pane above the divider)
}
