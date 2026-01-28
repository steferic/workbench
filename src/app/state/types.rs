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
    Keybindings,
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
            UtilityItem::Calendar,
            UtilityItem::GitHistory,
            UtilityItem::FileTree,
            UtilityItem::SuggestTodos,
            UtilityItem::Keybindings,
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
            UtilityItem::Calendar => "Calendar",
            UtilityItem::GitHistory => "Git History",
            UtilityItem::FileTree => "File Tree",
            UtilityItem::SuggestTodos => "Suggest Todos",
            UtilityItem::Keybindings => "Keybindings",
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
            UtilityItem::Calendar => "\u{1F4C5}",
            UtilityItem::GitHistory => "\u{1F4DC}",
            UtilityItem::FileTree => "\u{1F333}",
            UtilityItem::SuggestTodos => "\u{1F4A1}",
            UtilityItem::Keybindings => "\u{2328}",
            UtilityItem::ToggleBanner => "\u{1F4E2}",
        }
    }
}

/// Global config items (now just used for internal tracking, tree handles display)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConfigItem {
    #[default]
    ClaudeConfig,
    GeminiConfig,
}

impl ConfigItem {
    #[allow(dead_code)]
    pub fn all() -> &'static [ConfigItem] {
        &[
            ConfigItem::ClaudeConfig,
            ConfigItem::GeminiConfig,
        ]
    }

    #[allow(dead_code)]
    pub fn name(&self) -> &'static str {
        match self {
            ConfigItem::ClaudeConfig => "Claude Config",
            ConfigItem::GeminiConfig => "Gemini Config",
        }
    }

    #[allow(dead_code)]
    pub fn icon(&self) -> &'static str {
        match self {
            ConfigItem::ClaudeConfig => "\u{1F4DD}",
            ConfigItem::GeminiConfig => "\u{2728}",
        }
    }
}

/// A node in the config file tree
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ConfigTreeNode {
    /// Root item (Claude, Gemini, etc.)
    Root { name: String, path: std::path::PathBuf, expanded: bool },
    /// A directory that can be expanded (unused - keeping for potential tree view)
    Directory { name: String, path: std::path::PathBuf, expanded: bool, depth: usize },
    /// A file that can be opened (unused - keeping for potential tree view)
    File { name: String, path: std::path::PathBuf, depth: usize },
}

impl ConfigTreeNode {
    pub fn name(&self) -> &str {
        match self {
            ConfigTreeNode::Root { name, .. } => name,
            ConfigTreeNode::Directory { name, .. } => name,
            ConfigTreeNode::File { name, .. } => name,
        }
    }

    pub fn path(&self) -> &std::path::Path {
        match self {
            ConfigTreeNode::Root { path, .. } => path,
            ConfigTreeNode::Directory { path, .. } => path,
            ConfigTreeNode::File { path, .. } => path,
        }
    }

    #[allow(dead_code)]
    pub fn depth(&self) -> usize {
        match self {
            ConfigTreeNode::Root { .. } => 0,
            ConfigTreeNode::Directory { depth, .. } => *depth,
            ConfigTreeNode::File { depth, .. } => *depth,
        }
    }

    #[allow(dead_code)]
    pub fn is_expanded(&self) -> bool {
        match self {
            ConfigTreeNode::Root { expanded, .. } => *expanded,
            ConfigTreeNode::Directory { expanded, .. } => *expanded,
            ConfigTreeNode::File { .. } => false,
        }
    }

    #[allow(dead_code)]
    pub fn is_expandable(&self) -> bool {
        matches!(self, ConfigTreeNode::Root { .. } | ConfigTreeNode::Directory { .. })
    }

    #[allow(dead_code)]
    pub fn is_file(&self) -> bool {
        matches!(self, ConfigTreeNode::File { .. })
    }

    pub fn icon(&self) -> &'static str {
        match self {
            ConfigTreeNode::Root { expanded: true, .. } => "\u{1F4C2}", // Open folder
            ConfigTreeNode::Root { expanded: false, .. } => "\u{1F4C1}", // Closed folder
            ConfigTreeNode::Directory { expanded: true, .. } => "\u{1F4C2}",
            ConfigTreeNode::Directory { expanded: false, .. } => "\u{1F4C1}",
            ConfigTreeNode::File { name, .. } => {
                if name.ends_with(".json") {
                    "\u{1F4C4}" // Document
                } else if name.ends_with(".md") {
                    "\u{1F4DD}" // Memo
                } else if name.ends_with(".toml") {
                    "\u{2699}" // Gear
                } else {
                    "\u{1F4C4}" // Document
                }
            }
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
