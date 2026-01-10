use crate::models::{AgentType, Session, SessionStatus, Workspace, WorkspaceStatus, MAX_PINNED_TERMINALS};
use crate::pty::PtyHandle;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tui_textarea::TextArea;
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
    CreateParallelTask,     // Modal for starting a parallel task
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

pub struct DataState {
    pub workspaces: Vec<Workspace>,
    pub sessions: HashMap<Uuid, Vec<Session>>,
    /// Activity tracking (last output time for each session)
    pub last_activity: HashMap<Uuid, Instant>,
    /// Idle session queue (sessions waiting for attention, across all workspaces)
    pub idle_queue: Vec<Uuid>,
    /// Notepad state (per workspace) - TextArea handles cursor, scrolling, undo/redo
    pub notepads: HashMap<Uuid, TextArea<'static>>,
}

impl DataState {
    pub fn new() -> Self {
        Self {
            workspaces: Vec::new(),
            sessions: HashMap::new(),
            last_activity: HashMap::new(),
            idle_queue: Vec::new(),
            notepads: HashMap::new(),
        }
    }
}

pub struct SystemState {
    /// PTY handles (not serializable)
    pub pty_handles: HashMap<Uuid, PtyHandle>,
    /// Output buffers (virtual terminal state)
    pub output_buffers: HashMap<Uuid, vt100::Parser>,
    /// Terminal size
    pub terminal_size: (u16, u16),
    /// Animation frame counter (for spinners)
    pub animation_frame: usize,
    /// Should quit flag
    pub should_quit: bool,
    /// Brown noise player state
    pub brown_noise_playing: bool,
}

impl SystemState {
    pub fn new() -> Self {
        Self {
            pty_handles: HashMap::new(),
            output_buffers: HashMap::new(),
            terminal_size: (80, 24),
            animation_frame: 0,
            should_quit: false,
            brown_noise_playing: false,
        }
    }
}

pub struct UIState {
    pub focus: FocusPanel,
    pub input_mode: InputMode,
    pub selected_workspace_idx: usize,
    pub selected_session_idx: usize,
    pub active_session_id: Option<Uuid>,

    // Scroll state
    pub output_scroll_offset: u16,
    pub pinned_scroll_offsets: [u16; MAX_PINNED_TERMINALS],
    pub focused_pinned_pane: usize,

    // Dialog & Input
    pub input_buffer: String,
    pub pending_delete: Option<PendingDelete>,

    // File browser state
    pub file_browser_path: PathBuf,
    pub file_browser_all_entries: Vec<PathBuf>,
    pub file_browser_entries: Vec<PathBuf>,
    pub file_browser_selected: usize,
    pub file_browser_scroll: usize,
    pub file_browser_query: String,

    // Selection & Areas
    pub text_selection: TextSelection,
    pub pinned_text_selections: [TextSelection; MAX_PINNED_TERMINALS],
    pub output_pane_area: Option<(u16, u16, u16, u16)>,
    pub pinned_pane_areas: [Option<(u16, u16, u16, u16)>; MAX_PINNED_TERMINALS],
    pub workspace_area: Option<(u16, u16, u16, u16)>,
    pub session_area: Option<(u16, u16, u16, u16)>,
    pub todos_area: Option<(u16, u16, u16, u16)>,
    pub utilities_area: Option<(u16, u16, u16, u16)>,
    pub output_content_length: usize,
    pub pinned_content_lengths: [usize; MAX_PINNED_TERMINALS],

    // Layout
    pub split_view_enabled: bool,
    pub pinned_pane_ratios: [f32; MAX_PINNED_TERMINALS],
    pub left_panel_ratio: f32,
    pub output_split_ratio: f32,
    pub workspace_ratio: f32,
    pub sessions_ratio: f32,
    pub todos_ratio: f32,
    pub dragging_divider: Option<Divider>,
    pub drag_start_pos: Option<(u16, u16)>,
    pub drag_start_ratio: f32,

    // Utilities pane
    pub utility_section: UtilitySection,
    pub selected_utility: UtilityItem,
    pub selected_config: ConfigItem,
    pub utility_content: Vec<String>,
    pub utility_scroll_offset: usize,
    pub pie_chart_data: Vec<(String, f64, ratatui::style::Color)>,
    pub show_calendar: bool,

    // Banner
    pub banner_text: String,
    pub banner_offset: usize,
    pub banner_visible: bool,

    // Contextual IDs
    pub editing_session_id: Option<Uuid>,
    pub analyzer_session_id: Option<Uuid>,

    // Todos pane
    pub selected_todo_idx: usize,
    pub todo_pane_mode: TodoPaneMode,
    pub selected_todos_tab: TodosTab,

