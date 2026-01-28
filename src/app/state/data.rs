use crate::models::{Session, Workspace};
use std::collections::HashMap;
use std::time::Instant;
use tui_textarea::TextArea;
use uuid::Uuid;

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

impl Default for DataState {
    fn default() -> Self {
        Self::new()
    }
}
