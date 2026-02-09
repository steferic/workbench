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
// Parser buffer rows - kept large to preserve scrollback history.
// On resize, only columns are updated (to match PTY); rows stay at this value.
pub const PARSER_BUFFER_ROWS: u16 = 500;
// Scrollback limit is the max characters stored for scrollback history
pub const TERMINAL_SCROLLBACK_LIMIT: usize = 150_000;

pub use action::{Action, ParallelMergePlan, ParallelWorktreeSpec, UtilityContentPayload};
pub use runtime::run_tui;
pub use state::{AppState, ConfigTreeNode, Divider, FocusPanel, InputMode, PaneHelp, PendingDelete, PendingSessionStart, TextSelection, TodoPaneMode, TodosTab, UtilityItem, UtilitySection, WorkspaceAction};
