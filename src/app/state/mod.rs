mod data;
mod file_browser;
mod system;
mod types;
mod ui;

pub use data::DataState;
pub use system::{
    PendingSessionStart, RawOutputBuffer, ReplayCache, SystemState, ThreadCache, TranscriptBuffer,
    TranscriptLine, TranscriptSpan,
};
pub use types::*;
pub use ui::{PinnedPaneState, UIState, WorkspaceUiState};

use crate::agent_status::{Activity, Attention};
use crate::models::{Session, SessionStatus, Workspace, WorkspaceStatus};
use std::collections::HashMap;
use tui_textarea::TextArea;
use uuid::Uuid;

pub struct AppState {
    pub data: DataState,
    pub system: SystemState,
    pub ui: UIState,
    /// Per-workspace ephemeral UI state, keyed by workspace id. Lazily
    /// populated on first access via `ws_ui_mut`. Removed when a workspace is
    /// deleted (see `handlers/workspace.rs::ConfirmDeleteWorkspace`).
    pub ws_ui: HashMap<Uuid, WorkspaceUiState>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            data: DataState::new(),
            system: SystemState::new(),
            ui: UIState::new(),
            ws_ui: HashMap::new(),
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
        let right_panel_width = (w as f32 * (1.0 - self.ui.layout.left_panel_ratio)) as u16;

