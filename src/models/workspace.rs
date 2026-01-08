use super::todo::Todo;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Maximum number of pinned terminals per workspace
pub const MAX_PINNED_TERMINALS: usize = 4;

/// Workspace status for organizing active vs paused projects
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum WorkspaceStatus {
    #[default]
    Working,
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: Uuid,
    pub name: String,
    pub path: PathBuf,
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Pinned terminal session IDs - shown stacked in the pinned pane
    #[serde(default)]
    pub pinned_terminal_ids: Vec<Uuid>,
    /// Whether this workspace is actively being worked on
    #[serde(default)]
    pub status: WorkspaceStatus,
    /// Last time this workspace had activity (session created, input sent, etc.)
    #[serde(default)]
    pub last_active_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Todo items for this workspace
    #[serde(default)]
    pub todos: Vec<Todo>,
}

impl Workspace {
    pub fn new(name: String, path: PathBuf) -> Self {
        let now = chrono::Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            path,
            created_at: now,
            pinned_terminal_ids: Vec::new(),
            status: WorkspaceStatus::default(),
            last_active_at: Some(now),
            todos: Vec::new(),
        }
    }

    /// Toggle between Working and Paused status
    pub fn toggle_status(&mut self) {
        self.status = match self.status {
            WorkspaceStatus::Working => WorkspaceStatus::Paused,
            WorkspaceStatus::Paused => WorkspaceStatus::Working,
        };
    }

    /// Update last_active_at to now
    pub fn touch(&mut self) {
        self.last_active_at = Some(chrono::Utc::now());
    }

    /// Format last_active_at as a human-readable relative time string
    pub fn last_active_display(&self) -> String {
        match self.last_active_at {
            Some(ts) => {
                let now = chrono::Utc::now();
                let duration = now.signed_duration_since(ts);

                if duration.num_seconds() < 60 {
                    "just now".to_string()
                } else if duration.num_minutes() < 60 {
                    let mins = duration.num_minutes();
                    format!("{}m ago", mins)
                } else if duration.num_hours() < 24 {
                    let hours = duration.num_hours();
                    format!("{}h ago", hours)
                } else if duration.num_days() == 1 {
                    "yesterday".to_string()
                } else if duration.num_days() < 7 {
                    format!("{}d ago", duration.num_days())
                } else if duration.num_weeks() < 4 {
                    let weeks = duration.num_weeks();
                    if weeks == 1 {
                        "1w ago".to_string()
                    } else {
                        format!("{}w ago", weeks)
                    }
                } else {
                    // Show month/day for older items
                    ts.format("%b %d").to_string()
                }
            }
            None => "never".to_string(),
        }
    }

    /// Add a terminal to the pinned list (up to MAX_PINNED_TERMINALS)
    pub fn pin_terminal(&mut self, session_id: Uuid) -> bool {
        if self.pinned_terminal_ids.len() >= MAX_PINNED_TERMINALS {
            return false;
        }
        if !self.pinned_terminal_ids.contains(&session_id) {
            self.pinned_terminal_ids.push(session_id);
        }
        true
    }

    /// Remove a terminal from the pinned list
    pub fn unpin_terminal(&mut self, session_id: Uuid) {
        self.pinned_terminal_ids.retain(|id| *id != session_id);
    }

    /// Check if a terminal is pinned
    pub fn is_pinned(&self, session_id: Uuid) -> bool {
        self.pinned_terminal_ids.contains(&session_id)
    }

    pub fn from_path(path: PathBuf) -> Self {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        Self::new(name, path)
    }

    pub fn display_path(&self) -> String {
        self.path
            .to_str()
            .map(|s| {
                if let Some(home) = dirs::home_dir() {
                    if let Some(home_str) = home.to_str() {
                        if s.starts_with(home_str) {
                            return format!("~{}", &s[home_str.len()..]);
                        }
                    }
                }
                s.to_string()
            })
            .unwrap_or_else(|| self.path.display().to_string())
    }

    // ============ Todo Management ============

    /// Add a new todo item
    pub fn add_todo(&mut self, description: impl Into<String>) -> Uuid {
        let todo = Todo::new(description);
        let id = todo.id;
        self.todos.push(todo);
        id
    }

    /// Add a suggested todo item (from analyzer)
    pub fn add_suggested_todo(&mut self, description: impl Into<String>) -> Uuid {
        let todo = Todo::suggested(description);
        let id = todo.id;
        self.todos.push(todo);
        id
    }

    /// Remove a todo by ID
    pub fn remove_todo(&mut self, todo_id: Uuid) -> bool {
        let len_before = self.todos.len();
        self.todos.retain(|t| t.id != todo_id);
        self.todos.len() < len_before
    }

    /// Get a todo by ID
    pub fn get_todo(&self, todo_id: Uuid) -> Option<&Todo> {
        self.todos.iter().find(|t| t.id == todo_id)
    }

    /// Get a mutable todo by ID
    pub fn get_todo_mut(&mut self, todo_id: Uuid) -> Option<&mut Todo> {
        self.todos.iter_mut().find(|t| t.id == todo_id)
    }

    /// Get the next dispatchable todo (Queued first, then Pending)
    pub fn next_pending_todo(&self) -> Option<&Todo> {
        // Queued todos take priority
        self.todos.iter().find(|t| t.is_queued())
            .or_else(|| self.todos.iter().find(|t| t.is_pending()))
    }

    /// Get the next dispatchable todo mutably
    pub fn next_pending_todo_mut(&mut self) -> Option<&mut Todo> {
        // Check for queued first
        if self.todos.iter().any(|t| t.is_queued()) {
            self.todos.iter_mut().find(|t| t.is_queued())
        } else {
            self.todos.iter_mut().find(|t| t.is_pending())
        }
    }

    /// Check if there's an in-progress todo in this workspace
    pub fn has_in_progress_todo(&self) -> bool {
        self.todos.iter().any(|t| t.is_in_progress())
    }

    /// Count todos by status
    pub fn pending_todo_count(&self) -> usize {
        self.todos.iter().filter(|t| t.is_pending()).count()
    }

    pub fn in_progress_todo_count(&self) -> usize {
        self.todos.iter().filter(|t| t.is_in_progress()).count()
    }

    pub fn review_todo_count(&self) -> usize {
        self.todos.iter().filter(|t| t.is_ready_for_review()).count()
    }

    /// Get the IN-PROGRESS todo for a session (not ReadyForReview or Done)
    pub fn todo_for_session(&self, session_id: Uuid) -> Option<&Todo> {
        self.todos.iter().find(|t| {
            t.is_in_progress() && t.assigned_session_id() == Some(session_id)
        })
    }

    /// Get mutable IN-PROGRESS todo for a session
    pub fn todo_for_session_mut(&mut self, session_id: Uuid) -> Option<&mut Todo> {
        self.todos.iter_mut().find(|t| {
            t.is_in_progress() && t.assigned_session_id() == Some(session_id)
        })
    }
}
