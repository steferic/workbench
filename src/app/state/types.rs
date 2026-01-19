#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPanel {
    WorkspaceList,
    SessionList,
    TodosPane,
    UtilitiesPane,
    OutputPane,
    PinnedTerminalPane(usize), // Index of focused pinned pane (0-3)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    SelectWorkspaceAction,  // Choose between Create New or Open Existing
    CreateWorkspace,        // Browse to select existing directory (Open Existing)
    EnterWorkspaceName,     // Enter name for new workspace (Create New)
    CreateSession,
    CreateTodo,
    SetStartCommand,
    CreateParallelTask,     // Modal for starting a parallel task
    ConfirmMergeWorktree,   // Confirm commit and merge worktree
    Help,
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
    Session(uuid::Uuid, String),    // Session ID and name for display
    Workspace(uuid::Uuid, String),  // Workspace ID and name for display
    Todo(uuid::Uuid, String),       // Todo ID and description for display
}

/// Sections in the utilities pane
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UtilitySection {
    #[default]
    Utilities,
    Sounds,
    GlobalConfig,
    Notepad,
}

impl UtilitySection {
    pub fn toggle(&self) -> Self {
        match self {
            UtilitySection::Utilities => UtilitySection::Sounds,
            UtilitySection::Sounds => UtilitySection::GlobalConfig,
            UtilitySection::GlobalConfig => UtilitySection::Notepad,
            UtilitySection::Notepad => UtilitySection::Utilities,
        }
    }
}

/// Which pane-specific help popup is showing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneHelp {
    Workspaces,
    Sessions,
    Todos,
    Utilities,
}

/// Mode for the todos pane
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TodoPaneMode {
    #[default]
    Write,   // Manual mode - create todos and run one at a time
    Autorun, // Auto-dispatch todos to idle agents sequentially
}

impl TodoPaneMode {
    pub fn toggle(&self) -> Self {
        match self {
            TodoPaneMode::Write => TodoPaneMode::Autorun,
            TodoPaneMode::Autorun => TodoPaneMode::Write,
        }
    }
}

/// Tab selection for the todos pane
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TodosTab {
    #[default]
    Active,
    Archived,
    Reports,  // Reports from parallel task agents
}

impl TodosTab {
    pub fn toggle(&self) -> Self {
        match self {
            TodosTab::Active => TodosTab::Archived,
            TodosTab::Archived => TodosTab::Reports,
            TodosTab::Reports => TodosTab::Active,
        }
    }
}

/// Utility items available in the utilities pane
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UtilityItem {
    // Tools
    #[default]
    TopFiles,
    Calendar,
    GitHistory,
    FileTree,
    SuggestTodos,
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
            UtilityItem::Calendar,
            UtilityItem::GitHistory,
            UtilityItem::FileTree,
            UtilityItem::SuggestTodos,
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
            UtilityItem::Calendar => "Calendar",
            UtilityItem::GitHistory => "Git History",
            UtilityItem::FileTree => "File Tree",
            UtilityItem::SuggestTodos => "Suggest Todos",
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
            UtilityItem::Calendar => "\u{1F4C5}",
            UtilityItem::GitHistory => "\u{1F4DC}",
            UtilityItem::FileTree => "\u{1F333}",
            UtilityItem::SuggestTodos => "\u{1F4A1}",
        }
    }
}

/// Global config items
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConfigItem {
    #[default]
    ToggleBanner,
}

impl ConfigItem {
    pub fn all() -> &'static [ConfigItem] {
        &[ConfigItem::ToggleBanner]
    }

    pub fn name(&self) -> &'static str {
        match self {
            ConfigItem::ToggleBanner => "Banner Bar",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            ConfigItem::ToggleBanner => "\u{1F4E2}",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Divider {
    LeftRight,         // Between left panel and right panel
    WorkspaceSession,  // Between workspace list and session list (horizontal)
    SessionsTodos,     // Between sessions and todos in lower-left (horizontal)
    TodosUtilities,    // Between todos and utilities in lower-left (horizontal)
    OutputPinned,      // Between output pane and pinned terminal
    PinnedPanes(usize), // Between pinned panes (index is the pane above the divider)
}