        if self.should_show_split() {
            // Split between output and pinned - output gets the left portion
            let output_width = (right_panel_width as f32 * self.ui.layout.output_split_ratio) as u16;
            output_width.saturating_sub(2) // Account for borders
        } else {
            right_panel_width.saturating_sub(2)
        }
    }

    /// Calculate the inner width for the pinned terminal pane
    pub fn pinned_pane_cols(&self) -> u16 {
        let (w, _) = self.system.terminal_size;
        let right_panel_width = (w as f32 * (1.0 - self.ui.layout.left_panel_ratio)) as u16;

        if self.should_show_split() {
            let pinned_width =
                (right_panel_width as f32 * (1.0 - self.ui.layout.output_split_ratio)) as u16;
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

    /// Read the per-workspace UI state for the currently selected workspace.
    /// `None` when no workspace is selected (e.g. on first launch with an
    /// empty workspaces list).
    pub fn ws_ui(&self) -> Option<&WorkspaceUiState> {
        let id = self.selected_workspace()?.id;
        self.ws_ui.get(&id)
    }

    /// Mutable per-workspace UI state for the currently selected workspace.
    /// Lazily seeds an entry from `WorkspaceUiState::for_workspace` if one
    /// doesn't exist yet, so `last_active_session_id` is honored on first
    /// access after process start.
    pub fn ws_ui_mut(&mut self) -> Option<&mut WorkspaceUiState> {
        let ws = self.data.workspaces.get(self.ui.selected_workspace_idx)?;
        let id = ws.id;
        let seed = WorkspaceUiState::for_workspace(ws);
        Some(self.ws_ui.entry(id).or_insert(seed))
    }

    /// Read pinned-pane state at `idx` for the selected workspace. Bounds- and
    /// selection-checked: out-of-range indices return `None` instead of
    /// panicking like the old fixed-array indexing did.
    pub fn pinned_pane(&self, idx: usize) -> Option<&PinnedPaneState> {
        self.ws_ui()?.pinned_panes.get(idx)
    }

    /// Mutable pinned-pane state at `idx` for the selected workspace.
    pub fn pinned_pane_mut(&mut self, idx: usize) -> Option<&mut PinnedPaneState> {
        self.ws_ui_mut()?.pinned_panes.get_mut(idx)
    }

    // ------------------------------------------------------------------
    // Accessors for per-workspace UI state (the single source of truth is
    // `ws_ui`). Reads fall back to `0`/`None`/`default()` when no workspace
    // is selected so call sites don't thread `Option` through every render
    // path; writes are no-ops in that case.
    // ------------------------------------------------------------------

    pub fn output_scroll_offset(&self) -> u16 {
        self.ws_ui().map(|u| u.output_scroll_offset).unwrap_or(0)
    }
    pub fn set_output_scroll_offset(&mut self, value: u16) {
        if let Some(u) = self.ws_ui_mut() {
            u.output_scroll_offset = value;
        }
    }
    pub fn output_on_replay(&self) -> bool {
        self.ws_ui().map(|u| u.output_on_replay).unwrap_or(false)
    }
    /// Whether the output pane rendered from the replay parser last frame
    /// (used to detect live→replay transitions and translate selections).
    pub fn set_output_on_replay(&mut self, value: bool) {
        if let Some(u) = self.ws_ui_mut() {
            u.output_on_replay = value;
        }
    }
    pub fn output_content_length(&self) -> usize {
        self.ws_ui().map(|u| u.output_content_length).unwrap_or(0)
    }
    pub fn set_output_content_length(&mut self, value: usize) {
        if let Some(u) = self.ws_ui_mut() {
            u.output_content_length = value;
        }
    }
    pub fn focused_pinned_pane(&self) -> usize {
        self.ws_ui().map(|u| u.focused_pinned_pane).unwrap_or(0)
    }
    pub fn set_focused_pinned_pane(&mut self, value: usize) {
        if let Some(u) = self.ws_ui_mut() {
            u.focused_pinned_pane = value;
        }
    }
    pub fn active_session_id(&self) -> Option<Uuid> {
        self.ws_ui().and_then(|u| u.active_session_id)
    }
    pub fn set_active_session_id(&mut self, value: Option<Uuid>) {
        if let Some(u) = self.ws_ui_mut() {
            u.active_session_id = value;
        }
    }
    pub fn selected_session_idx(&self) -> usize {
        self.ws_ui().map(|u| u.selected_session_idx).unwrap_or(0)
    }
    pub fn set_selected_session_idx(&mut self, value: usize) {
        if let Some(u) = self.ws_ui_mut() {
            u.selected_session_idx = value;
        }
    }
    pub fn drag_mouse_pos(&self) -> Option<(u16, u16)> {
        self.ws_ui().and_then(|u| u.drag_mouse_pos)
    }
    pub fn set_drag_mouse_pos(&mut self, value: Option<(u16, u16)>) {
        if let Some(u) = self.ws_ui_mut() {
            u.drag_mouse_pos = value;
        }
    }
    /// Output-pane text selection. Pinned-pane selections live on the
    /// per-pane `PinnedPaneState`.
    pub fn text_selection(&self) -> TextSelection {
        self.ws_ui().map(|u| u.text_selection).unwrap_or_default()
    }
    pub fn set_text_selection(&mut self, value: TextSelection) {
        if let Some(u) = self.ws_ui_mut() {
            u.text_selection = value;
        }
    }
    pub fn pinned_scroll_offset(&self, idx: usize) -> u16 {
        self.pinned_pane(idx).map(|p| p.scroll_offset).unwrap_or(0)
    }
    pub fn set_pinned_scroll_offset(&mut self, idx: usize, value: u16) {
        if let Some(p) = self.pinned_pane_mut(idx) {
            p.scroll_offset = value;
        }
    }
    pub fn pinned_content_length(&self, idx: usize) -> usize {
        self.pinned_pane(idx).map(|p| p.content_length).unwrap_or(0)
    }
    pub fn set_pinned_content_length(&mut self, idx: usize, value: usize) {
        if let Some(p) = self.pinned_pane_mut(idx) {
            p.content_length = value;
        }
    }
    pub fn pinned_on_replay(&self, idx: usize) -> bool {
        self.pinned_pane(idx).map(|p| p.on_replay).unwrap_or(false)
    }
    pub fn pinned_text_selection(&self, idx: usize) -> TextSelection {
        self.pinned_pane(idx)
            .map(|p| p.text_selection)
            .unwrap_or_default()
    }
    pub fn set_pinned_text_selection(&mut self, idx: usize, value: TextSelection) {
        if let Some(p) = self.pinned_pane_mut(idx) {
            p.text_selection = value;
        }
    }

    /// Clear `active_session_id` in every workspace's UI state that points at
    /// `session_id`. Sessions can be deleted from non-selected workspaces, so
    /// this must not be limited to the selected workspace's entry.
    pub fn clear_active_session_everywhere(&mut self, session_id: Uuid) {
        for ws_ui in self.ws_ui.values_mut() {
            if ws_ui.active_session_id == Some(session_id) {
                ws_ui.active_session_id = None;
            }
        }
    }

    /// Pin a terminal session in the currently selected workspace, keeping
    /// `Workspace.pinned_terminal_ids` and `WorkspaceUiState.pinned_panes`
    /// length-aligned. Returns `true` if the pin was added.
    pub fn pin_terminal_for_selected(&mut self, session_id: Uuid) -> bool {
        let Some(ws) = self.selected_workspace_mut() else {
            return false;
        };
        let added = ws.pin_terminal(session_id);
        if added {
            if let Some(ws_ui) = self.ws_ui_mut() {
                ws_ui.pinned_panes.push(PinnedPaneState::default());
            }
        }
        added
    }

    /// Unpin a terminal session from the workspace that owns it (not just the
    /// selected one — sessions can belong to non-selected workspaces). Removes
    /// the matching `PinnedPaneState` so the Vecs stay index-aligned.
    pub fn unpin_terminal_anywhere(&mut self, session_id: Uuid) {
        for ws in self.data.workspaces.iter_mut() {
            if let Some(pos) = ws
                .pinned_terminal_ids
                .iter()
                .position(|id| *id == session_id)
            {
                ws.pinned_terminal_ids.remove(pos);
                if let Some(ws_ui) = self.ws_ui.get_mut(&ws.id) {
                    if pos < ws_ui.pinned_panes.len() {
                        ws_ui.pinned_panes.remove(pos);
                    }
                    // If the removed pane was below `focused_pinned_pane`,
                    // shift focus left so it keeps pointing to a valid pane.
                    if ws_ui.focused_pinned_pane >= ws_ui.pinned_panes.len()
                        && !ws_ui.pinned_panes.is_empty()
                    {
                        ws_ui.focused_pinned_pane = ws_ui.pinned_panes.len() - 1;
                    } else if ws_ui.pinned_panes.is_empty() {
                        ws_ui.focused_pinned_pane = 0;
                    }
                }
            }
        }
    }

    /// Returns workspace indices in visual order (Working first, then Paused)
    pub fn workspace_visual_order(&self) -> Vec<usize> {
        let mut working: Vec<usize> = self
            .data
            .workspaces
            .iter()
            .enumerate()
            .filter(|(_, ws)| ws.status == WorkspaceStatus::Working)
            .map(|(i, _)| i)
            .collect();

        let paused: Vec<usize> = self
            .data
            .workspaces
            .iter()
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
        if let Some(pos) = visual_order
            .iter()
            .position(|&idx| idx == self.ui.selected_workspace_idx)
        {
            if pos > 0 {
                self.ui.selected_workspace_idx = visual_order[pos - 1];
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
        if let Some(pos) = visual_order
            .iter()
            .position(|&idx| idx == self.ui.selected_workspace_idx)
        {
            if pos < visual_order.len() - 1 {
                self.ui.selected_workspace_idx = visual_order[pos + 1];
            }
        }
    }

    /// Returns session indices in visual order (Agents first, then Terminals)
    pub fn session_visual_order(&self) -> Vec<usize> {
        let sessions = self.sessions_for_selected_workspace();

        let mut agents: Vec<usize> = sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.agent_type.is_terminal())
            .map(|(i, _)| i)
            .collect();

        let terminals: Vec<usize> = sessions
            .iter()
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
        if let Some(pos) = visual_order
            .iter()
            .position(|&idx| idx == self.selected_session_idx())
        {
            if pos > 0 {
                self.set_selected_session_idx(visual_order[pos - 1]);
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
        if let Some(pos) = visual_order
            .iter()
            .position(|&idx| idx == self.selected_session_idx())
        {
            if pos < visual_order.len() - 1 {
                self.set_selected_session_idx(visual_order[pos + 1]);
            }
        }
    }

    pub fn sessions_for_selected_workspace(&self) -> &[Session] {
        self.selected_workspace()
            .and_then(|ws| self.data.sessions.get(&ws.id))
            .map(|s| s.as_slice())
            .unwrap_or(&[])
    }

    pub fn selected_session(&self) -> Option<&Session> {
        self.sessions_for_selected_workspace()
            .get(self.selected_session_idx())
    }

    /// Check if the active session is one of the pinned terminals
    pub fn active_is_pinned(&self) -> bool {
        if let Some(active) = self.active_session_id() {
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
        self.active_session_id()
            .and_then(|id| self.system.output_buffers.get(&id))
    }

    /// Get active session, but return None if the active session is pinned
    pub fn active_session(&self) -> Option<&Session> {
        // Don't show pinned terminal in output pane when split view is active
        if self.should_show_split() && self.active_is_pinned() {
            return None;
        }
        self.active_session_id()
            .and_then(|id| self.data.sessions.values().flatten().find(|s| s.id == id))
    }

    /// Get all pinned terminal IDs for the current workspace
    pub fn pinned_terminal_ids(&self) -> &[Uuid] {
        self.selected_workspace()
            .map(|ws| ws.pinned_terminal_ids.as_slice())
            .unwrap_or(&[])
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
        self.pinned_terminal_id_at(index)
            .and_then(|id| self.data.sessions.values().flatten().find(|s| s.id == id))
    }

    /// Check if we should show split view (has at least one pinned terminal and split is enabled)
    pub fn should_show_split(&self) -> bool {
        self.ui.layout.split_view_enabled && self.pinned_count() > 0
    }

    /// Calculate normalized ratios for the current number of pinned panes
    /// Returns ratios that sum to 1.0
    pub fn normalized_pinned_ratios(&self) -> Vec<f32> {
        let count = self.pinned_count();
        if count == 0 {
            return vec![];
        }

        let ratios: Vec<f32> = self
            .ui
            .layout
            .pinned_pane_ratios
            .iter()
            .take(count)
            .copied()
            .collect();
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
        self.data
            .sessions
            .entry(workspace_id)
            .or_default()
            .push(session);
    }

    pub fn get_session(&self, session_id: Uuid) -> Option<&Session> {
        self.data
            .sessions
            .values()
            .flatten()
            .find(|s| s.id == session_id)
    }

    pub fn get_session_mut(&mut self, session_id: Uuid) -> Option<&mut Session> {
        self.data
            .sessions
            .values_mut()
            .flatten()
            .find(|s| s.id == session_id)
    }

    /// Get the workspace ID that contains a session
    pub fn workspace_id_for_session(&self, session_id: Uuid) -> Option<Uuid> {
        self.data.sessions.iter().find_map(|(ws_id, sessions)| {
            if sessions.iter().any(|s| s.id == session_id) {
                Some(*ws_id)
            } else {
                None
            }
        })
    }

    /// Get mutable reference to workspace by ID
    pub fn get_workspace_mut(&mut self, workspace_id: Uuid) -> Option<&mut Workspace> {
        self.data
            .workspaces
            .iter_mut()
            .find(|ws| ws.id == workspace_id)
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
        self.clear_active_session_everywhere(session_id);
        // Unpin from whichever workspace owns the pin — sessions can belong
        // to non-selected workspaces, so the previous "selected workspace
        // only" code was wrong.
        self.unpin_terminal_anywhere(session_id);
        // Remove output buffer + raw bytes + replay cache
        self.system.remove_session_buffers(&session_id);
        // Remove PTY handle if exists
        self.system.pty_handles.remove(&session_id);
        // Remove activity tracking
        self.data.last_activity.remove(&session_id);
        self.data.last_send_input.remove(&session_id);
    }

    /// Check if a session is actively working (received output within last 2 seconds)
    pub fn is_session_working(&self, session_id: Uuid) -> bool {
        if let Some(last) = self.data.last_activity.get(&session_id) {
            last.elapsed().as_secs_f32() < 2.0
        } else {
            false
        }
    }

    /// The model a session is answering with, ready to print — "Opus 5"
    /// rather than "Claude".
    ///
    /// The journal wins, because it is the only source that describes a turn
    /// that actually happened: every assistant line (Claude) and every
    /// `turn_context` (Codex) carries the model that produced it, so `/model`
    /// mid-session shows up on the next turn.
    ///
    /// The hook is a fallback, not the truth. It reports the model the session
    /// was *started* with and keeps reporting it after a switch: three live
    /// Claude sessions moved to Opus at 14:06 and were still being announced
    /// as Fable by `Notification` events firing after 16:37, which is the bug
    /// this ordering fixes. Being later than the journal line does not make it
    /// righter, so freshness cannot arbitrate between the two — only
    /// provenance can.
    ///
    /// It still earns its place underneath: Codex names its model on
    /// `SessionStart` but does not write its rollout until the first turn, so
    /// without the fallback every Codex agent read as plain "Codex" until
    /// someone talked to it. A session resumed onto a different model wears
    /// the old one for that same window — until its first turn is journalled —
    /// which is the one case this ordering is worse at, and it corrects
    /// itself.
    ///
    /// `None` when neither source has named one, so every caller keeps the
    /// provider name as its fallback.
    pub fn session_model(&self, session_id: Uuid) -> Option<String> {
        let journalled = self
            .system
            .agent_tasks
            .get(&session_id)
            .and_then(|tracker| tracker.model());
        let reported = || {
            self.system
                .agent_status
                .get(&session_id)
                .and_then(|status| status.model.as_deref())
        };
        let raw = journalled.or_else(reported)?;
        Some(crate::models::model_label(raw))
    }

    /// What to call a session on screen: its model if it has named one, else
    /// the agent it is running in.
    pub fn session_label(&self, session_id: Uuid) -> String {
        self.session_model(session_id).unwrap_or_else(|| {
            self.get_session(session_id)
                .map(|s| s.agent_type.display_name())
                .unwrap_or_default()
        })
    }

    /// What a session is doing, preferring what the agent reported over what
    /// its output looks like.
    ///
    /// Hooks are authoritative while fresh: only the agent knows the
    /// difference between thinking and being stopped at a permission prompt,
    /// and both look like silence from out here. Providers without hooks (and
    /// agents whose reports have gone stale) fall back to output timing, which
    /// is what workbench always did.
    pub fn activity(&self, session_id: Uuid) -> Activity {
        let running = self
            .get_session(session_id)
            .map(|s| s.status == SessionStatus::Running)
            .unwrap_or(false);
        if !running {
            return Activity::Exited;
        }

        if let Some(status) = self.system.agent_status.get(&session_id) {
            if status.is_fresh(chrono::Utc::now()) && status.activity != Activity::Exited {
                return status.activity;
            }
        }

        if self.is_session_working(session_id) {
            Activity::Working
        } else {
            Activity::Idle
        }
    }

    /// Whether `activity` is the agent's own report rather than an inference
    /// from output timing. Mirrors the condition in `activity` exactly, so the
    /// two cannot drift.
    ///
    /// The distinction matters where the two disagree about what silence
    /// means: a hook knows a turn ended, while output timing only knows that
    /// bytes stopped arriving — which a screen that merely repainted also
    /// satisfies.
    pub fn activity_is_reported(&self, session_id: Uuid) -> bool {
        self.system
            .agent_status
            .get(&session_id)
            .is_some_and(|status| {
                status.is_fresh(chrono::Utc::now()) && status.activity != Activity::Exited
            })
    }

    /// The prose behind `activity`, when the agent supplied any.
    pub fn activity_reason(&self, session_id: Uuid) -> Option<&str> {
        let status = self.system.agent_status.get(&session_id)?;
        status
            .is_fresh(chrono::Utc::now())
            .then(|| status.reason.as_str())
    }

    /// Sessions that stopped and are waiting on the user, newest report first.
    pub fn sessions_needing_attention(&self) -> Vec<(Uuid, Attention)> {
        let now = chrono::Utc::now();
        let mut waiting: Vec<(Uuid, Attention, chrono::DateTime<chrono::Utc>)> = self
            .system
            .agent_status
            .iter()
            .filter(|(_, status)| status.is_fresh(now))
            .filter_map(|(id, status)| {
                let kind = status.activity.needs_attention()?;
                let running = self
                    .get_session(*id)
                    .map(|s| s.status == SessionStatus::Running)
                    .unwrap_or(false);
                running.then_some((*id, kind, status.at))
            })
            .collect();
        waiting.sort_by(|a, b| b.2.cmp(&a.2));
        waiting.into_iter().map(|(id, kind, _)| (id, kind)).collect()
    }

    /// Check if a workspace has sessions waiting to start in the startup queue
    pub fn is_workspace_loading(&self, workspace_id: Uuid) -> bool {
        self.system
            .startup_queue
            .iter()
            .any(|p| p.workspace_id == workspace_id)
    }

    /// Get spinner character for animation
    pub fn spinner_char(&self) -> &'static str {
        const SPINNER_FRAMES: &[&str] = &[
            "\u{280B}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283C}", "\u{2834}", "\u{2826}",
            "\u{2827}", "\u{2807}", "\u{280F}",
        ];
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
        let working_workspace_ids: Vec<Uuid> = self
            .data
            .workspaces
            .iter()
            .filter(|ws| ws.status == WorkspaceStatus::Working)
            .map(|ws| ws.id)
            .collect();

        // Get all running AGENT sessions from WORKING workspaces (exclude terminals)
        let running_agent_sessions: Vec<Uuid> = self
            .data
            .sessions
            .iter()
            .filter(|(ws_id, _)| working_workspace_ids.contains(ws_id))
            .flat_map(|(_, sessions)| sessions)
            .filter(|s| s.status == SessionStatus::Running && s.agent_type.is_agent())
            .map(|s| s.id)
            .collect();

        // Sessions that cannot take new work. "Idle" here means free, so an
        // agent stopped at a permission prompt does not qualify: it cannot
        // read a consult until a human unblocks it, and offering it as the
        // next idle target would just strand the message.
        let working_sessions: Vec<Uuid> = running_agent_sessions
            .iter()
            .filter(|id| !self.activity(**id).is_free())
            .copied()
            .collect();

        // Remove sessions that are no longer running or are now working
        self.data
            .idle_queue
            .retain(|id| running_agent_sessions.contains(id) && !working_sessions.contains(id));

        // Track which sessions are newly idle
        let mut newly_idle = Vec::new();

        // Add newly idle sessions (running but not working, not already in queue)
        // Note: Active session CAN be idle - the tasks pane and comms rely on it
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
        self.data
            .sessions
            .values()
            .flatten()
            .filter(|s| s.status == SessionStatus::Running)
            .count()
    }

    /// Check if any agent in a workspace is actively working
    pub fn is_workspace_working(&self, workspace_id: Uuid) -> bool {
        self.data
            .sessions
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
        self.data
            .notepads
            .iter()
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

#[cfg(test)]
mod model_tests {
    use super::*;
    use crate::agent_status::{AgentStatus, Activity};
    use crate::agent_tasks::{Provider, Source, TaskSource, TaskTracker};
    use crate::models::AgentType;
    use std::io::Write;

    fn session(state: &mut AppState) -> Uuid {
        let workspace = Workspace::new("w".into(), std::path::PathBuf::from("/tmp/w"));
        let workspace_id = workspace.id;
        let session = Session::new(workspace_id, AgentType::Claude, false);
        let session_id = session.id;
        state.data.workspaces.push(workspace);
        state.data.sessions.insert(workspace_id, vec![session]);
        session_id
    }

    fn started_as(state: &mut AppState, id: Uuid, model: &str) {
        state.system.agent_status.insert(
            id,
            AgentStatus {
                activity: Activity::Idle,
                reason: "waiting for your input".into(),
                at: chrono::Utc::now(),
                event: "Notification".into(),
                transcript: None,
                model: Some(model.to_string()),
            },
        );
    }

    /// Point a real tracker at a real log, the way a live session's is.
    fn journalled(state: &mut AppState, id: Uuid, line: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "{line}").unwrap();
        file.flush().unwrap();

        let mut tracker =
            TaskTracker::with_source(Provider::Claude, Source::File(file.path().to_path_buf()));
        tracker.refresh(
            &TaskSource {
                provider: Provider::Claude,
                session_uuid: id.to_string(),
                cwd: std::path::PathBuf::from("/tmp/w"),
                started_at: chrono::Utc::now(),
                conversation: None,
                spawned_at: None,
                reported: None,
            },
            &std::collections::HashSet::new(),
        );
        state.system.agent_tasks.insert(id, tracker);
        file // held: dropping it deletes the log the tracker is reading
    }

    /// The bug: `/model` mid-session moved three live agents to Opus, and the
    /// hook went on announcing the model each was *started* with for hours —
    /// on `Notification` events firing a minute later than the Opus turn they
    /// contradicted. A turn that happened outranks a field that never moved.
    #[test]
    fn a_mid_session_model_switch_beats_the_one_it_started_with() {
        let mut state = AppState::default();
        let id = session(&mut state);

        started_as(&mut state, id, "claude-fable-5");
        assert_eq!(
            state.session_model(id).as_deref(),
            Some("Fable 5"),
            "with nothing journalled yet, the hook is all there is"
        );

        let _log = journalled(&mut state, id, r#"{"message":{"model":"claude-opus-5"}}"#);

        assert_eq!(
            state.session_model(id).as_deref(),
            Some("Opus 5"),
            "the journal names the model that answered; the hook names the one it booted on"
        );
    }

    /// Why the hook is kept at all: Codex names its model at `SessionStart`
    /// but writes no rollout until its first turn, so without the fallback it
    /// reads as bare "Codex" until someone talks to it.
    #[test]
    fn a_session_with_nothing_journalled_still_names_its_model() {
        let mut state = AppState::default();
        let id = session(&mut state);

        assert_eq!(state.session_model(id), None, "nothing to go on yet");

        started_as(&mut state, id, "gpt-5.6-sol");
        assert_eq!(state.session_model(id).as_deref(), Some("GPT-5.6 Sol"));
    }

    /// A subagent shares the file and may run a different model on the
    /// session's behalf, so its line must not be mistaken for the session's.
    #[test]
    fn a_subagents_model_is_not_the_sessions() {
        let mut state = AppState::default();
        let id = session(&mut state);
        started_as(&mut state, id, "claude-fable-5");

        let _log = journalled(
            &mut state,
            id,
            r#"{"isSidechain":true,"message":{"model":"claude-haiku-4-5"}}"#,
        );

        assert_eq!(
            state.session_model(id).as_deref(),
            Some("Fable 5"),
            "the sidechain is ignored, so the hook remains the only answer"
        );
    }
}

#[cfg(test)]
mod activity_tests {
    use super::*;
    use crate::agent_status::{AgentStatus, Attention};
    use crate::models::AgentType;

    fn state_with_agent() -> (AppState, Uuid) {
        let mut state = AppState::default();
        let workspace = Workspace::new("w".into(), std::path::PathBuf::from("/tmp/w"));
        let workspace_id = workspace.id;
        let session = Session::new(workspace_id, AgentType::Claude, false);
        let session_id = session.id;
        state.data.workspaces.push(workspace);
        state.data.sessions.insert(workspace_id, vec![session]);
        (state, session_id)
    }

    fn report(state: &mut AppState, id: Uuid, activity: Activity, age_mins: i64) {
        state.system.agent_status.insert(
            id,
            AgentStatus {
                activity,
                reason: "because".into(),
                at: chrono::Utc::now() - chrono::TimeDelta::minutes(age_mins),
                event: "test".into(),
                transcript: None,
                model: None,
            },
        );
    }

    /// The whole point: silence looks identical whether an agent is thinking
    /// or stopped at a permission prompt. Only the agent can tell us.
    #[test]
    fn a_blocked_agent_is_not_mistaken_for_a_quiet_one() {
        let (mut state, id) = state_with_agent();
        // No output for a while — the old inference would call this idle.
        assert_eq!(state.activity(id), Activity::Idle);

        report(
            &mut state,
            id,
            Activity::NeedsAttention(Attention::Permission),
            0,
        );
        assert_eq!(
            state.activity(id),
            Activity::NeedsAttention(Attention::Permission)
        );
        assert_eq!(
            state.sessions_needing_attention(),
            vec![(id, Attention::Permission)]
        );
    }

    #[test]
    fn a_blocked_agent_is_not_offered_as_free_for_new_work() {
        let (mut state, id) = state_with_agent();
        report(&mut state, id, Activity::Idle, 0);
        assert!(state.activity(id).is_free());

        // It cannot read a consult until a human unblocks it.
        report(&mut state, id, Activity::NeedsAttention(Attention::Input), 0);
        assert!(!state.activity(id).is_free());
        assert!(state.update_idle_queue().is_empty());
        assert!(!state.data.idle_queue.contains(&id));
    }

    #[test]
    fn stale_reports_hand_back_to_output_timing() {
        let (mut state, id) = state_with_agent();
        // An agent that died without a SessionEnd would otherwise look busy
        // forever.
        report(&mut state, id, Activity::Working, 45);
        assert_eq!(state.activity(id), Activity::Idle, "stale report ignored");

        report(&mut state, id, Activity::Working, 1);
        assert_eq!(state.activity(id), Activity::Working);
    }

    #[test]
    fn a_stopped_session_reports_nothing_regardless_of_its_last_hook() {
        let (mut state, id) = state_with_agent();
        report(&mut state, id, Activity::Working, 0);
        if let Some(session) = state.get_session_mut(id) {
            session.status = SessionStatus::Stopped;
        }
        assert_eq!(state.activity(id), Activity::Exited);
        assert!(state.sessions_needing_attention().is_empty());
    }

    #[test]
    fn the_most_recent_blocked_agent_is_surfaced_first() {
        let (mut state, first) = state_with_agent();
        let workspace_id = state.data.workspaces[0].id;
        let second = Session::new(workspace_id, AgentType::Claude, false);
        let second_id = second.id;
        state
            .data
            .sessions
            .get_mut(&workspace_id)
            .unwrap()
            .push(second);

        report(
            &mut state,
            first,
            Activity::NeedsAttention(Attention::Permission),
            5,
        );
        report(
            &mut state,
            second_id,
            Activity::NeedsAttention(Attention::Question),
            1,
        );

        assert_eq!(
            state.sessions_needing_attention(),
            vec![(second_id, Attention::Question), (first, Attention::Permission)]
        );
    }
}
