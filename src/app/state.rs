use crate::models::{Session, SessionStatus, Workspace, WorkspaceStatus, MAX_PINNED_TERMINALS};
use crate::pty::PtyHandle;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use uuid::Uuid;

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
            WorkspaceAction::OpenExisting => "📂",
        }
    }
}

/// Pending delete confirmation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingDelete {
    Session(Uuid, String),    // Session ID and name for display
    Workspace(Uuid, String),  // Workspace ID and name for display
    Todo(Uuid, String),       // Todo ID and description for display
}

/// Sections in the utilities pane
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UtilitySection {
    #[default]
    Utilities,
    GlobalConfig,
    Notepad,
}

impl UtilitySection {
    pub fn toggle(&self) -> Self {
        match self {
            UtilitySection::Utilities => UtilitySection::GlobalConfig,
            UtilitySection::GlobalConfig => UtilitySection::Notepad,
            UtilitySection::Notepad => UtilitySection::Utilities,
        }
    }

    pub fn next(&self) -> Self {
        self.toggle()
    }

    pub fn prev(&self) -> Self {
        match self {
            UtilitySection::Utilities => UtilitySection::Notepad,
            UtilitySection::GlobalConfig => UtilitySection::Utilities,
            UtilitySection::Notepad => UtilitySection::GlobalConfig,
        }
    }
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

    pub fn name(&self) -> &'static str {
        match self {
            TodoPaneMode::Write => "Write",
            TodoPaneMode::Autorun => "Autorun",
        }
    }
}

/// Tab selection for the todos pane
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TodosTab {
    #[default]
    Active,
    Archived,
}

impl TodosTab {
    pub fn toggle(&self) -> Self {
        match self {
            TodosTab::Active => TodosTab::Archived,
            TodosTab::Archived => TodosTab::Active,
        }
    }
}

/// Utility items available in the utilities pane
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UtilityItem {
    #[default]
    BrownNoise,
    TopFiles,
    Calendar,
    GitHistory,
    FileTree,
    SuggestTodos,
}