    // Workspace action selection
    pub selected_workspace_action: WorkspaceAction,
    pub workspace_create_mode: bool,

    // Parallel task modal state
    pub parallel_task_prompt: String,
    pub parallel_task_agents: Vec<(AgentType, bool)>,  // Agent type and whether selected
    pub parallel_task_agent_idx: usize,  // Currently focused agent in selection
    pub parallel_task_request_report: bool,  // Whether to request PARALLEL_REPORT.md
    pub selected_report_idx: usize,  // Selected report in Reports tab
}

impl UIState {
    pub fn new() -> Self {
        Self {
            focus: FocusPanel::WorkspaceList,
            input_mode: InputMode::Normal,
            selected_workspace_idx: 0,
            selected_session_idx: 0,
            active_session_id: None,
            output_scroll_offset: 0,
            pinned_scroll_offsets: [0; MAX_PINNED_TERMINALS],
            focused_pinned_pane: 0,
            input_buffer: String::new(),
            pending_delete: None,
            file_browser_path: dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
            file_browser_all_entries: Vec::new(),
            file_browser_entries: Vec::new(),
            file_browser_selected: 0,
            file_browser_scroll: 0,
            file_browser_query: String::new(),
            text_selection: TextSelection::default(),
            pinned_text_selections: [TextSelection::default(); MAX_PINNED_TERMINALS],
            output_pane_area: None,
            pinned_pane_areas: [None; MAX_PINNED_TERMINALS],
            workspace_area: None,
            session_area: None,
            todos_area: None,
            utilities_area: None,
            output_content_length: 0,
            pinned_content_lengths: [0; MAX_PINNED_TERMINALS],
            split_view_enabled: true,
            pinned_pane_ratios: [0.25; MAX_PINNED_TERMINALS],
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
            banner_text: "✦ WORKBENCH ✦ Multi-Agent Development Environment ✦ Claude • Gemini • Codex • Grok ✦ ".to_string(),
            banner_offset: 0,
            banner_visible: true,
            editing_session_id: None,
            analyzer_session_id: None,
            selected_todo_idx: 0,
            todo_pane_mode: TodoPaneMode::default(),
            selected_todos_tab: TodosTab::default(),
            selected_workspace_action: WorkspaceAction::default(),
            workspace_create_mode: false,
            parallel_task_prompt: String::new(),
            parallel_task_agents: vec![
                (AgentType::Claude, true),
                (AgentType::Codex, true),
                (AgentType::Gemini, true),
                (AgentType::Grok, false),
            ],
            parallel_task_agent_idx: 0,
            parallel_task_request_report: true,  // Default to requesting reports
            selected_report_idx: 0,
        }
    }
}

