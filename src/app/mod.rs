mod action;
mod handler;
pub mod handlers;
mod pty_ops;
mod runtime;
mod selection;
mod session_start;
mod state;
mod utilities;

// Terminal buffer configuration
// Live parser rows - small viewport, just enough for current screen.
// Deep scrollback is handled via raw byte replay (see tui::replay).
pub const PARSER_BUFFER_ROWS: u16 = 80;
// Note: TERMINAL_SCROLLBACK_LIMIT, REPLAY_PARSER_ROWS, and RAW_OUTPUT_BUFFER_CAPACITY
// are now configurable via UserConfig (loaded from ~/.config/workbench/user_config.toml).

pub use action::{Action, ParallelMergePlan, ParallelWorktreeSpec, UtilityContentPayload};
pub use runtime::run_tui;
pub use state::{AppState, ConfigTab, ConfigTreeNode, Divider, FocusPanel, InputMode, PendingDelete, PendingSessionStart, RawOutputBuffer, ReplayCache, TextSelection, TodoPaneMode, TodosTab, UtilityItem, UtilitySection, WorkspaceAction};