impl UtilityItem {
    pub fn all() -> &'static [UtilityItem] {
        &[
            UtilityItem::BrownNoise,
            UtilityItem::TopFiles,
            UtilityItem::Calendar,
            UtilityItem::GitHistory,
            UtilityItem::FileTree,
            UtilityItem::SuggestTodos,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            UtilityItem::BrownNoise => "Brown Noise",
            UtilityItem::TopFiles => "Top Files (LOC)",
            UtilityItem::Calendar => "Calendar",
            UtilityItem::GitHistory => "Git History",
            UtilityItem::FileTree => "File Tree",
            UtilityItem::SuggestTodos => "Suggest Todos",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            UtilityItem::BrownNoise => "🔊",
            UtilityItem::TopFiles => "📊",
            UtilityItem::Calendar => "📅",
            UtilityItem::GitHistory => "📜",
            UtilityItem::FileTree => "🌳",
            UtilityItem::SuggestTodos => "💡",
        }
    }

    pub fn is_toggle(&self) -> bool {
        matches!(self, UtilityItem::BrownNoise)
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
            ConfigItem::ToggleBanner => "📢",
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

pub struct AppState {
    // Data
    pub workspaces: Vec<Workspace>,
    pub sessions: HashMap<Uuid, Vec<Session>>,

    // PTY handles (not serializable)
    pub pty_handles: HashMap<Uuid, PtyHandle>,

    // Output buffers (virtual terminal state)
    pub output_buffers: HashMap<Uuid, vt100::Parser>,

    // Activity tracking (last output time for each session)
    pub last_activity: HashMap<Uuid, Instant>,

    // Animation frame counter (for spinners)
    pub animation_frame: usize,

    // Idle session queue (sessions waiting for attention, across all workspaces)
    pub idle_queue: Vec<Uuid>,

    // UI state
    pub focus: FocusPanel,
    pub input_mode: InputMode,
    pub selected_workspace_idx: usize,
    pub selected_session_idx: usize,
    pub active_session_id: Option<Uuid>,

    // Scroll state for output pane
    pub output_scroll_offset: u16,

    // Scroll state for pinned terminal panes (per-pane)
    pub pinned_scroll_offsets: [u16; MAX_PINNED_TERMINALS],

    // Focused pinned pane index (0-3)
    pub focused_pinned_pane: usize,

    // Input buffer for dialogs
    pub input_buffer: String,

    // File browser state
    pub file_browser_path: PathBuf,
    pub file_browser_entries: Vec<PathBuf>,
    pub file_browser_selected: usize,
    pub file_browser_scroll: usize,

    // Terminal size
    pub terminal_size: (u16, u16),

    // Text selection state (for mouse-based selection in output pane)
    pub text_selection: TextSelection,

    // Text selection state for pinned terminal panes (per-pane)
    pub pinned_text_selections: [TextSelection; MAX_PINNED_TERMINALS],

    // Output pane inner area (for coordinate conversion)
    pub output_pane_area: Option<(u16, u16, u16, u16)>, // (x, y, width, height)

    // Pinned terminal pane areas (for mouse clicks, per-pane)
    pub pinned_pane_areas: [Option<(u16, u16, u16, u16)>; MAX_PINNED_TERMINALS],

    // Split view enabled (show pinned terminal alongside active session)
    pub split_view_enabled: bool,

    // Pinned pane height ratios (equal distribution by default)
    // Each ratio represents the relative height of that pane
    pub pinned_pane_ratios: [f32; MAX_PINNED_TERMINALS],

    // Resizable pane ratios (0.0 to 1.0)
    pub left_panel_ratio: f32,       // Left panel width ratio (default 0.30)
    pub output_split_ratio: f32,     // Output/pinned split ratio (default 0.50)
    pub workspace_ratio: f32,        // Workspace/lower-left split ratio (default 0.40)
    pub sessions_ratio: f32,         // Sessions portion of lower-left (default 0.40)
    pub todos_ratio: f32,            // Todos portion of remaining lower-left (default 0.50)

    // Divider dragging state
    pub dragging_divider: Option<Divider>,
    pub drag_start_pos: Option<(u16, u16)>,
    pub drag_start_ratio: f32,

    // Utilities pane state
    pub utility_section: UtilitySection,   // Which section is active (Utilities or GlobalConfig)
    pub selected_utility: UtilityItem,
    pub selected_config: ConfigItem,
    pub utility_content: Vec<String>,      // Cached content lines for display
    pub utility_scroll_offset: usize,
    pub pie_chart_data: Vec<(String, f64, ratatui::style::Color)>, // (label, value, color) for pie chart
    pub show_calendar: bool, // Whether to show the calendar widget view

    // Notepad state (per workspace)
    pub notepad_content: HashMap<Uuid, String>, // workspace_id -> notepad text
    pub notepad_cursor_pos: usize,       // Cursor position in current notepad
    pub notepad_scroll_offset: usize,    // Scroll offset for notepad view

    // Banner / marquee state
    pub banner_text: String,
    pub banner_offset: usize,
    pub banner_visible: bool,

    // Session being edited (for SetStartCommand mode)
    pub editing_session_id: Option<Uuid>,

    // Should quit flag
    pub should_quit: bool,

    // Brown noise player state
    pub brown_noise_playing: bool,

    // Analyzer session for suggesting todos
    pub analyzer_session_id: Option<Uuid>,

    // Pending delete confirmation
    pub pending_delete: Option<PendingDelete>,

    // Todos pane state
    pub selected_todo_idx: usize,
    pub todo_pane_mode: TodoPaneMode,
    pub selected_todos_tab: TodosTab,

    // Workspace action selection state
    pub selected_workspace_action: WorkspaceAction,
    // Track if we're creating new or opening existing (affects file browser behavior)
    pub workspace_create_mode: bool, // true = Create New, false = Open Existing
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

impl AppState {
    pub fn new() -> Self {
        Self {
            workspaces: Vec::new(),
            sessions: HashMap::new(),
            pty_handles: HashMap::new(),
            output_buffers: HashMap::new(),
            last_activity: HashMap::new(),
            animation_frame: 0,
            idle_queue: Vec::new(),
            focus: FocusPanel::WorkspaceList,
            input_mode: InputMode::Normal,
            selected_workspace_idx: 0,
            selected_session_idx: 0,
            active_session_id: None,
            output_scroll_offset: 0,
            pinned_scroll_offsets: [0; MAX_PINNED_TERMINALS],
            focused_pinned_pane: 0,
            input_buffer: String::new(),
            file_browser_path: dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
            file_browser_entries: Vec::new(),
            file_browser_selected: 0,
            file_browser_scroll: 0,
            terminal_size: (80, 24),
            text_selection: TextSelection::default(),
            pinned_text_selections: [TextSelection::default(); MAX_PINNED_TERMINALS],
            output_pane_area: None,
            pinned_pane_areas: [None; MAX_PINNED_TERMINALS],
            split_view_enabled: true, // Default to split view when pinned terminal exists
            pinned_pane_ratios: [0.25; MAX_PINNED_TERMINALS], // Equal distribution
            left_panel_ratio: 0.30,
            output_split_ratio: 0.50,
            workspace_ratio: 0.40,
            sessions_ratio: 0.40,
            todos_ratio: 0.50,
            dragging_divider: None,
            drag_start_pos: None,
            drag_start_ratio: 0.0,
            utility_section: UtilitySection::default(),
            selected_utility: UtilityItem::default(),
            selected_config: ConfigItem::default(),
            utility_content: Vec::new(),
            utility_scroll_offset: 0,
            pie_chart_data: Vec::new(),
            show_calendar: false,
            notepad_content: HashMap::new(),
            notepad_cursor_pos: 0,
            notepad_scroll_offset: 0,
            banner_text: "✦ WORKBENCH ✦ Multi-Agent Development Environment ✦ Claude • Gemini • Codex • Grok ✦ ".to_string(),
            banner_offset: 0,
            banner_visible: true,
            editing_session_id: None,
            should_quit: false,
            brown_noise_playing: false,
            analyzer_session_id: None,
            pending_delete: None,
            selected_todo_idx: 0,
            todo_pane_mode: TodoPaneMode::default(),
            selected_todos_tab: TodosTab::default(),
            selected_workspace_action: WorkspaceAction::default(),
            workspace_create_mode: false,
        }
    }

    /// Calculate the inner width for the output pane (for PTY sizing)
    pub fn output_pane_cols(&self) -> u16 {
        let (w, _) = self.terminal_size;
        let right_panel_width = (w as f32 * (1.0 - self.left_panel_ratio)) as u16;

        if self.should_show_split() {
            // Split between output and pinned - output gets the left portion
            let output_width = (right_panel_width as f32 * self.output_split_ratio) as u16;
            output_width.saturating_sub(2) // Account for borders
        } else {
            right_panel_width.saturating_sub(2)
        }
    }

    /// Calculate the inner width for the pinned terminal pane
    pub fn pinned_pane_cols(&self) -> u16 {
        let (w, _) = self.terminal_size;
        let right_panel_width = (w as f32 * (1.0 - self.left_panel_ratio)) as u16;

        if self.should_show_split() {
            let pinned_width = (right_panel_width as f32 * (1.0 - self.output_split_ratio)) as u16;
            pinned_width.saturating_sub(2)
        } else {
            0
        }
    }

    /// Calculate rows for PTY (accounts for borders and status bar)
    pub fn pane_rows(&self) -> u16 {
        let (_, h) = self.terminal_size;
        h.saturating_sub(4) // Status bar + top/bottom borders
    }

    pub fn refresh_file_browser(&mut self) {
        self.file_browser_entries.clear();
        self.file_browser_selected = 0;
        self.file_browser_scroll = 0;

        if let Ok(entries) = std::fs::read_dir(&self.file_browser_path) {
            let mut dirs: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .filter(|p| {
                    // Filter out hidden directories (starting with .)
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| !n.starts_with('.'))
                        .unwrap_or(false)
                })
                .collect();

            // Sort alphabetically
            dirs.sort_by(|a, b| {
                a.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_lowercase()
                    .cmp(
                        &b.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("")
                            .to_lowercase(),
                    )
            });

            self.file_browser_entries = dirs;
        }
    }

    pub fn file_browser_enter_selected(&mut self) {
        if let Some(path) = self.file_browser_entries.get(self.file_browser_selected).cloned() {
            self.file_browser_path = path;
            self.refresh_file_browser();
        }
    }

    pub fn file_browser_go_up(&mut self) {
        if let Some(parent) = self.file_browser_path.parent() {
            self.file_browser_path = parent.to_path_buf();
            self.refresh_file_browser();
        }
    }

    pub fn selected_workspace(&self) -> Option<&Workspace> {
        self.workspaces.get(self.selected_workspace_idx)
    }

    pub fn selected_workspace_mut(&mut self) -> Option<&mut Workspace> {
        self.workspaces.get_mut(self.selected_workspace_idx)
    }

    /// Returns workspace indices in visual order (Working first, then Paused)
    pub fn workspace_visual_order(&self) -> Vec<usize> {
        let mut working: Vec<usize> = self.workspaces.iter()
            .enumerate()
            .filter(|(_, ws)| ws.status == WorkspaceStatus::Working)
            .map(|(i, _)| i)
            .collect();

        let paused: Vec<usize> = self.workspaces.iter()
            .enumerate()
            .filter(|(_, ws)| ws.status == WorkspaceStatus::Paused)
            .map(|(i, _)| i)
            .collect();

        working.extend(paused);
        working
    }

    /// Navigate to previous workspace in visual order
    pub fn select_prev_workspace(&mut self) {
        let visual_order = self.workspace_visual_order();
        if visual_order.is_empty() {
            return;
        }

        // Find current position in visual order
        if let Some(pos) = visual_order.iter().position(|&idx| idx == self.selected_workspace_idx) {
            if pos > 0 {
                self.selected_workspace_idx = visual_order[pos - 1];
                self.selected_session_idx = 0;
            }
        }
    }

    /// Navigate to next workspace in visual order
    pub fn select_next_workspace(&mut self) {
        let visual_order = self.workspace_visual_order();
        if visual_order.is_empty() {
            return;
        }

        // Find current position in visual order
        if let Some(pos) = visual_order.iter().position(|&idx| idx == self.selected_workspace_idx) {
            if pos < visual_order.len() - 1 {
                self.selected_workspace_idx = visual_order[pos + 1];
                self.selected_session_idx = 0;
            }
        }
    }

    /// Returns session indices in visual order (Agents first, then Terminals)
    pub fn session_visual_order(&self) -> Vec<usize> {
        let sessions = self.sessions_for_selected_workspace();

        let mut agents: Vec<usize> = sessions.iter()
            .enumerate()
            .filter(|(_, s)| !s.agent_type.is_terminal())
            .map(|(i, _)| i)
            .collect();

        let terminals: Vec<usize> = sessions.iter()
            .enumerate()
            .filter(|(_, s)| s.agent_type.is_terminal())
            .map(|(i, _)| i)
            .collect();

        agents.extend(terminals);
        agents
    }

    /// Navigate to previous session in visual order
    pub fn select_prev_session(&mut self) {
        let visual_order = self.session_visual_order();
        if visual_order.is_empty() {
            return;
        }

        // Find current position in visual order
        if let Some(pos) = visual_order.iter().position(|&idx| idx == self.selected_session_idx) {
            if pos > 0 {
                self.selected_session_idx = visual_order[pos - 1];
            }
        }
    }

    /// Navigate to next session in visual order
    pub fn select_next_session(&mut self) {
        let visual_order = self.session_visual_order();
        if visual_order.is_empty() {
            return;
        }

        // Find current position in visual order
        if let Some(pos) = visual_order.iter().position(|&idx| idx == self.selected_session_idx) {
            if pos < visual_order.len() - 1 {
                self.selected_session_idx = visual_order[pos + 1];
            }
        }
    }

    pub fn sessions_for_selected_workspace(&self) -> Vec<&Session> {
        self.selected_workspace()
            .and_then(|ws| self.sessions.get(&ws.id))
            .map(|s| s.iter().collect())
            .unwrap_or_default()
    }

    pub fn selected_session(&self) -> Option<&Session> {
        let sessions = self.sessions_for_selected_workspace();
        sessions.get(self.selected_session_idx).copied()
    }

    /// Check if the active session is one of the pinned terminals
    pub fn active_is_pinned(&self) -> bool {
        if let Some(active) = self.active_session_id {
            self.pinned_terminal_ids().contains(&active)
        } else {
            false
        }
    }

    /// Get active output, but return None if the active session is pinned
    /// (since pinned terminals are shown in their own pane)
    pub fn active_output(&self) -> Option<&vt100::Parser> {
        // Don't show pinned terminal in output pane when split view is active
        if self.should_show_split() && self.active_is_pinned() {
            return None;
        }
        self.active_session_id
            .and_then(|id| self.output_buffers.get(&id))
    }

    /// Get active session, but return None if the active session is pinned
    pub fn active_session(&self) -> Option<&Session> {
        // Don't show pinned terminal in output pane when split view is active
        if self.should_show_split() && self.active_is_pinned() {
            return None;
        }
        self.active_session_id.and_then(|id| {
            self.sessions
                .values()
                .flatten()
                .find(|s| s.id == id)
        })
    }

    /// Get all pinned terminal IDs for the current workspace
    pub fn pinned_terminal_ids(&self) -> Vec<Uuid> {
        self.selected_workspace()
            .map(|ws| ws.pinned_terminal_ids.clone())
            .unwrap_or_default()
    }

    /// Get the number of pinned terminals
    pub fn pinned_count(&self) -> usize {
        self.selected_workspace()
            .map(|ws| ws.pinned_terminal_ids.len())
            .unwrap_or(0)
    }

    /// Get pinned terminal ID at a specific index
    pub fn pinned_terminal_id_at(&self, index: usize) -> Option<Uuid> {
        self.selected_workspace()
            .and_then(|ws| ws.pinned_terminal_ids.get(index).copied())
    }

    /// Get the pinned terminal's output buffer at a specific index
    pub fn pinned_terminal_output_at(&self, index: usize) -> Option<&vt100::Parser> {
        self.pinned_terminal_id_at(index)
            .and_then(|id| self.output_buffers.get(&id))
    }

    /// Get the pinned terminal session at a specific index
    pub fn pinned_terminal_session_at(&self, index: usize) -> Option<&Session> {
        self.pinned_terminal_id_at(index).and_then(|id| {
            self.sessions
                .values()
                .flatten()
                .find(|s| s.id == id)
        })
    }

    /// Get scroll offset for a specific pinned pane
    pub fn pinned_scroll_offset(&self, index: usize) -> u16 {
        self.pinned_scroll_offsets.get(index).copied().unwrap_or(0)
    }

    /// Get text selection for a specific pinned pane
    pub fn pinned_text_selection(&self, index: usize) -> Option<&TextSelection> {
        self.pinned_text_selections.get(index)
    }

    /// Get mutable text selection for a specific pinned pane
    pub fn pinned_text_selection_mut(&mut self, index: usize) -> Option<&mut TextSelection> {
        self.pinned_text_selections.get_mut(index)
    }

    /// Check if the focus is on any pinned pane
    pub fn is_focused_on_pinned(&self) -> bool {
        matches!(self.focus, FocusPanel::PinnedTerminalPane(_))
    }

    /// Get focused pinned pane index if focused on pinned pane
    pub fn focused_pinned_index(&self) -> Option<usize> {
        match self.focus {
            FocusPanel::PinnedTerminalPane(idx) => Some(idx),
            _ => None,
        }
    }

    /// Get the pinned terminal's output buffer (for the focused pane)
    pub fn pinned_terminal_output(&self) -> Option<&vt100::Parser> {
        self.pinned_terminal_output_at(self.focused_pinned_pane)
    }

    /// Get the pinned terminal session (for the focused pane)
    pub fn pinned_terminal_session(&self) -> Option<&Session> {
        self.pinned_terminal_session_at(self.focused_pinned_pane)
    }

    /// Check if we should show split view (has at least one pinned terminal and split is enabled)
    pub fn should_show_split(&self) -> bool {
        self.split_view_enabled && self.pinned_count() > 0
    }

    /// Calculate normalized ratios for the current number of pinned panes
    /// Returns ratios that sum to 1.0
    pub fn normalized_pinned_ratios(&self) -> Vec<f32> {
        let count = self.pinned_count();
        if count == 0 {
            return vec![];
        }

        let ratios: Vec<f32> = self.pinned_pane_ratios.iter().take(count).copied().collect();
        let sum: f32 = ratios.iter().sum();

        if sum <= 0.0 {
            // Fallback to equal distribution
            vec![1.0 / count as f32; count]
        } else {
            ratios.iter().map(|r| r / sum).collect()
        }
    }

    pub fn add_workspace(&mut self, workspace: Workspace) {
        self.workspaces.push(workspace);
    }

    pub fn add_session(&mut self, session: Session) {
        let workspace_id = session.workspace_id;
        self.sessions
            .entry(workspace_id)
            .or_insert_with(Vec::new)
            .push(session);
    }

    pub fn get_session_mut(&mut self, session_id: Uuid) -> Option<&mut Session> {
        self.sessions
            .values_mut()
            .flatten()
            .find(|s| s.id == session_id)
    }

    /// Get the workspace ID that contains a session
    pub fn workspace_id_for_session(&self, session_id: Uuid) -> Option<Uuid> {
        self.sessions.iter()
            .find_map(|(ws_id, sessions)| {
                if sessions.iter().any(|s| s.id == session_id) {
                    Some(*ws_id)
                } else {
                    None
                }
            })
    }

    /// Get mutable reference to workspace by ID
    pub fn get_workspace_mut(&mut self, workspace_id: Uuid) -> Option<&mut Workspace> {
        self.workspaces.iter_mut().find(|ws| ws.id == workspace_id)
    }

    /// Get reference to workspace by ID
    pub fn get_workspace(&self, workspace_id: Uuid) -> Option<&Workspace> {
        self.workspaces.iter().find(|ws| ws.id == workspace_id)
    }

    pub fn delete_session(&mut self, session_id: Uuid) {
        for sessions in self.sessions.values_mut() {
            sessions.retain(|s| s.id != session_id);
        }
        // Clear active session if it was deleted
        if self.active_session_id == Some(session_id) {
            self.active_session_id = None;
        }
        // Unpin session if it was pinned
        if let Some(ws) = self.selected_workspace_mut() {
            ws.unpin_terminal(session_id);
        }
        // Remove output buffer
        self.output_buffers.remove(&session_id);
        // Remove PTY handle if exists
        self.pty_handles.remove(&session_id);
        // Remove activity tracking
        self.last_activity.remove(&session_id);
    }

    /// Check if a session is actively working (received output within last 2 seconds)
    pub fn is_session_working(&self, session_id: Uuid) -> bool {
        if let Some(last) = self.last_activity.get(&session_id) {
            last.elapsed().as_secs_f32() < 2.0
        } else {
            false
        }
    }

    /// Get spinner character for animation
    pub fn spinner_char(&self) -> &'static str {
        const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        SPINNER_FRAMES[self.animation_frame % SPINNER_FRAMES.len()]
    }

    /// Advance animation frame
    pub fn tick_animation(&mut self) {
        self.animation_frame = self.animation_frame.wrapping_add(1);

        // Scroll banner every 3 frames for smooth but not too fast scrolling
        if self.animation_frame % 3 == 0 {
            let text_len = self.banner_text.chars().count();
            if text_len > 0 {
                self.banner_offset = (self.banner_offset + 1) % text_len;
            }
        }
    }

    /// Update idle queue based on current session states
    /// Only includes sessions from "Working" workspaces
    /// Returns IDs of sessions that just became idle (new to the queue)
    pub fn update_idle_queue(&mut self) -> Vec<Uuid> {
        use crate::models::SessionStatus;

        // Get IDs of "Working" workspaces only
        let working_workspace_ids: Vec<Uuid> = self.workspaces.iter()
            .filter(|ws| ws.status == WorkspaceStatus::Working)
            .map(|ws| ws.id)
            .collect();

        // Get all running AGENT sessions from WORKING workspaces (exclude terminals)
        let running_agent_sessions: Vec<Uuid> = self.sessions.iter()
            .filter(|(ws_id, _)| working_workspace_ids.contains(ws_id))
            .flat_map(|(_, sessions)| sessions)
            .filter(|s| s.status == SessionStatus::Running && s.agent_type.is_agent())
            .map(|s| s.id)
            .collect();

        // Check which sessions are currently working (to avoid borrow issues)
        let working_sessions: Vec<Uuid> = running_agent_sessions.iter()
            .filter(|id| self.is_session_working(**id))
            .copied()
            .collect();

        // Remove sessions that are no longer running or are now working
        self.idle_queue.retain(|id| {
            running_agent_sessions.contains(id) && !working_sessions.contains(id)
        });

        // Track which sessions are newly idle
        let mut newly_idle = Vec::new();

        // Add newly idle sessions (running but not working, not already in queue)
        // Note: Active session CAN be idle - we need it for todo dispatch
        for session_id in running_agent_sessions {
            if !working_sessions.contains(&session_id)
                && !self.idle_queue.contains(&session_id)
            {
                self.idle_queue.push(session_id);
                newly_idle.push(session_id);
            }
        }

        newly_idle
    }

    /// Get the next idle session from the queue
    pub fn pop_next_idle(&mut self) -> Option<Uuid> {
        if self.idle_queue.is_empty() {
            None
        } else {
            Some(self.idle_queue.remove(0))
        }
    }

    /// Remove a session from the idle queue (e.g., when activated)
    pub fn remove_from_idle_queue(&mut self, session_id: Uuid) {
        self.idle_queue.retain(|id| *id != session_id);
    }

    /// Get count of idle sessions in queue
    pub fn idle_queue_count(&self) -> usize {
        self.idle_queue.len()
    }

    pub fn running_session_count(&self) -> usize {
        self.sessions
            .values()
            .flatten()
            .filter(|s| s.status == SessionStatus::Running)
            .count()
    }

    pub fn workspace_session_count(&self, workspace_id: Uuid) -> usize {
        self.sessions
            .get(&workspace_id)
            .map(|s| s.len())
            .unwrap_or(0)
    }

    pub fn workspace_running_count(&self, workspace_id: Uuid) -> usize {
        self.sessions
            .get(&workspace_id)
            .map(|sessions| {
                sessions
                    .iter()
                    .filter(|s| s.status == SessionStatus::Running)
                    .count()
            })
            .unwrap_or(0)
    }

    /// Check if any agent in a workspace is actively working
    pub fn is_workspace_working(&self, workspace_id: Uuid) -> bool {
        self.sessions
            .get(&workspace_id)
            .map(|sessions| {
                sessions
                    .iter()
                    .filter(|s| !s.agent_type.is_terminal()) // Only check agents, not terminals
                    .any(|s| self.is_session_working(s.id))
            })
            .unwrap_or(false)
    }

    /// Get notepad content for the current workspace
    pub fn current_notepad(&self) -> &str {
        self.selected_workspace()
            .and_then(|ws| self.notepad_content.get(&ws.id))
            .map(|s| s.as_str())
            .unwrap_or("")
    }

    /// Get mutable notepad content for the current workspace (creates if missing)
    pub fn current_notepad_mut(&mut self) -> Option<&mut String> {
        let ws_id = self.selected_workspace().map(|ws| ws.id)?;
        Some(self.notepad_content.entry(ws_id).or_insert_with(String::new))
    }

    /// Insert a character at the cursor position in the notepad
    pub fn notepad_insert_char(&mut self, c: char) {
        let ws_id = match self.selected_workspace() {
            Some(ws) => ws.id,
            None => return,
        };
        let content = self.notepad_content.entry(ws_id).or_insert_with(String::new);
        // Clamp cursor position to valid range for this workspace's content
        let cursor_pos = self.notepad_cursor_pos.min(content.len());
        content.insert(cursor_pos, c);
        self.notepad_cursor_pos = cursor_pos + c.len_utf8();
    }

    /// Delete character before cursor (backspace)
    pub fn notepad_backspace(&mut self) {
        let ws_id = match self.selected_workspace() {
            Some(ws) => ws.id,
            None => return,
        };
        let content = self.notepad_content.entry(ws_id).or_insert_with(String::new);
        // Clamp cursor position to valid range
        let cursor_pos = self.notepad_cursor_pos.min(content.len());
        if cursor_pos == 0 {
            return;
        }
        // Find the previous character boundary
        let mut new_pos = cursor_pos - 1;
        while new_pos > 0 && !content.is_char_boundary(new_pos) {
            new_pos -= 1;
        }
        content.remove(new_pos);
        self.notepad_cursor_pos = new_pos;
    }

    /// Delete character at cursor (delete key)
    pub fn notepad_delete(&mut self) {
        let ws_id = match self.selected_workspace() {
            Some(ws) => ws.id,
            None => return,
        };
        let content = self.notepad_content.entry(ws_id).or_insert_with(String::new);
        // Clamp cursor position to valid range
        let cursor_pos = self.notepad_cursor_pos.min(content.len());
        self.notepad_cursor_pos = cursor_pos;
        if cursor_pos < content.len() {
            content.remove(cursor_pos);
        }
    }

    /// Move cursor left
    pub fn notepad_cursor_left(&mut self) {
        let content_bytes: Vec<u8> = self.current_notepad().bytes().collect();
        if self.notepad_cursor_pos > 0 {
            let mut new_pos = self.notepad_cursor_pos - 1;
            // Find char boundary (check if byte is not a continuation byte)
            while new_pos > 0 && (content_bytes.get(new_pos).map(|b| b & 0xC0 == 0x80).unwrap_or(false)) {
                new_pos -= 1;
            }
            self.notepad_cursor_pos = new_pos;
        }
    }

    /// Move cursor right
    pub fn notepad_cursor_right(&mut self) {
        let content_len = self.current_notepad().len();
        let content_bytes: Vec<u8> = self.current_notepad().bytes().collect();
        if self.notepad_cursor_pos < content_len {
            let mut new_pos = self.notepad_cursor_pos + 1;
            // Find char boundary
            while new_pos < content_len && (content_bytes.get(new_pos).map(|b| b & 0xC0 == 0x80).unwrap_or(false)) {
                new_pos += 1;
            }
            self.notepad_cursor_pos = new_pos;
        }
    }

    /// Move cursor to start of line
    pub fn notepad_cursor_home(&mut self) {
        let content = self.current_notepad().to_string();
        let cursor_pos = self.notepad_cursor_pos.min(content.len());
        // Find the start of the current line
        let before_cursor = &content[..cursor_pos];
        if let Some(newline_pos) = before_cursor.rfind('\n') {
            self.notepad_cursor_pos = newline_pos + 1;
        } else {
            self.notepad_cursor_pos = 0;
        }
    }

    /// Move cursor to end of line
    pub fn notepad_cursor_end(&mut self) {
        let content = self.current_notepad().to_string();
        let cursor_pos = self.notepad_cursor_pos.min(content.len());
        // Find the end of the current line
        let after_cursor = &content[cursor_pos..];
        if let Some(newline_pos) = after_cursor.find('\n') {
            self.notepad_cursor_pos = cursor_pos + newline_pos;
        } else {
            self.notepad_cursor_pos = content.len();
        }
    }

    /// Reset notepad cursor when switching workspaces
    pub fn reset_notepad_cursor(&mut self) {
        self.notepad_cursor_pos = self.current_notepad().len();
        self.notepad_scroll_offset = 0;
    }

    /// Delete word before cursor (Option+Backspace)
    pub fn notepad_delete_word(&mut self) {
        let ws_id = match self.selected_workspace() {
            Some(ws) => ws.id,
            None => return,
        };
        let content = self.notepad_content.entry(ws_id).or_insert_with(String::new);
        let cursor_pos = self.notepad_cursor_pos.min(content.len());
        if cursor_pos == 0 {
            return;
        }

        // Find start of word (skip whitespace then skip word chars)
        let before = &content[..cursor_pos];
        let mut new_pos = cursor_pos;

        // Skip trailing whitespace
        for (i, c) in before.char_indices().rev() {
            if !c.is_whitespace() {
                new_pos = i + c.len_utf8();
                break;
            }
            new_pos = i;
        }

        // Skip word characters
        let before_word = &content[..new_pos];
        for (i, c) in before_word.char_indices().rev() {
            if c.is_whitespace() {
                new_pos = i + c.len_utf8();
                break;
            }
            new_pos = i;
            if i == 0 {
                new_pos = 0;
            }
        }

        // Remove the word
        content.replace_range(new_pos..cursor_pos, "");
        self.notepad_cursor_pos = new_pos;
    }

    /// Delete to start of line (Cmd+Backspace)
    pub fn notepad_delete_line(&mut self) {
        let ws_id = match self.selected_workspace() {
            Some(ws) => ws.id,
            None => return,
        };
        let content = self.notepad_content.entry(ws_id).or_insert_with(String::new);
        let cursor_pos = self.notepad_cursor_pos.min(content.len());

        // Find the start of the current line
        let before_cursor = &content[..cursor_pos];
        let line_start = before_cursor.rfind('\n')
            .map(|pos| pos + 1)
            .unwrap_or(0);

        // Remove from line start to cursor
        content.replace_range(line_start..cursor_pos, "");
        self.notepad_cursor_pos = line_start;
    }

    /// Delete word after cursor (Option+Delete)
    pub fn notepad_delete_word_forward(&mut self) {
        let ws_id = match self.selected_workspace() {
            Some(ws) => ws.id,
            None => return,
        };
        let content = self.notepad_content.entry(ws_id).or_insert_with(String::new);
        let cursor_pos = self.notepad_cursor_pos.min(content.len());
        if cursor_pos >= content.len() {
            return;
        }

        // Find end of word (skip word chars then skip whitespace)
        let after = &content[cursor_pos..];
        let mut delete_len = 0;

        // Skip word characters first
        let mut chars = after.chars().peekable();
        while let Some(c) = chars.peek() {
            if c.is_whitespace() {
                break;
            }
            delete_len += c.len_utf8();
            chars.next();
        }

        // Then skip whitespace
        while let Some(c) = chars.peek() {
            if !c.is_whitespace() {
                break;
            }
            delete_len += c.len_utf8();
            chars.next();
        }

        // Remove the word
        content.replace_range(cursor_pos..(cursor_pos + delete_len), "");
        // Cursor position stays the same
    }

    /// Delete to end of line (Cmd+Delete or Ctrl+K)
    pub fn notepad_delete_to_end(&mut self) {
        let ws_id = match self.selected_workspace() {
            Some(ws) => ws.id,
            None => return,
        };
        let content = self.notepad_content.entry(ws_id).or_insert_with(String::new);
        let cursor_pos = self.notepad_cursor_pos.min(content.len());

        // Find the end of the current line
        let after_cursor = &content[cursor_pos..];
        let line_end = after_cursor.find('\n')
            .map(|pos| cursor_pos + pos)
            .unwrap_or(content.len());

        // Remove from cursor to line end
        content.replace_range(cursor_pos..line_end, "");
        // Cursor position stays the same
    }

    /// Move cursor to previous word (Option+Left)
    pub fn notepad_word_left(&mut self) {
        let content = self.current_notepad().to_string();
        let cursor_pos = self.notepad_cursor_pos.min(content.len());
        if cursor_pos == 0 {
            return;
        }

        let before = &content[..cursor_pos];
        let mut new_pos = cursor_pos;

        // Skip trailing whitespace
        for (i, c) in before.char_indices().rev() {
            if !c.is_whitespace() {
                new_pos = i + c.len_utf8();
                break;
            }
            new_pos = i;
        }

        // Skip word characters to find start of word
        let before_word = &content[..new_pos];
        for (i, c) in before_word.char_indices().rev() {
            if c.is_whitespace() {
                new_pos = i + c.len_utf8();
                break;
            }
            new_pos = i;
            if i == 0 {
                new_pos = 0;
            }
        }

        self.notepad_cursor_pos = new_pos;
    }

    /// Move cursor to next word (Option+Right)
    pub fn notepad_word_right(&mut self) {
        let content = self.current_notepad().to_string();
        let cursor_pos = self.notepad_cursor_pos.min(content.len());
        if cursor_pos >= content.len() {
            return;
        }

        let after = &content[cursor_pos..];
        let mut move_len = 0;

        // Skip word characters first
        let mut chars = after.chars().peekable();
        while let Some(c) = chars.peek() {
            if c.is_whitespace() {
                break;
            }
            move_len += c.len_utf8();
            chars.next();
        }

        // Then skip whitespace
        while let Some(c) = chars.peek() {
            if !c.is_whitespace() {
                break;
            }
            move_len += c.len_utf8();
            chars.next();
        }

        self.notepad_cursor_pos = cursor_pos + move_len;
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
