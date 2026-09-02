use crate::models::{AgentType, Workspace, MAX_PINNED_TERMINALS};
use std::path::PathBuf;
use uuid::Uuid;

use std::collections::VecDeque;

use super::types::{
    ConfigTab, Divider, FocusPanel, InputMode, PendingDelete, TaskEdit, TasksTab,
    TextSelection, Toast, UtilityItem, UtilitySection, WorkspaceAction,
};

/// Per-pinned-pane runtime UI state. Lives inside `WorkspaceUiState` and is
/// index-aligned with `Workspace.pinned_terminal_ids`. Pin/unpin must mutate
/// both Vecs in lockstep — go through `AppState::pin_terminal_for_selected` /
/// `unpin_terminal_for_selected` to keep the invariant.
#[derive(Default, Debug, Clone)]
pub struct PinnedPaneState {
    pub scroll_offset: u16,
    pub text_selection: TextSelection,
    pub on_replay: bool,
    pub content_length: usize,
}

/// Per-workspace ephemeral UI state. Switching workspaces no longer wipes
/// these values — each workspace keeps its own scroll positions, selection,
/// focused pane, etc. Nothing here is persisted; on app restart, only
/// `Workspace.last_active_session_id` is reapplied via `for_workspace`.
#[derive(Default, Debug)]
pub struct WorkspaceUiState {
    pub selected_session_idx: usize,
    /// Which kind of session the pane is listing. Per-workspace, because the
    /// cursor is: a tab that changed under you when you switched project
    /// would leave the two disagreeing about which sessions exist.
    pub sessions_tab: super::types::SessionsTab,
    pub active_session_id: Option<Uuid>,
    pub focused_pinned_pane: usize,
    pub output_scroll_offset: u16,
    pub output_on_replay: bool,
    pub output_content_length: usize,
    /// Selection in the OUTPUT pane only. Pinned-pane selections live on
    /// the per-pane `PinnedPaneState`.
    pub text_selection: TextSelection,
    pub drag_mouse_pos: Option<(u16, u16)>,
    /// Index-aligned with `Workspace.pinned_terminal_ids`. Length must always
    /// match — see pin/unpin helpers.
    pub pinned_panes: Vec<PinnedPaneState>,
}

impl WorkspaceUiState {
    /// Construct fresh state for a workspace, seeding `active_session_id`
    /// from the workspace's persisted `last_active_session_id` so the
    /// "remember last session" behavior survives lazy initialization.
    pub fn for_workspace(ws: &Workspace) -> Self {
        Self {
            active_session_id: ws.last_active_session_id,
            pinned_panes: vec![PinnedPaneState::default(); ws.pinned_terminal_ids.len()],
            ..Self::default()
        }
    }
}

/// Fuzzy file browser modal state (used when adding a workspace by path).
#[derive(Debug)]
pub struct FileBrowserState {
    pub path: PathBuf,
    pub all_entries: Vec<PathBuf>,
    pub entries: Vec<PathBuf>,
    pub selected: usize,
    pub scroll: usize,
    pub query: String,
}

impl Default for FileBrowserState {
    fn default() -> Self {
        Self {
            path: dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
            all_entries: Vec::new(),
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
            query: String::new(),
        }
    }
}

/// Config window (settings UI) navigation + editing state.
#[derive(Default, Debug)]
pub struct ConfigWindowState {
    pub tab: ConfigTab,
    pub selected_row: usize,
    pub selected_col: usize,
    pub editing: bool,
    pub edit_buffer: String,
    pub rebinding: bool,
    pub scroll_offset: usize,
}

/// Command palette overlay state.
#[derive(Default, Debug)]
pub struct CommandPaletteState {
    pub query: String,
    pub selected: usize,
    pub pending_action: Option<crate::app::Action>,
}

/// "Run parallel task" modal state: the prompt being composed, agent selection,
/// and the report-tab selection that shares this feature.
#[derive(Debug)]
pub struct ParallelTaskModalState {
    pub prompt: String,
    pub agents: Vec<(AgentType, bool)>, // Agent type and whether selected
    pub agent_idx: usize,               // Currently focused agent in selection
    pub request_report: bool,           // Whether to request PARALLEL_REPORT.md
    pub dangerous_mode: bool,           // Whether to skip permission prompts
    pub selected_report_idx: usize,     // Selected report in Reports tab
    pub request_id: u64,
}

