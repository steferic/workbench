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
// Rows determines how many lines of history are kept in the vt100 parser
// Scrollback limit is the max characters stored for scrollback
pub const TERMINAL_BUFFER_ROWS: u16 = 3000;
pub const TERMINAL_SCROLLBACK_LIMIT: usize = 150_000;

pub use action::{Action, ParallelMergePlan, ParallelWorktreeSpec, UtilityContentPayload};
pub use runtime::run_tui;
pub use state::{AppState, ConfigItem, Divider, FocusPanel, InputMode, PaneHelp, PendingDelete, PendingSessionStart, TextSelection, TodoPaneMode, TodosTab, UtilityItem, UtilitySection, WorkspaceAction};
