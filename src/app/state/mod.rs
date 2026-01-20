mod data;
mod file_browser;
mod system;
mod types;
mod ui;

pub use data::DataState;
pub use system::{PendingSessionStart, SystemState};
pub use types::*;
pub use ui::UIState;

use crate::models::{Session, SessionStatus, Workspace, WorkspaceStatus};
use std::collections::HashMap;
use tui_textarea::TextArea;
use uuid::Uuid;

pub struct AppState {
    pub data: DataState,
    pub system: SystemState,
    pub ui: UIState,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            data: DataState::new(),
            system: SystemState::new(),
            ui: UIState::new(),
        }
    }

    /// Calculate the inner width for the output pane (for PTY sizing)
    /// Uses actual rendered area if available, otherwise calculates from ratios
    pub fn output_pane_cols(&self) -> u16 {
        // Use actual rendered area if available (more accurate due to Layout rounding)
        if let Some((_, _, width, _)) = self.ui.output_pane_area {
            return width.saturating_sub(2); // Subtract borders
        }

        // Fallback to calculated value
        let (w, _) = self.system.terminal_size;
        let right_panel_width = (w as f32 * (1.0 - self.ui.left_panel_ratio)) as u16;

        if self.should_show_split() {
            // Split between output and pinned - output gets the left portion
            let output_width = (right_panel_width as f32 * self.ui.output_split_ratio) as u16;
            output_width.saturating_sub(2) // Account for borders
        } else {
            right_panel_width.saturating_sub(2)
        }
    }

    /// Calculate the inner width for the pinned terminal pane
    pub fn pinned_pane_cols(&self) -> u16 {
        let (w, _) = self.system.terminal_size;
        let right_panel_width = (w as f32 * (1.0 - self.ui.left_panel_ratio)) as u16;

        if self.should_show_split() {
            let pinned_width = (right_panel_width as f32 * (1.0 - self.ui.output_split_ratio)) as u16;
            pinned_width.saturating_sub(2)
        } else {
            0
        }
    }

    /// Calculate rows for PTY (accounts for borders, status bar, and banner)
    /// Uses actual rendered area if available, otherwise calculates from ratios
    pub fn pane_rows(&self) -> u16 {
        // Use actual rendered area if available (more accurate due to Layout rounding)
        if let Some((_, _, _, height)) = self.ui.output_pane_area {
            return height.saturating_sub(2); // Subtract borders
        }

        // Fallback to calculated value
        let (_, h) = self.system.terminal_size;
        // Status bar (1) + pane borders (2) + banner if visible (1)
        let chrome = if self.ui.banner_visible { 4 } else { 3 };
        h.saturating_sub(chrome)
    }

    pub fn selected_workspace(&self) -> Option<&Workspace> {
        self.data.workspaces.get(self.ui.selected_workspace_idx)
    }

    pub fn selected_workspace_mut(&mut self) -> Option<&mut Workspace> {
        self.data.workspaces.get_mut(self.ui.selected_workspace_idx)
    }

    /// Returns workspace indices in visual order (Working first, then Paused)
    pub fn workspace_visual_order(&self) -> Vec<usize> {
        let mut working: Vec<usize> = self.data.workspaces.iter()
            .enumerate()
            .filter(|(_, ws)| ws.status == WorkspaceStatus::Working)
            .map(|(i, _)| i)
            .collect();

        let paused: Vec<usize> = self.data.workspaces.iter()
            .enumerate()
            .filter(|(_, ws)| ws.status == WorkspaceStatus::Paused)
            .map(|(i, _)| i)
            .collect();

        working.extend(paused);
        working
    }

    /// Navigate to previous workspace in visual order
    pub fn select_prev_workspace(&mut self) {
        let visual_order = self.workspace_visual_order();
        if visual_order.is_empty() {
            return;
        }

        // Find current position in visual order
        if let Some(pos) = visual_order.iter().position(|&idx| idx == self.ui.selected_workspace_idx) {
            if pos > 0 {
                self.ui.selected_workspace_idx = visual_order[pos - 1];
                self.ui.selected_session_idx = 0;
            }
        }
    }

    /// Navigate to next workspace in visual order
    pub fn select_next_workspace(&mut self) {
        let visual_order = self.workspace_visual_order();
        if visual_order.is_empty() {
            return;
        }

        // Find current position in visual order
        if let Some(pos) = visual_order.iter().position(|&idx| idx == self.ui.selected_workspace_idx) {
            if pos < visual_order.len() - 1 {
                self.ui.selected_workspace_idx = visual_order[pos + 1];
                self.ui.selected_session_idx = 0;
            }
        }
    }

    /// Returns session indices in visual order (Agents first, then Terminals)
    pub fn session_visual_order(&self) -> Vec<usize> {
        let sessions = self.sessions_for_selected_workspace();

        let mut agents: Vec<usize> = sessions.iter()
            .enumerate()
            .filter(|(_, s)| !s.agent_type.is_terminal())
            .map(|(i, _)| i)
            .collect();

        let terminals: Vec<usize> = sessions.iter()
            .enumerate()
            .filter(|(_, s)| s.agent_type.is_terminal())
            .map(|(i, _)| i)
            .collect();

        agents.extend(terminals);
        agents
    }

    /// Navigate to previous session in visual order
    pub fn select_prev_session(&mut self) {
        let visual_order = self.session_visual_order();
        if visual_order.is_empty() {
            return;
        }

        // Find current position in visual order
        if let Some(pos) = visual_order.iter().position(|&idx| idx == self.ui.selected_session_idx) {
            if pos > 0 {
                self.ui.selected_session_idx = visual_order[pos - 1];
            }
        }
    }

    /// Navigate to next session in visual order
    pub fn select_next_session(&mut self) {
        let visual_order = self.session_visual_order();
        if visual_order.is_empty() {
            return;
        }

        // Find current position in visual order
        if let Some(pos) = visual_order.iter().position(|&idx| idx == self.ui.selected_session_idx) {
            if pos < visual_order.len() - 1 {
                self.ui.selected_session_idx = visual_order[pos + 1];
            }
        }
    }

    pub fn sessions_for_selected_workspace(&self) -> Vec<&Session> {
        self.selected_workspace()
            .and_then(|ws| self.data.sessions.get(&ws.id))
            .map(|s| s.iter().collect())
            .unwrap_or_default()
    }

    pub fn selected_session(&self) -> Option<&Session> {
        let sessions = self.sessions_for_selected_workspace();
        sessions.get(self.ui.selected_session_idx).copied()
    }

    /// Check if the active session is one of the pinned terminals
    pub fn active_is_pinned(&self) -> bool {
        if let Some(active) = self.ui.active_session_id {
            self.pinned_terminal_ids().contains(&active)
        } else {
            false
        }
    }

    /// Get active output, but return None if the active session is pinned
    /// (since pinned terminals are shown in their own pane)
    pub fn active_output(&self) -> Option<&vt100::Parser> {
        // Don't show pinned terminal in output pane when split view is active
        if self.should_show_split() && self.active_is_pinned() {
            return None;
        }
        self.ui.active_session_id
            .and_then(|id| self.system.output_buffers.get(&id))
    }

    /// Get active session, but return None if the active session is pinned
    pub fn active_session(&self) -> Option<&Session> {
        // Don't show pinned terminal in output pane when split view is active
        if self.should_show_split() && self.active_is_pinned() {
            return None;
        }
        self.ui.active_session_id.and_then(|id| {
            self.data.sessions
                .values()
                .flatten()
                .find(|s| s.id == id)
        })
    }

    /// Get all pinned terminal IDs for the current workspace
    pub fn pinned_terminal_ids(&self) -> Vec<Uuid> {
        self.selected_workspace()
            .map(|ws| ws.pinned_terminal_ids.clone())
            .unwrap_or_default()
    }

    /// Get the number of pinned terminals
    pub fn pinned_count(&self) -> usize {
        self.selected_workspace()
            .map(|ws| ws.pinned_terminal_ids.len())
            .unwrap_or(0)
    }

    /// Get pinned terminal ID at a specific index
    pub fn pinned_terminal_id_at(&self, index: usize) -> Option<Uuid> {
        self.selected_workspace()
            .and_then(|ws| ws.pinned_terminal_ids.get(index).copied())
    }

    /// Get the pinned terminal's output buffer at a specific index
    pub fn pinned_terminal_output_at(&self, index: usize) -> Option<&vt100::Parser> {
        self.pinned_terminal_id_at(index)
            .and_then(|id| self.system.output_buffers.get(&id))
    }

    /// Get the pinned terminal session at a specific index
    pub fn pinned_terminal_session_at(&self, index: usize) -> Option<&Session> {
        self.pinned_terminal_id_at(index).and_then(|id| {
            self.data.sessions
                .values()
                .flatten()
                .find(|s| s.id == id)
        })
    }

    /// Check if we should show split view (has at least one pinned terminal and split is enabled)
    pub fn should_show_split(&self) -> bool {
        self.ui.split_view_enabled && self.pinned_count() > 0
    }

    /// Calculate normalized ratios for the current number of pinned panes
    /// Returns ratios that sum to 1.0
    pub fn normalized_pinned_ratios(&self) -> Vec<f32> {
        let count = self.pinned_count();
        if count == 0 {
            return vec![];
        }

        let ratios: Vec<f32> = self.ui.pinned_pane_ratios.iter().take(count).copied().collect();
        let sum: f32 = ratios.iter().sum();

        if sum <= 0.0 {
            // Fallback to equal distribution
            vec![1.0 / count as f32; count]
        } else {
            ratios.iter().map(|r| r / sum).collect()
        }
    }

    pub fn add_workspace(&mut self, workspace: Workspace) {
        self.data.workspaces.push(workspace);
    }

    pub fn add_session(&mut self, session: Session) {
        let workspace_id = session.workspace_id;
        self.data.sessions
            .entry(workspace_id)
            .or_default()
            .push(session);
    }

    pub fn get_session_mut(&mut self, session_id: Uuid) -> Option<&mut Session> {
        self.data.sessions
            .values_mut()
            .flatten()
            .find(|s| s.id == session_id)
    }

    /// Get the workspace ID that contains a session
    pub fn workspace_id_for_session(&self, session_id: Uuid) -> Option<Uuid> {
        self.data.sessions.iter()
            .find_map(|(ws_id, sessions)| {
                if sessions.iter().any(|s| s.id == session_id) {
                    Some(*ws_id)
                } else {
                    None
                }
            })
    }

    /// Get mutable reference to workspace by ID
    pub fn get_workspace_mut(&mut self, workspace_id: Uuid) -> Option<&mut Workspace> {
        self.data.workspaces.iter_mut().find(|ws| ws.id == workspace_id)
    }

    /// Get reference to workspace by ID
    pub fn get_workspace(&self, workspace_id: Uuid) -> Option<&Workspace> {
        self.data.workspaces.iter().find(|ws| ws.id == workspace_id)
    }

    pub fn delete_session(&mut self, session_id: Uuid) {
        for sessions in self.data.sessions.values_mut() {
            sessions.retain(|s| s.id != session_id);
        }
        // Clear active session if it was deleted
        if self.ui.active_session_id == Some(session_id) {
            self.ui.active_session_id = None;
        }
        // Unpin session if it was pinned
        if let Some(ws) = self.selected_workspace_mut() {
            ws.unpin_terminal(session_id);
        }
        // Remove output buffer
        self.system.output_buffers.remove(&session_id);
        // Remove PTY handle if exists
        self.system.pty_handles.remove(&session_id);
        // Remove activity tracking
        self.data.last_activity.remove(&session_id);
    }

    /// Check if a session is actively working (received output within last 2 seconds)
    pub fn is_session_working(&self, session_id: Uuid) -> bool {
        if let Some(last) = self.data.last_activity.get(&session_id) {
            last.elapsed().as_secs_f32() < 2.0
        } else {
            false
        }
    }

    /// Check if a workspace has sessions waiting to start in the startup queue
    pub fn is_workspace_loading(&self, workspace_id: Uuid) -> bool {
        self.system.startup_queue.iter().any(|p| p.workspace_id == workspace_id)
    }

    /// Get spinner character for animation
    pub fn spinner_char(&self) -> &'static str {
        const SPINNER_FRAMES: &[&str] = &["\u{280B}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283C}", "\u{2834}", "\u{2826}", "\u{2827}", "\u{2807}", "\u{280F}"];
        SPINNER_FRAMES[self.system.animation_frame % SPINNER_FRAMES.len()]
    }

    /// Advance animation frame
    pub fn tick_animation(&mut self) {
        self.system.animation_frame = self.system.animation_frame.wrapping_add(1);

        // Scroll banner every 3 frames for smooth but not too fast scrolling
        if self.system.animation_frame.is_multiple_of(3) {
            let text_len = self.ui.banner_text.chars().count();
            if text_len > 0 {
                self.ui.banner_offset = (self.ui.banner_offset + 1) % text_len;
            }
        }
    }

    /// Update idle queue based on current session states
    /// Only includes sessions from "Working" workspaces
    /// Returns IDs of sessions that just became idle (new to the queue)
    pub fn update_idle_queue(&mut self) -> Vec<Uuid> {
        // Get IDs of "Working" workspaces only
        let working_workspace_ids: Vec<Uuid> = self.data.workspaces.iter()
            .filter(|ws| ws.status == WorkspaceStatus::Working)
            .map(|ws| ws.id)
            .collect();

        // Get all running AGENT sessions from WORKING workspaces (exclude terminals)
        let running_agent_sessions: Vec<Uuid> = self.data.sessions.iter()
            .filter(|(ws_id, _)| working_workspace_ids.contains(ws_id))
            .flat_map(|(_, sessions)| sessions)
            .filter(|s| s.status == SessionStatus::Running && s.agent_type.is_agent())
            .map(|s| s.id)
            .collect();

        // Check which sessions are currently working (to avoid borrow issues)
        let working_sessions: Vec<Uuid> = running_agent_sessions.iter()
            .filter(|id| self.is_session_working(**id))
            .copied()
            .collect();

        // Remove sessions that are no longer running or are now working
        self.data.idle_queue.retain(|id| {
            running_agent_sessions.contains(id) && !working_sessions.contains(id)
        });

        // Track which sessions are newly idle
        let mut newly_idle = Vec::new();

        // Add newly idle sessions (running but not working, not already in queue)
        // Note: Active session CAN be idle - we need it for todo dispatch
        for session_id in running_agent_sessions {
            if !working_sessions.contains(&session_id)
                && !self.data.idle_queue.contains(&session_id)
            {
                self.data.idle_queue.push(session_id);
                newly_idle.push(session_id);
            }
        }

        newly_idle
    }

    /// Get count of idle sessions in queue
    pub fn idle_queue_count(&self) -> usize {
        self.data.idle_queue.len()
    }

    pub fn running_session_count(&self) -> usize {
        self.data.sessions
            .values()
            .flatten()
            .filter(|s| s.status == SessionStatus::Running)
            .count()
    }

    pub fn workspace_session_count(&self, workspace_id: Uuid) -> usize {
        self.data.sessions
            .get(&workspace_id)
            .map(|s| s.len())
            .unwrap_or(0)
    }

    pub fn workspace_running_count(&self, workspace_id: Uuid) -> usize {
        self.data.sessions
            .get(&workspace_id)
            .map(|sessions| {
                sessions
                    .iter()
                    .filter(|s| s.status == SessionStatus::Running)
                    .count()
            })
            .unwrap_or(0)
    }

    /// Check if any agent in a workspace is actively working
    pub fn is_workspace_working(&self, workspace_id: Uuid) -> bool {
        self.data.sessions
            .get(&workspace_id)
            .map(|sessions| {
                sessions
                    .iter()
                    .filter(|s| !s.agent_type.is_terminal()) // Only check agents, not terminals
                    .any(|s| self.is_session_working(s.id))
            })
            .unwrap_or(false)
    }

    /// Get or create the TextArea for the current workspace
    pub fn current_notepad(&mut self) -> Option<&mut TextArea<'static>> {
        let ws_id = self.selected_workspace().map(|ws| ws.id)?;
        Some(self.data.notepads.entry(ws_id).or_default())
    }

    /// Get notepad content as string for persistence
    pub fn notepad_content_for_persistence(&self) -> HashMap<Uuid, String> {
        self.data.notepads.iter()
            .map(|(id, ta)| (*id, ta.lines().join("\n")))
            .filter(|(_, content)| !content.is_empty())
            .collect()
    }

    /// Load notepad content from persisted string
    pub fn load_notepad_content(&mut self, ws_id: Uuid, content: String) {
        let lines: Vec<String> = if content.is_empty() {
            vec![]
        } else {
            content.lines().map(|s| s.to_string()).collect()
        };
        let textarea = if lines.is_empty() {
            TextArea::default()
        } else {
            TextArea::new(lines)
        };
        self.data.notepads.insert(ws_id, textarea);
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