impl Default for ParallelTaskModalState {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            agents: vec![
                (AgentType::Claude, true),
                (AgentType::Codex, true),
                (AgentType::Gemini, true),
                (AgentType::Grok, false),
            ],
            agent_idx: 0,
            request_report: true, // Default to requesting reports
            dangerous_mode: true, // Default to dangerous mode for parallel tasks
            selected_report_idx: 0,
            request_id: 0,
        }
    }
}

/// Pane layout: split ratios and the active divider drag.
#[derive(Debug)]
pub struct LayoutState {
    pub split_view_enabled: bool,
    pub pinned_pane_ratios: [f32; MAX_PINNED_TERMINALS],
    pub left_panel_ratio: f32,
    pub output_split_ratio: f32,
    pub workspace_ratio: f32,
    pub sessions_ratio: f32,
    pub tasks_ratio: f32,
    pub dragging_divider: Option<Divider>,
    pub drag_start_pos: Option<(u16, u16)>,
    pub drag_start_ratio: f32,
}

impl Default for LayoutState {
    fn default() -> Self {
        Self {
            split_view_enabled: true,
            pinned_pane_ratios: [0.25; MAX_PINNED_TERMINALS],
            left_panel_ratio: 0.30,
            output_split_ratio: 0.50,
            workspace_ratio: 0.40,
            sessions_ratio: 0.40,
            tasks_ratio: 0.50,
            dragging_divider: None,
            drag_start_pos: None,
            drag_start_ratio: 0.0,
        }
    }
}

pub struct UIState {
    pub focus: FocusPanel,
    pub input_mode: InputMode,
    pub selected_workspace_idx: usize,

    // NOTE: All per-workspace view state (active/selected session, scroll
    // offsets, text selections, pinned-pane state, drag tracking) lives in
    // `WorkspaceUiState` — access it via the `AppState` accessors
    // (`active_session_id()`, `pinned_scroll_offset(idx)`, …). Only state
    // that is genuinely global to the app belongs here.

    // Dialog & Input
    pub input_buffer: String,
    pub pending_delete: Option<PendingDelete>,
    pub pending_quit: bool, // First Esc/q press - waiting for confirmation

    // File browser modal
    pub file_browser: FileBrowserState,

    // Rendered pane rectangles (layout, identical for every workspace)
    pub output_pane_area: Option<(u16, u16, u16, u16)>,
    pub pinned_pane_areas: [Option<(u16, u16, u16, u16)>; MAX_PINNED_TERMINALS],
    pub workspace_area: Option<(u16, u16, u16, u16)>,
    pub session_area: Option<(u16, u16, u16, u16)>,
    pub tasks_area: Option<(u16, u16, u16, u16)>,
    pub utilities_area: Option<(u16, u16, u16, u16)>,

    // Pane layout
    pub layout: LayoutState,

    // Utilities pane
    pub utility_section: UtilitySection,
    pub selected_utility: UtilityItem, // For Utilities section (tools)
    pub selected_theme: crate::theme::ThemeMode,
    pub selected_sound: UtilityItem,   // For Sounds section
    pub utility_content: Vec<String>,
    pub utility_scroll_offset: usize,
    pub pie_chart_data: Vec<(String, f64, ratatui::style::Color)>,
    pub show_calendar: bool,
    /// Served URL and its terminal-native QR rows while the Phone QR utility is open.
    pub phone_qr: Option<(String, Vec<String>)>,
    pub utility_request_id: u64,

    // Banner
    pub banner_text: String,
    pub banner_offset: usize,
    pub banner_visible: bool,

    // Active UI theme
    pub theme_mode: crate::theme::ThemeMode,

    // Contextual IDs
    pub editing_session_id: Option<Uuid>,
    pub merging_session_id: Option<Uuid>, // Session being merged (for ConfirmMergeWorktree modal)
    pub merging_parallel_attempt_id: Option<Uuid>, // Parallel attempt being merged

    // Tasks pane: selection is an index into the flattened row list built by
    // `app::tasks_view::rows` (agent headers, prompts and tasks interleaved).
    pub selected_task_row: usize,
    pub selected_tasks_tab: TasksTab,
    /// Agent whose tasks are currently on screen. When the Sessions pane
    /// cursor moves to a different agent the row selection is stale, so it
    /// resets (see `handlers::tasks::sync_selection`).
    pub tasks_agent: Option<Uuid>,
    /// What the message being composed in `ComposeTaskMessage` will do, and
    /// which agent receives it.
    pub task_edit: Option<(Uuid, TaskEdit, String)>,
    /// Short-lived feedback shown in the tasks pane footer (toasts are not
    /// rendered, and this is where the user is already looking).
    pub task_status: Option<(String, std::time::Instant)>,