pub struct AppState {
    pub data: DataState,
    pub system: SystemState,
    pub ui: UIState,
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
            data: DataState::new(),
            system: SystemState::new(),
            ui: UIState::new(),
        }
    }

    /// Calculate the inner width for the output pane (for PTY sizing)
    pub fn output_pane_cols(&self) -> u16 {
        let (w, _) = self.system.terminal_size;
        let right_panel_width = (w as f32 * (1.0 - self.ui.left_panel_ratio)) as u16;

        if self.should_show_split() {
            // Split between output and pinned - output gets the left portion
            let output_width = (right_panel_width as f32 * self.ui.output_split_ratio) as u16;
            output_width.saturating_sub(2) // Account for borders
        } else {
            right_panel_width.saturating_sub(2)
        }
    }

    /// Calculate the inner width for the pinned terminal pane
    pub fn pinned_pane_cols(&self) -> u16 {
        let (w, _) = self.system.terminal_size;
        let right_panel_width = (w as f32 * (1.0 - self.ui.left_panel_ratio)) as u16;

        if self.should_show_split() {
            let pinned_width = (right_panel_width as f32 * (1.0 - self.ui.output_split_ratio)) as u16;
            pinned_width.saturating_sub(2)
        } else {
            0
        }
    }

    /// Calculate rows for PTY (accounts for borders and status bar)
    pub fn pane_rows(&self) -> u16 {
        let (_, h) = self.system.terminal_size;
        h.saturating_sub(4) // Status bar + top/bottom borders
    }

    pub fn refresh_file_browser(&mut self) {
        self.ui.file_browser_all_entries.clear();
        self.ui.file_browser_entries.clear();
        self.ui.file_browser_selected = 0;
        self.ui.file_browser_scroll = 0;

        if let Ok(entries) = std::fs::read_dir(&self.ui.file_browser_path) {
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

            self.ui.file_browser_all_entries = dirs;
        }
        self.apply_file_browser_filter();
    }

    pub fn file_browser_enter_selected(&mut self) {
        if let Some(path) = self.ui.file_browser_entries.get(self.ui.file_browser_selected).cloned() {
            self.ui.file_browser_path = path;
            self.ui.file_browser_query.clear();
            self.refresh_file_browser();
        }
    }

    pub fn file_browser_go_up(&mut self) {
        if let Some(parent) = self.ui.file_browser_path.parent() {
            self.ui.file_browser_path = parent.to_path_buf();
            self.ui.file_browser_query.clear();
            self.refresh_file_browser();
        }
    }

    pub fn apply_file_browser_filter(&mut self) {
        let query = self.ui.file_browser_query.trim();
        if query.is_empty() {
            self.ui.file_browser_entries = self.ui.file_browser_all_entries.clone();
            self.ui.file_browser_selected = 0;
            self.ui.file_browser_scroll = 0;
            return;
        }

        let query_lower = query.to_ascii_lowercase();
        if let Some(path) = resolve_query_path(&self.ui.file_browser_path, query) {
            self.ui.file_browser_path = path;
            self.ui.file_browser_query.clear();
            self.refresh_file_browser();
            return;
        }

        let mut matches: Vec<(usize, String, PathBuf)> = Vec::new();
        let use_absolute = query.starts_with('/');

        for path in &self.ui.file_browser_all_entries {
            let candidate = if use_absolute {
                path.to_string_lossy().to_string()
            } else {
                shorten_home_path(path)
            };
            if let Some(score) = fuzzy_score(&query_lower, &candidate) {
                matches.push((score, candidate.to_ascii_lowercase(), path.clone()));
            }
        }

        matches.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        self.ui.file_browser_entries = matches.into_iter().map(|(_, _, path)| path).collect();
        self.ui.file_browser_selected = 0;
        self.ui.file_browser_scroll = 0;
    }

    pub fn selected_workspace(&self) -> Option<&Workspace> {
        self.data.workspaces.get(self.ui.selected_workspace_idx)
    }

    pub fn selected_workspace_mut(&mut self) -> Option<&mut Workspace> {
        self.data.workspaces.get_mut(self.ui.selected_workspace_idx)
    }

    /// Returns workspace indices in visual order (Working first, then Paused)
    pub fn workspace_visual_order(&self) -> Vec<usize> {
        let mut working: Vec<usize> = self.data.workspaces.iter()
            .enumerate()
            .filter(|(_, ws)| ws.status == WorkspaceStatus::Working)
            .map(|(i, _)| i)
            .collect();

        let paused: Vec<usize> = self.data.workspaces.iter()
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
        if let Some(pos) = visual_order.iter().position(|&idx| idx == self.ui.selected_workspace_idx) {
            if pos > 0 {
                self.ui.selected_workspace_idx = visual_order[pos - 1];
                self.ui.selected_session_idx = 0;
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
        if let Some(pos) = visual_order.iter().position(|&idx| idx == self.ui.selected_workspace_idx) {
            if pos < visual_order.len() - 1 {
                self.ui.selected_workspace_idx = visual_order[pos + 1];
                self.ui.selected_session_idx = 0;
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
        if let Some(pos) = visual_order.iter().position(|&idx| idx == self.ui.selected_session_idx) {
            if pos > 0 {
                self.ui.selected_session_idx = visual_order[pos - 1];
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
        if let Some(pos) = visual_order.iter().position(|&idx| idx == self.ui.selected_session_idx) {
            if pos < visual_order.len() - 1 {
                self.ui.selected_session_idx = visual_order[pos + 1];
            }
        }
    }

    pub fn sessions_for_selected_workspace(&self) -> Vec<&Session> {
        self.selected_workspace()
            .and_then(|ws| self.data.sessions.get(&ws.id))
            .map(|s| s.iter().collect())
            .unwrap_or_default()
    }

    pub fn selected_session(&self) -> Option<&Session> {
        let sessions = self.sessions_for_selected_workspace();
        sessions.get(self.ui.selected_session_idx).copied()
    }

    /// Check if the active session is one of the pinned terminals
    pub fn active_is_pinned(&self) -> bool {
        if let Some(active) = self.ui.active_session_id {
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
        self.ui.active_session_id
            .and_then(|id| self.system.output_buffers.get(&id))
    }

    /// Get active session, but return None if the active session is pinned
    pub fn active_session(&self) -> Option<&Session> {
        // Don't show pinned terminal in output pane when split view is active
        if self.should_show_split() && self.active_is_pinned() {
            return None;
        }
        self.ui.active_session_id.and_then(|id| {
            self.data.sessions
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
            .and_then(|id| self.system.output_buffers.get(&id))
    }

    /// Get the pinned terminal session at a specific index
    pub fn pinned_terminal_session_at(&self, index: usize) -> Option<&Session> {
        self.pinned_terminal_id_at(index).and_then(|id| {
            self.data.sessions
                .values()
                .flatten()
                .find(|s| s.id == id)
        })
    }

    /// Check if we should show split view (has at least one pinned terminal and split is enabled)
    pub fn should_show_split(&self) -> bool {
        self.ui.split_view_enabled && self.pinned_count() > 0
    }

    /// Calculate normalized ratios for the current number of pinned panes
    /// Returns ratios that sum to 1.0
    pub fn normalized_pinned_ratios(&self) -> Vec<f32> {
        let count = self.pinned_count();
        if count == 0 {
            return vec![];
        }

        let ratios: Vec<f32> = self.ui.pinned_pane_ratios.iter().take(count).copied().collect();
        let sum: f32 = ratios.iter().sum();

        if sum <= 0.0 {
            // Fallback to equal distribution
            vec![1.0 / count as f32; count]
        } else {
            ratios.iter().map(|r| r / sum).collect()
        }
    }

    pub fn add_workspace(&mut self, workspace: Workspace) {
        self.data.workspaces.push(workspace);
    }

    pub fn add_session(&mut self, session: Session) {
        let workspace_id = session.workspace_id;
        self.data.sessions
            .entry(workspace_id)
            .or_insert_with(Vec::new)
            .push(session);
    }

    pub fn get_session_mut(&mut self, session_id: Uuid) -> Option<&mut Session> {
        self.data.sessions
            .values_mut()
            .flatten()
            .find(|s| s.id == session_id)
    }

    /// Get the workspace ID that contains a session
    pub fn workspace_id_for_session(&self, session_id: Uuid) -> Option<Uuid> {
        self.data.sessions.iter()
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
        self.data.workspaces.iter_mut().find(|ws| ws.id == workspace_id)
    }

    /// Get reference to workspace by ID
    pub fn get_workspace(&self, workspace_id: Uuid) -> Option<&Workspace> {
        self.data.workspaces.iter().find(|ws| ws.id == workspace_id)
    }

    pub fn delete_session(&mut self, session_id: Uuid) {
        for sessions in self.data.sessions.values_mut() {
            sessions.retain(|s| s.id != session_id);
        }
        // Clear active session if it was deleted
        if self.ui.active_session_id == Some(session_id) {
            self.ui.active_session_id = None;
        }
        // Unpin session if it was pinned
        if let Some(ws) = self.selected_workspace_mut() {
            ws.unpin_terminal(session_id);
        }
        // Remove output buffer
        self.system.output_buffers.remove(&session_id);
        // Remove PTY handle if exists
        self.system.pty_handles.remove(&session_id);
        // Remove activity tracking
        self.data.last_activity.remove(&session_id);
    }

    /// Check if a session is actively working (received output within last 2 seconds)
    pub fn is_session_working(&self, session_id: Uuid) -> bool {
        if let Some(last) = self.data.last_activity.get(&session_id) {
            last.elapsed().as_secs_f32() < 2.0
        } else {
            false
        }
    }

    /// Get spinner character for animation
    pub fn spinner_char(&self) -> &'static str {
        const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        SPINNER_FRAMES[self.system.animation_frame % SPINNER_FRAMES.len()]
    }

    /// Advance animation frame
    pub fn tick_animation(&mut self) {
        self.system.animation_frame = self.system.animation_frame.wrapping_add(1);

        // Scroll banner every 3 frames for smooth but not too fast scrolling
        if self.system.animation_frame % 3 == 0 {
            let text_len = self.ui.banner_text.chars().count();
            if text_len > 0 {
                self.ui.banner_offset = (self.ui.banner_offset + 1) % text_len;
            }
        }
    }

    /// Update idle queue based on current session states
    /// Only includes sessions from "Working" workspaces
    /// Returns IDs of sessions that just became idle (new to the queue)
    pub fn update_idle_queue(&mut self) -> Vec<Uuid> {
        use crate::models::SessionStatus;

        // Get IDs of "Working" workspaces only
        let working_workspace_ids: Vec<Uuid> = self.data.workspaces.iter()
            .filter(|ws| ws.status == WorkspaceStatus::Working)
            .map(|ws| ws.id)
            .collect();

        // Get all running AGENT sessions from WORKING workspaces (exclude terminals)
        let running_agent_sessions: Vec<Uuid> = self.data.sessions.iter()
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
        self.data.idle_queue.retain(|id| {
            running_agent_sessions.contains(id) && !working_sessions.contains(id)
        });

        // Track which sessions are newly idle
        let mut newly_idle = Vec::new();

        // Add newly idle sessions (running but not working, not already in queue)
        // Note: Active session CAN be idle - we need it for todo dispatch
        for session_id in running_agent_sessions {
            if !working_sessions.contains(&session_id)
                && !self.data.idle_queue.contains(&session_id)
            {
                self.data.idle_queue.push(session_id);
                newly_idle.push(session_id);
            }
        }

        newly_idle
    }

    /// Get count of idle sessions in queue
    pub fn idle_queue_count(&self) -> usize {
        self.data.idle_queue.len()
    }

    pub fn running_session_count(&self) -> usize {
        self.data.sessions
            .values()
            .flatten()
            .filter(|s| s.status == SessionStatus::Running)
            .count()
    }

    pub fn workspace_session_count(&self, workspace_id: Uuid) -> usize {
        self.data.sessions
            .get(&workspace_id)
            .map(|s| s.len())
            .unwrap_or(0)
    }

    pub fn workspace_running_count(&self, workspace_id: Uuid) -> usize {
        self.data.sessions
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
        self.data.sessions
            .get(&workspace_id)
            .map(|sessions| {
                sessions
                    .iter()
                    .filter(|s| !s.agent_type.is_terminal()) // Only check agents, not terminals
                    .any(|s| self.is_session_working(s.id))
            })
            .unwrap_or(false)
    }

    /// Get or create the TextArea for the current workspace
    pub fn current_notepad(&mut self) -> Option<&mut TextArea<'static>> {
        let ws_id = self.selected_workspace().map(|ws| ws.id)?;
        Some(self.data.notepads.entry(ws_id).or_insert_with(TextArea::default))
    }

    /// Get notepad content as string for persistence
    pub fn notepad_content_for_persistence(&self) -> HashMap<Uuid, String> {
        self.data.notepads.iter()
            .map(|(id, ta)| (*id, ta.lines().join("\n")))
            .filter(|(_, content)| !content.is_empty())
            .collect()
    }

    /// Load notepad content from persisted string
    pub fn load_notepad_content(&mut self, ws_id: Uuid, content: String) {
        let lines: Vec<String> = if content.is_empty() {
            vec![]
        } else {
            content.lines().map(|s| s.to_string()).collect()
        };
        let textarea = if lines.is_empty() {
            TextArea::default()
        } else {
            TextArea::new(lines)
        };
        self.data.notepads.insert(ws_id, textarea);
    }
}

fn shorten_home_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let (Some(home_str), Some(path_str)) = (home.to_str(), path.to_str()) {
            if path_str.starts_with(home_str) {
                return format!("~{}", &path_str[home_str.len()..]);
            }
        }
    }
    path.to_string_lossy().to_string()
}

fn resolve_query_path(base: &Path, query: &str) -> Option<PathBuf> {
    if !is_path_like(query) {
        return None;
    }

    let expanded = if let Some(rest) = query.strip_prefix("~/") {
        dirs::home_dir().map(|home| home.join(rest))?
    } else if query.starts_with('~') {
        PathBuf::from(query)
    } else {
        PathBuf::from(query)
    };

    let candidates = if expanded.is_absolute() {
        vec![expanded]
    } else {
        let mut list = vec![base.join(&expanded)];
        if let Some(home) = dirs::home_dir() {
            list.push(home.join(&expanded));
        }
        list
    };

    for candidate in candidates {
        if candidate.exists() && candidate.is_dir() {
            return Some(candidate);
        }
    }

    None
}

fn is_path_like(query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return false;
    }
    if query.starts_with('/') {
        return true;
    }
    if query.starts_with('~') || query.starts_with('.') {
        return query.len() > 1;
    }
    query.contains('/')
}

fn fuzzy_score(query_lower: &str, candidate: &str) -> Option<usize> {
    if query_lower.is_empty() {
        return Some(0);
    }

    let candidate_lower = candidate.to_ascii_lowercase();
    let mut score = 0usize;
    let mut last_match: Option<usize> = None;
    let mut search_start = 0usize;

    for qch in query_lower.chars() {
        let mut found = None;
        for (idx, cch) in candidate_lower[search_start..].char_indices() {
            if cch == qch {
                found = Some(search_start + idx);
                break;
            }
        }

        let match_idx = found?;
        if let Some(prev) = last_match {
            score += match_idx.saturating_sub(prev + 1);
        } else {
            score += match_idx;
        }
        last_match = Some(match_idx);
        search_start = match_idx + 1;
    }

    Some(score)
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
