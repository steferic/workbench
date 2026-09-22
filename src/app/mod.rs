mod action;
pub mod agent_input;
pub mod cleanup;
pub mod comms_tick;
pub mod desk_view;
mod handler;

/// What the detail overlay is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTarget {
    Proposal {
        workspace_id: uuid::Uuid,
        proposal_id: uuid::Uuid,
    },
    Objective {
        workspace_id: uuid::Uuid,
        objective_id: uuid::Uuid,
    },
}

pub mod handlers;
pub mod objectives_view;
mod pty_ops;
mod runtime;
mod selection;
mod session_start;
mod state;
pub mod tasks_view;
pub mod todo_dispatch;
mod utilities;
pub mod verify;
mod workspace_nav;

// Terminal buffer configuration
// Live parser rows - small viewport, just enough for current screen.
// Deep scrollback is handled via raw byte replay (see tui::replay).
pub const PARSER_BUFFER_ROWS: u16 = 80;
// Note: TERMINAL_SCROLLBACK_LIMIT, REPLAY_PARSER_ROWS, and RAW_OUTPUT_BUFFER_CAPACITY
// are now configurable via UserConfig (loaded from ~/.config/workbench/user_config.toml).

pub use action::{
    Action, ParallelMergePlan, ParallelWorktreeSpec, UtilityContentPayload, WorktreeMergeOutcome,
};
pub use runtime::run_tui;
pub(crate) mod jobs;
pub(crate) mod servers;
pub use state::{
    AppState, ConfigTab, Divider, FocusPanel, InputMode, PendingDelete, PendingSessionStart,
    RawOutputBuffer, ReplayCache, SessionsTab, SystemState, TaskEdit, TextSelection, ThreadCache,
    Toast, ToastLevel, TranscriptBuffer, TranscriptLine, TranscriptSpan, UtilityItem,
    UtilitySection, WorkspaceAction,
};

#[cfg(test)]
pub(crate) use handler::process_action;