    // Workspace action selection
    pub selected_workspace_action: WorkspaceAction,
    pub workspace_create_mode: bool,

    // Parallel task modal
    pub parallel_task: ParallelTaskModalState,

    /// Which objective is being written, when one is. Separate from
    /// `task_edit` because an objective belongs to the project rather than to
    /// a session; `Some((workspace, None))` is a new one.
    pub objective_edit: Option<(Uuid, Option<Uuid>)>,
    /// Cursor within the Objectives tab.
    pub selected_objective: usize,
    /// Cursor in the Managers tab.
    pub selected_manager: usize,
    /// Cursor on the Desk tab.
    pub selected_desk_row: usize,
    /// A proposal or objective opened for reading in full — the context a
    /// decision deserves, without traveling to it.
    pub detail: Option<crate::app::DetailTarget>,
    /// An approval waiting on the user to pick who does the work: the
    /// proposal named no agent, and "approve" is not allowed to mean nothing.
    pub assign: Option<(uuid::Uuid, uuid::Uuid)>,
    /// How far `j` has read into an objectives-tab row taller than the pane.
    /// Written by the keys, consumed by the renderer's scroll.
    pub objective_scroll: u16,
    /// How many lines of the selected row are still below the fold. Written
    /// by the renderer each frame, read by `j` to decide between reading
    /// further into the row and moving to the next one.
    pub objective_overflow: u16,

    // Debug overlay (F11)
    pub show_debug_overlay: bool,

    // Config window
    pub config: ConfigWindowState,

    // Command palette
    pub palette: CommandPaletteState,

    // Toast notifications
    pub toasts: VecDeque<Toast>,
}

impl UIState {
    pub fn new() -> Self {
        Self {
            focus: FocusPanel::WorkspaceList,
            input_mode: InputMode::Normal,
            selected_workspace_idx: 0,
            input_buffer: String::new(),
            pending_delete: None,
            pending_quit: false,
            file_browser: FileBrowserState::default(),
            output_pane_area: None,
            pinned_pane_areas: [None; MAX_PINNED_TERMINALS],
            workspace_area: None,
            session_area: None,
            tasks_area: None,
            utilities_area: None,
            layout: LayoutState::default(),
            utility_section: UtilitySection::default(),
            selected_utility: UtilityItem::default(),
            selected_theme: crate::theme::ThemeMode::default(),
            selected_sound: UtilityItem::BrownNoise,  // Default to first sound
            utility_content: Vec::new(),
            utility_scroll_offset: 0,
            pie_chart_data: Vec::new(),
            show_calendar: false,
            phone_qr: None,
            utility_request_id: 0,
            banner_text: "\u{2726} WORKBENCH \u{2726} Multi-Agent Development Environment \u{2726} Claude \u{2022} Gemini \u{2022} Codex \u{2022} Grok \u{2726} ".to_string(),
            banner_offset: 0,
            banner_visible: true,
            theme_mode: crate::theme::ThemeMode::default(),
            editing_session_id: None,
            merging_session_id: None,
            merging_parallel_attempt_id: None,
            selected_task_row: 0,
            selected_tasks_tab: TasksTab::default(),
            tasks_agent: None,
            task_edit: None,
            task_status: None,
            selected_workspace_action: WorkspaceAction::default(),
            workspace_create_mode: false,
            parallel_task: ParallelTaskModalState::default(),
            objective_edit: None,
            selected_objective: 0,
            selected_manager: 0,
            selected_desk_row: 0,
            detail: None,
            assign: None,
            objective_scroll: 0,
            objective_overflow: 0,
            show_debug_overlay: false,
            config: ConfigWindowState::default(),
            palette: CommandPaletteState::default(),
            toasts: VecDeque::new(),
        }
    }
}

impl UIState {
    /// Show a message in the tasks pane footer for a few seconds.
    pub fn set_task_status(&mut self, message: impl Into<String>) {
        self.task_status = Some((message.into(), std::time::Instant::now()));
    }

    /// The footer message, if it hasn't aged out.
    pub fn task_status(&self) -> Option<&str> {
        self.task_status.as_ref().and_then(|(msg, at)| {
            (at.elapsed() < std::time::Duration::from_secs(4)).then_some(msg.as_str())
        })
    }
}

impl Default for UIState {
    fn default() -> Self {
        Self::new()
    }
}
