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

pub use action::{
    Action, ParallelMergePlan, ParallelWorktreeSpec, UtilityContentPayload, WorktreeMergeOutcome,
};
pub use runtime::run_tui;
pub(crate) mod jobs;
pub(crate) mod servers;
pub use state::{
    AppState, ConfigTab, Divider, FocusPanel, InputMode, PendingDelete, PendingSessionStart,
    SessionsTab, SystemState, TaskEdit, TextSelection, ThreadCache, Toast, ToastLevel,
    TranscriptBuffer, TranscriptLine, TranscriptSpan, UtilityItem, UtilitySection, WorkspaceAction,
};

#[cfg(test)]
pub(crate) use handler::process_action;
