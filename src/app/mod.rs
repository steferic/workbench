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
// Minimal scrollback for the live parser (it doesn't need much)
pub const TERMINAL_SCROLLBACK_LIMIT: usize = 200;
// Replay parser rows - used on demand when scrolling deep into history
pub const REPLAY_PARSER_ROWS: u16 = 500;
// Max raw PTY bytes stored per session for replay (512KB)
pub const RAW_OUTPUT_BUFFER_CAPACITY: usize = 512 * 1024;

pub use action::{Action, ParallelMergePlan, ParallelWorktreeSpec, UtilityContentPayload};
pub use runtime::run_tui;
pub use state::{AppState, ConfigTreeNode, Divider, FocusPanel, InputMode, PaneHelp, PendingDelete, PendingSessionStart, RawOutputBuffer, ReplayCache, TextSelection, TodoPaneMode, TodosTab, UtilityItem, UtilitySection, WorkspaceAction};
