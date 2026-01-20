use crate::models::{AgentType, MAX_PINNED_TERMINALS};
use std::path::PathBuf;
use uuid::Uuid;

use super::types::{
    ConfigItem, Divider, FocusPanel, InputMode, PaneHelp, PendingDelete,
    TextSelection, TodoPaneMode, TodosTab, UtilityItem, UtilitySection, WorkspaceAction,
};

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
    pub pending_quit: bool,  // First Esc/q press - waiting for confirmation

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
    pub drag_mouse_pos: Option<(u16, u16)>,  // Track mouse position during text selection drag for smooth scrolling
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
    pub selected_utility: UtilityItem,  // For Utilities section (tools)
    pub selected_sound: UtilityItem,    // For Sounds section
    pub selected_config: ConfigItem,
    pub utility_content: Vec<String>,
    pub utility_scroll_offset: usize,
    pub pie_chart_data: Vec<(String, f64, ratatui::style::Color)>,
    pub show_calendar: bool,
    pub utility_request_id: u64,

    // Banner
    pub banner_text: String,
    pub banner_offset: usize,
    pub banner_visible: bool,

    // Contextual IDs
    pub editing_session_id: Option<Uuid>,
    pub analyzer_session_id: Option<Uuid>,
    pub merging_session_id: Option<Uuid>,  // Session being merged (for ConfirmMergeWorktree modal)

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
    pub parallel_task_request_id: u64,

    // Pane-specific help popup
    pub pane_help: Option<PaneHelp>,

    // Debug overlay (F12)
    pub show_debug_overlay: bool,
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
            pending_quit: false,
            file_browser_path: dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
            file_browser_all_entries: Vec::new(),
            file_browser_entries: Vec::new(),
            file_browser_selected: 0,
            file_browser_scroll: 0,
            file_browser_query: String::new(),
            text_selection: TextSelection::default(),
            pinned_text_selections: [TextSelection::default(); MAX_PINNED_TERMINALS],
            drag_mouse_pos: None,
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
            selected_sound: UtilityItem::BrownNoise,  // Default to first sound
            selected_config: ConfigItem::default(),
            utility_content: Vec::new(),
            utility_scroll_offset: 0,
            pie_chart_data: Vec::new(),
            show_calendar: false,
            utility_request_id: 0,
            banner_text: "\u{2726} WORKBENCH \u{2726} Multi-Agent Development Environment \u{2726} Claude \u{2022} Gemini \u{2022} Codex \u{2022} Grok \u{2726} ".to_string(),
            banner_offset: 0,
            banner_visible: true,
            editing_session_id: None,
            analyzer_session_id: None,
            merging_session_id: None,
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
            parallel_task_request_id: 0,
            pane_help: None,
            show_debug_overlay: false,
        }
    }
}

impl Default for UIState {
    fn default() -> Self {
        Self::new()
    }
}
