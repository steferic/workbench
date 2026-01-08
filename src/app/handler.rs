use crate::app::{
    Action, AppState, ConfigItem, Divider, FocusPanel, InputMode, PendingDelete,
    TextSelection, TodosTab, UtilityItem, UtilitySection,
};
use crate::models::{AgentType, Session, Workspace};
use crate::persistence;
use crate::pty::PtyManager;
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::pty_ops::resize_ptys_to_panes;
use super::selection::{clear_all_pinned_selections, copy_to_clipboard, extract_selected_text};
use super::session_start::start_workspace_sessions;
use super::utilities::load_utility_content;

pub fn process_action(
    state: &mut AppState,
    action: Action,
    pty_manager: &PtyManager,
    action_tx: &mpsc::UnboundedSender<Action>,
) -> Result<()> {
    match action {
        Action::Quit => {
            state.should_quit = true;
        }
        Action::Tick => {
            use crate::app::TodoPaneMode;

            state.tick_animation();
            let newly_idle = state.update_idle_queue();

            // Check if analyzer session went idle
            if let Some(analyzer_id) = state.analyzer_session_id {
                if newly_idle.contains(&analyzer_id) {
                    // Parse output for TODO: lines
                    if let Some(parser) = state.output_buffers.get(&analyzer_id) {
                        let screen = parser.screen();
                        let mut todos_to_add: Vec<String> = Vec::new();

                        // Get full screen contents as text
                        let full_contents = screen.contents();

                        // Scan for TODO: lines
                        for line in full_contents.lines() {
                            // Look for "TODO: " pattern
                            if let Some(idx) = line.find("TODO: ") {
                                let todo_text = line[idx + 6..].trim();
                                if !todo_text.is_empty() {
                                    // Clean up any control characters
                                    let clean_text: String = todo_text
                                        .chars()
                                        .filter(|c| !c.is_control())
                                        .collect();
                                    if !clean_text.is_empty() && clean_text.len() > 5 {
                                        todos_to_add.push(clean_text);
                                    }
                                }
                            }
                        }

                        // Add each TODO as a suggested todo
                        for todo_desc in todos_to_add {
                            let _ = action_tx.send(Action::AddSuggestedTodo(todo_desc));
                        }
                    }

                    // Clear analyzer session
                    state.analyzer_session_id = None;
                }
            }

            // Process newly idle sessions - mark their todos as ready for review
            for session_id in &newly_idle {
                if let Some(workspace_id) = state.workspace_id_for_session(*session_id) {
                    let has_in_progress_todo = state.get_workspace(workspace_id)
                        .and_then(|ws| ws.todo_for_session(*session_id))
                        .map(|todo| todo.is_in_progress())
                        .unwrap_or(false);

                    if has_in_progress_todo {
                        if let Some(ws) = state.get_workspace_mut(workspace_id) {
                            if let Some(todo) = ws.todo_for_session_mut(*session_id) {
                                let todo_id = todo.id;
                                let _ = action_tx.send(Action::MarkTodoReadyForReview(todo_id));
                            }
                        }
                    }
                }
            }

            // In Autorun mode, check ALL idle sessions for pending todos (not just newly idle)
            if state.todo_pane_mode == TodoPaneMode::Autorun {
                // Get the current idle queue
                let idle_sessions: Vec<Uuid> = state.idle_queue.clone();

                for session_id in idle_sessions {
                    if let Some(workspace_id) = state.workspace_id_for_session(session_id) {
                        // Check if session already has an in-progress todo
                        let has_in_progress = state.get_workspace(workspace_id)
                            .and_then(|ws| ws.todo_for_session(session_id))
                            .map(|t| t.is_in_progress())
                            .unwrap_or(false);

                        if !has_in_progress {
                            // Find next pending todo for this workspace
                            let pending_todo_info = state.get_workspace(workspace_id)
                                .and_then(|ws| ws.next_pending_todo())
                                .map(|todo| (todo.id, todo.description.clone()));

                            if let Some((todo_id, description)) = pending_todo_info {
                                let _ = action_tx.send(Action::DispatchTodoToSession(session_id, todo_id, description));
                                break; // Only dispatch one todo per tick to avoid overwhelming
                            }
                        }
                    }
                }
            }
        }
        Action::Resize(w, h) => {
            state.terminal_size = (w, h);
            resize_ptys_to_panes(state);
        }

        // Mouse click - determine which panel was clicked, check for divider, or start selection
        Action::MouseClick(x, y) => {
            let (w, h) = state.terminal_size;
            let main_height = h.saturating_sub(1); // Subtract status bar
            let left_width = (w as f32 * state.left_panel_ratio) as u16;
            let workspace_height = (main_height as f32 * state.workspace_ratio) as u16;

            // Check if clicking on a divider (within 2 pixels)
            let divider_tolerance = 2u16;

            // Left-right divider
            if x >= left_width.saturating_sub(divider_tolerance) && x <= left_width + divider_tolerance && y < main_height {
                state.dragging_divider = Some(Divider::LeftRight);
                state.drag_start_pos = Some((x, y));
                state.drag_start_ratio = state.left_panel_ratio;
                return Ok(());
            }

            // Workspace-session divider (within left panel)
            if x < left_width && y >= workspace_height.saturating_sub(divider_tolerance) && y <= workspace_height + divider_tolerance {
                state.dragging_divider = Some(Divider::WorkspaceSession);
                state.drag_start_pos = Some((x, y));
                state.drag_start_ratio = state.workspace_ratio;
                return Ok(());
            }

            // Sessions-Todos divider (within lower-left area)
            let lower_left_height = main_height.saturating_sub(workspace_height);
            let sessions_height = (lower_left_height as f32 * state.sessions_ratio) as u16;
            let sessions_todos_divider_y = workspace_height + sessions_height;

            if x < left_width && y >= sessions_todos_divider_y.saturating_sub(divider_tolerance) && y <= sessions_todos_divider_y + divider_tolerance {
                state.dragging_divider = Some(Divider::SessionsTodos);
                state.drag_start_pos = Some((x, y));
                state.drag_start_ratio = state.sessions_ratio;
                return Ok(());
            }

            // Todos-Utilities divider (within lower-left area)
            let remaining_height = lower_left_height.saturating_sub(sessions_height);
            let todos_height = (remaining_height as f32 * state.todos_ratio) as u16;
            let todos_utilities_divider_y = sessions_todos_divider_y + todos_height;

            if x < left_width && y >= todos_utilities_divider_y.saturating_sub(divider_tolerance) && y <= todos_utilities_divider_y + divider_tolerance {
                state.dragging_divider = Some(Divider::TodosUtilities);
                state.drag_start_pos = Some((x, y));
                state.drag_start_ratio = state.todos_ratio;
                return Ok(());
            }

            // Output-pinned divider (only when split view is active)
            if state.should_show_split() {
                if let Some((ox, _, ow, _)) = state.output_pane_area {
                    let divider_x = ox + ow;
                    if x >= divider_x.saturating_sub(divider_tolerance) && x <= divider_x + divider_tolerance && y < main_height {
                        state.dragging_divider = Some(Divider::OutputPinned);
                        state.drag_start_pos = Some((x, y));
                        state.drag_start_ratio = state.output_split_ratio;
                        return Ok(());
                    }
                }

                // Check for dividers between pinned panes (horizontal dividers)
                let pinned_count = state.pinned_count();
                if pinned_count > 1 {
                    for pane_idx in 0..(pinned_count - 1) {
                        if let Some((_, py, _, ph)) = state.pinned_pane_areas[pane_idx] {
                            let divider_y = py + ph;
                            if y >= divider_y.saturating_sub(divider_tolerance) && y <= divider_y + divider_tolerance {
                                // Check if x is within the pinned pane area
                                if let Some((px, _, pw, _)) = state.pinned_pane_areas[0] {
                                    if x >= px && x < px + pw {
                                        state.dragging_divider = Some(Divider::PinnedPanes(pane_idx));
                                        state.drag_start_pos = Some((x, y));
                                        state.drag_start_ratio = state.pinned_pane_ratios[pane_idx];
                                        return Ok(());
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Not on a divider - handle panel focus and selection
            if y < main_height {
                if x < left_width {
                    // Left panel - clear any selection
                    state.text_selection = TextSelection::default();
                    clear_all_pinned_selections(state);
                    if y < workspace_height {
                        state.focus = FocusPanel::WorkspaceList;
                    } else {
                        // Lower left is split: sessions | todos | utilities (using dynamic ratios)
                        // Reuse the already calculated values from divider detection
                        if y < sessions_todos_divider_y {
                            state.focus = FocusPanel::SessionList;
                        } else if y < todos_utilities_divider_y {
                            state.focus = FocusPanel::TodosPane;
                        } else {
                            state.focus = FocusPanel::UtilitiesPane;
                        }
                    }
                } else {
                    // Right panel - check if clicking on pinned pane or output pane
                    // Clear all selections first
                    state.text_selection = TextSelection::default();
                    clear_all_pinned_selections(state);

                    // Check if click is in any pinned pane area
                    let mut clicked_pinned_idx: Option<usize> = None;
                    for (idx, area_opt) in state.pinned_pane_areas.iter().enumerate() {
                        if let Some((px, py, pw, ph)) = area_opt {
                            if x >= *px && x < px + pw && y >= *py && y < py + ph {
                                clicked_pinned_idx = Some(idx);
                                break;
                            }
                        }
                    }

                    if let Some(idx) = clicked_pinned_idx {
                        state.focus = FocusPanel::PinnedTerminalPane(idx);
                        state.focused_pinned_pane = idx;

                        // Start selection in pinned pane
                        if let Some((area_x, area_y, _, _)) = state.pinned_pane_areas[idx] {
                            let text_col = (x.saturating_sub(area_x).saturating_sub(1)) as usize;
                            let text_row = (y.saturating_sub(area_y).saturating_sub(1)) as usize;

                            if let Some(sel) = state.pinned_text_selections.get_mut(idx) {
                                *sel = TextSelection {
                                    start: Some((text_row, text_col)),
                                    end: Some((text_row, text_col)),
                                    is_dragging: true,
                                };
                            }
                        }
                    } else {
                        // Output pane - start selection
                        state.focus = FocusPanel::OutputPane;

                        // Convert screen coordinates to text position
                        if let Some((area_x, area_y, _, _)) = state.output_pane_area {
                            // Account for border (1 pixel)
                            let text_col = (x.saturating_sub(area_x).saturating_sub(1)) as usize;
                            let text_row = (y.saturating_sub(area_y).saturating_sub(1) + state.output_scroll_offset) as usize;

                            state.text_selection = TextSelection {
                                start: Some((text_row, text_col)),
                                end: Some((text_row, text_col)),
                                is_dragging: true,
                            };
                        }
                    }
                }
            } else {
                // Clicked on status bar - clear selection
                state.text_selection = TextSelection::default();
                clear_all_pinned_selections(state);
            }
        }

        // Mouse drag - update selection end or resize divider
        Action::MouseDrag(x, y) => {
            // Handle divider dragging
            if let Some(divider) = state.dragging_divider {
                let (w, h) = state.terminal_size;
                let main_height = h.saturating_sub(1);

                match divider {
                    Divider::LeftRight => {
                        // Calculate new ratio based on x position
                        let new_ratio = (x as f32 / w as f32).clamp(0.15, 0.50);
                        state.left_panel_ratio = new_ratio;
                    }
                    Divider::WorkspaceSession => {
                        // Calculate new ratio based on y position
                        let new_ratio = (y as f32 / main_height as f32).clamp(0.20, 0.80);
                        state.workspace_ratio = new_ratio;
                    }
                    Divider::SessionsTodos => {
                        // Calculate new sessions_ratio based on y position within lower_left
                        let workspace_height = (main_height as f32 * state.workspace_ratio) as u16;
                        let lower_left_height = main_height.saturating_sub(workspace_height);
                        let y_in_lower_left = y.saturating_sub(workspace_height);
                        let new_ratio = (y_in_lower_left as f32 / lower_left_height as f32).clamp(0.15, 0.70);
                        state.sessions_ratio = new_ratio;
                    }
                    Divider::TodosUtilities => {
                        // Calculate new todos_ratio based on y position within remaining area
                        let workspace_height = (main_height as f32 * state.workspace_ratio) as u16;
                        let lower_left_height = main_height.saturating_sub(workspace_height);
                        let sessions_height = (lower_left_height as f32 * state.sessions_ratio) as u16;
                        let remaining_height = lower_left_height.saturating_sub(sessions_height);
                        let y_in_remaining = y.saturating_sub(workspace_height).saturating_sub(sessions_height);
                        let new_ratio = (y_in_remaining as f32 / remaining_height as f32).clamp(0.20, 0.80);
                        state.todos_ratio = new_ratio;
                    }
                    Divider::OutputPinned => {
                        // Calculate new ratio within right panel
                        let left_width = (w as f32 * state.left_panel_ratio) as u16;
                        let right_panel_width = w.saturating_sub(left_width);
                        let x_in_right = x.saturating_sub(left_width);
                        let new_ratio = (x_in_right as f32 / right_panel_width as f32).clamp(0.20, 0.80);
                        state.output_split_ratio = new_ratio;
                    }
                    Divider::PinnedPanes(pane_idx) => {
                        // Adjust ratios between adjacent pinned panes
                        let count = state.pinned_count();
                        if count > 1 && pane_idx < count - 1 {
                            // Get total height and calculate new distribution
                            let mut ratios = state.pinned_pane_ratios;
                            let sum: f32 = ratios.iter().take(count).sum();

                            // Calculate y position relative to pinned area start
                            if let Some((_, py, _, _)) = state.pinned_pane_areas[0] {
                                let pinned_total_height = state.pinned_pane_areas.iter()
                                    .take(count)
                                    .filter_map(|a| a.map(|(_, _, _, h)| h))
                                    .sum::<u16>();

                                let y_in_pinned = y.saturating_sub(py) as f32;
                                let new_split = y_in_pinned / pinned_total_height as f32;

                                // Adjust the ratios of the two adjacent panes
                                let combined_ratio = ratios[pane_idx] + ratios[pane_idx + 1];
                                let ratio_above: f32 = ratios.iter().take(pane_idx).sum();

                                // New ratio for pane at pane_idx
                                let new_upper_ratio = ((new_split - ratio_above / sum) * sum).clamp(0.1, combined_ratio - 0.1);
                                ratios[pane_idx] = new_upper_ratio;
                                ratios[pane_idx + 1] = combined_ratio - new_upper_ratio;

                                state.pinned_pane_ratios = ratios;
                            }
                        }
                    }
                }
                return Ok(());
            }

            // Handle text selection dragging in output pane
            if state.text_selection.is_dragging {
                if let Some((area_x, area_y, _, _)) = state.output_pane_area {
                    let text_col = (x.saturating_sub(area_x).saturating_sub(1)) as usize;
                    let text_row = (y.saturating_sub(area_y).saturating_sub(1) + state.output_scroll_offset) as usize;
                    state.text_selection.end = Some((text_row, text_col));
                }
            }

            // Handle text selection dragging in pinned panes
            for (idx, sel) in state.pinned_text_selections.iter_mut().enumerate() {
                if sel.is_dragging {
                    if let Some((area_x, area_y, _, _)) = state.pinned_pane_areas[idx] {
                        let text_col = (x.saturating_sub(area_x).saturating_sub(1)) as usize;
                        let text_row = (y.saturating_sub(area_y).saturating_sub(1)) as usize;
                        sel.end = Some((text_row, text_col));
                    }
                }
            }
        }

        // Mouse up - finalize selection or divider drag
        Action::MouseUp(x, y) => {
            // Finalize divider dragging
            if state.dragging_divider.is_some() {
                state.dragging_divider = None;
                state.drag_start_pos = None;
                // Resize PTYs to match new pane sizes
                resize_ptys_to_panes(state);
                // Save pane ratios to config
                let config = persistence::GlobalConfig {
                    banner_visible: state.banner_visible,
                    left_panel_ratio: state.left_panel_ratio,
                    workspace_ratio: state.workspace_ratio,
                    sessions_ratio: state.sessions_ratio,
                    todos_ratio: state.todos_ratio,
                    output_split_ratio: state.output_split_ratio,
                };
                let _ = persistence::save_config(&config);
                return Ok(());
            }

            // Finalize text selection in output pane
            if state.text_selection.is_dragging {
                if let Some((area_x, area_y, _, _)) = state.output_pane_area {
                    let text_col = (x.saturating_sub(area_x).saturating_sub(1)) as usize;
                    let text_row = (y.saturating_sub(area_y).saturating_sub(1) + state.output_scroll_offset) as usize;
                    state.text_selection.end = Some((text_row, text_col));
                }
                state.text_selection.is_dragging = false;

                // If start and end are the same, clear selection (it was just a click)
                if state.text_selection.start == state.text_selection.end {
                    state.text_selection = TextSelection::default();
                }
            }

            // Finalize text selection in pinned panes
            for (idx, sel) in state.pinned_text_selections.iter_mut().enumerate() {
                if sel.is_dragging {
                    if let Some((area_x, area_y, _, _)) = state.pinned_pane_areas[idx] {
                        let text_col = (x.saturating_sub(area_x).saturating_sub(1)) as usize;
                        let text_row = (y.saturating_sub(area_y).saturating_sub(1)) as usize;
                        sel.end = Some((text_row, text_col));
                    }
                    sel.is_dragging = false;

                    // If start and end are the same, clear selection (it was just a click)
                    if sel.start == sel.end {
                        *sel = TextSelection::default();
                    }
                }
            }
        }

        // Copy selected text to clipboard
        Action::CopySelection => {
            // Check if copying from output pane
            if let (Some(parser), Some(start), Some(end)) = (
                state.active_output(),
                state.text_selection.start,
                state.text_selection.end,
            ) {
                let text = extract_selected_text(parser.screen(), start, end);
                copy_to_clipboard(&text);
            }
            // Check if copying from any pinned pane
            for idx in 0..state.pinned_count() {
                let sel = &state.pinned_text_selections[idx];
                if let (Some(parser), Some(start), Some(end)) = (
                    state.pinned_terminal_output_at(idx),
                    sel.start,
                    sel.end,
                ) {
                    let text = extract_selected_text(parser.screen(), start, end);
                    copy_to_clipboard(&text);
                    break; // Only copy from one pane
                }
            }
            state.text_selection = TextSelection::default();
            clear_all_pinned_selections(state);
        }

        // Clear selection
        Action::ClearSelection => {
            state.text_selection = TextSelection::default();
            clear_all_pinned_selections(state);
        }

        // Pin/unpin terminal and toggle split view
        Action::PinSession(session_id) => {
            // Pin to the currently selected workspace
            let ws_idx = state.selected_workspace_idx;
            if ws_idx < state.workspaces.len() {
                let pinned = state.workspaces[ws_idx].pin_terminal(session_id);
                if pinned {
                    // Auto-enable split view
                    state.split_view_enabled = true;
                    // Focus the newly pinned pane
                    let new_idx = state.workspaces[ws_idx].pinned_terminal_ids.len().saturating_sub(1);
                    state.focused_pinned_pane = new_idx;
                    // Resize PTYs to match new pane configuration
                    resize_ptys_to_panes(state);
                    // Auto-save
                    let _ = persistence::save(&state.workspaces, &state.sessions);
                }
            }
        }
        Action::UnpinSession(session_id) => {
            if let Some(ws) = state.workspaces.get_mut(state.selected_workspace_idx) {
                ws.unpin_terminal(session_id);
                // Adjust focused pinned pane if needed
                let count = ws.pinned_terminal_ids.len();
                if state.focused_pinned_pane >= count && count > 0 {
                    state.focused_pinned_pane = count - 1;
                }
                // Resize PTYs
                resize_ptys_to_panes(state);
                // Auto-save
                let _ = persistence::save(&state.workspaces, &state.sessions);
            }
        }
        Action::UnpinFocusedSession => {
            let session_id = state.pinned_terminal_id_at(state.focused_pinned_pane);
            if let (Some(ws), Some(sid)) = (state.workspaces.get_mut(state.selected_workspace_idx), session_id) {
                ws.unpin_terminal(sid);
                // Adjust focused pinned pane if needed
                let count = ws.pinned_terminal_ids.len();
                if state.focused_pinned_pane >= count && count > 0 {
                    state.focused_pinned_pane = count - 1;
                }
                // If no more pinned terminals, switch focus back to session list
                if count == 0 {
                    state.focus = FocusPanel::SessionList;
                }
                // Resize PTYs
                resize_ptys_to_panes(state);
                // Auto-save
                let _ = persistence::save(&state.workspaces, &state.sessions);
            }
        }
        Action::ToggleSplitView => {
            state.split_view_enabled = !state.split_view_enabled;
            // Resize PTYs to match new pane configuration
            resize_ptys_to_panes(state);
        }
        Action::FocusPinnedPane(idx) => {
            if idx < state.pinned_count() {
                state.focused_pinned_pane = idx;
                state.focus = FocusPanel::PinnedTerminalPane(idx);
            }
        }
        Action::NextPinnedPane => {
            let count = state.pinned_count();
            if count > 0 {
                state.focused_pinned_pane = (state.focused_pinned_pane + 1) % count;
                state.focus = FocusPanel::PinnedTerminalPane(state.focused_pinned_pane);
            }
        }
        Action::PrevPinnedPane => {
            let count = state.pinned_count();
            if count > 0 {
                state.focused_pinned_pane = if state.focused_pinned_pane == 0 {
                    count - 1
                } else {
                    state.focused_pinned_pane - 1
                };
                state.focus = FocusPanel::PinnedTerminalPane(state.focused_pinned_pane);
            }
        }

        // Navigation
        Action::MoveUp => match state.focus {
            FocusPanel::WorkspaceList => {
                let prev_idx = state.selected_workspace_idx;
                state.select_prev_workspace(); // Navigate in visual order
                // Start all stopped sessions if workspace changed
                if state.selected_workspace_idx != prev_idx {
                    start_workspace_sessions(state, pty_manager, action_tx);
                    state.reset_notepad_cursor(); // Reset cursor for new workspace's notepad
                }
            }
            FocusPanel::SessionList => {
                state.select_prev_session(); // Navigate in visual order (agents first, then terminals)
            }
            _ => {}
        },
        Action::MoveDown => match state.focus {
            FocusPanel::WorkspaceList => {
                let prev_idx = state.selected_workspace_idx;
                state.select_next_workspace(); // Navigate in visual order
                // Start all stopped sessions if workspace changed
                if state.selected_workspace_idx != prev_idx {
                    start_workspace_sessions(state, pty_manager, action_tx);
                    state.reset_notepad_cursor(); // Reset cursor for new workspace's notepad
                }
            }
            FocusPanel::SessionList => {
                state.select_next_session(); // Navigate in visual order (agents first, then terminals)
            }
            _ => {}
        },
        Action::FocusLeft => {
            // Cycle: WorkspaceList <- SessionList <- TodosPane <- UtilitiesPane <- OutputPane <- PinnedPanes <- (wrap)
            let pinned_count = state.pinned_count();
            state.focus = match state.focus {
                FocusPanel::WorkspaceList => {
                    // Wrap to rightmost pane
                    if state.should_show_split() && pinned_count > 0 {
                        FocusPanel::PinnedTerminalPane(pinned_count - 1)
                    } else {
                        FocusPanel::OutputPane
                    }
                }
                FocusPanel::SessionList => FocusPanel::WorkspaceList,
                FocusPanel::TodosPane => FocusPanel::SessionList,
                FocusPanel::UtilitiesPane => FocusPanel::TodosPane,
                FocusPanel::OutputPane => FocusPanel::UtilitiesPane,
                FocusPanel::PinnedTerminalPane(idx) => {
                    if idx == 0 {
                        FocusPanel::OutputPane
                    } else {
                        FocusPanel::PinnedTerminalPane(idx - 1)
                    }
                }
            };
        }
        Action::FocusRight => {
            // Cycle: WorkspaceList -> SessionList -> TodosPane -> UtilitiesPane -> OutputPane -> PinnedPanes -> (wrap)
            let pinned_count = state.pinned_count();
            let prev_focus = state.focus;
            state.focus = match state.focus {
                FocusPanel::WorkspaceList => FocusPanel::SessionList,
                FocusPanel::SessionList => FocusPanel::TodosPane,
                FocusPanel::TodosPane => FocusPanel::UtilitiesPane,
                FocusPanel::UtilitiesPane => FocusPanel::OutputPane,
                FocusPanel::OutputPane => {
                    if state.should_show_split() && pinned_count > 0 {
                        FocusPanel::PinnedTerminalPane(0)
                    } else {
                        FocusPanel::WorkspaceList // Wrap around
                    }
                }
                FocusPanel::PinnedTerminalPane(idx) => {
                    if idx + 1 < pinned_count {
                        FocusPanel::PinnedTerminalPane(idx + 1)
                    } else {
                        FocusPanel::WorkspaceList // Wrap around
                    }
                }
            };
            // Start all stopped sessions when entering SessionList from WorkspaceList
            if prev_focus == FocusPanel::WorkspaceList && state.focus == FocusPanel::SessionList {
                start_workspace_sessions(state, pty_manager, action_tx);
            }
        }
        Action::FocusUtilitiesPane => {
            state.focus = FocusPanel::UtilitiesPane;
        }
        Action::ScrollOutputUp => {
            // Scroll the focused pane
            if let FocusPanel::PinnedTerminalPane(idx) = state.focus {
                if let Some(offset) = state.pinned_scroll_offsets.get_mut(idx) {
                    *offset = offset.saturating_add(1);
                }
            } else {
                state.output_scroll_offset = state.output_scroll_offset.saturating_sub(1);
            }
        }
        Action::ScrollOutputDown => {
            // Scroll the focused pane
            if let FocusPanel::PinnedTerminalPane(idx) = state.focus {
                if let Some(offset) = state.pinned_scroll_offsets.get_mut(idx) {
                    *offset = offset.saturating_sub(1);
                }
            } else {
                state.output_scroll_offset = state.output_scroll_offset.saturating_add(1);
            }
        }
        Action::ScrollOutputToBottom => {
            if let FocusPanel::PinnedTerminalPane(idx) = state.focus {
                if let Some(offset) = state.pinned_scroll_offsets.get_mut(idx) {
                    *offset = 0;
                }
            } else {
                state.output_scroll_offset = 0;
            }
        }
        Action::JumpToNextIdle => {
            use crate::models::WorkspaceStatus;

            // Get indices of all Working workspaces
            let working_indices: Vec<usize> = state.workspaces.iter()
                .enumerate()
                .filter(|(_, ws)| ws.status == WorkspaceStatus::Working)
                .map(|(idx, _)| idx)
                .collect();

            if working_indices.is_empty() {
                return Ok(());
            }

            // Find current position in working workspaces and rotate to next
            let current_pos = working_indices.iter()
                .position(|&idx| idx == state.selected_workspace_idx);

            let next_idx = match current_pos {
                Some(pos) => working_indices[(pos + 1) % working_indices.len()],
                None => working_indices[0], // Not on a working workspace, go to first one
            };

            // Switch to the next working workspace
            state.selected_workspace_idx = next_idx;
            state.selected_session_idx = 0;

            // Activate first session if available
            if let Some(ws) = state.workspaces.get(next_idx) {
                if let Some(sessions) = state.sessions.get(&ws.id) {
                    if let Some(session) = sessions.first() {
                        state.active_session_id = Some(session.id);
                        state.focus = FocusPanel::OutputPane;
                        state.output_scroll_offset = 0;
                    }
                }
            }
        }

        // Mode changes
        Action::EnterHelpMode => {
            state.input_mode = InputMode::Help;
        }
        Action::EnterWorkspaceActionMode => {
            state.input_mode = InputMode::SelectWorkspaceAction;
            state.selected_workspace_action = crate::app::WorkspaceAction::default();
        }
        Action::EnterCreateWorkspaceMode => {
            state.input_mode = InputMode::CreateWorkspace;
            state.input_buffer.clear();
            // Initialize file browser at home directory
            state.file_browser_path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
            state.refresh_file_browser();
        }
        Action::EnterWorkspaceNameMode => {
            state.input_mode = InputMode::EnterWorkspaceName;
            state.input_buffer.clear();
        }
        Action::EnterCreateSessionMode => {
            if state.selected_workspace().is_some() {
                state.input_mode = InputMode::CreateSession;
            }
        }
        Action::ExitMode => {
            state.input_mode = InputMode::Normal;
            state.input_buffer.clear();
            state.editing_session_id = None;
        }
        Action::EnterSetStartCommandMode => {
            // Get session info first to avoid borrow issues
            let session_info = state.selected_session()
                .filter(|s| s.agent_type.is_terminal())
                .map(|s| (s.id, s.start_command.clone()));

            if let Some((session_id, existing_cmd)) = session_info {
                state.editing_session_id = Some(session_id);
                // Pre-fill with existing command if any
                state.input_buffer = existing_cmd.unwrap_or_default();
                state.input_mode = InputMode::SetStartCommand;
            }
        }
        Action::SetStartCommand(session_id, command) => {
            if let Some(session) = state.get_session_mut(session_id) {
                session.start_command = if command.is_empty() {
                    None
                } else {
                    Some(command)
                };
            }
            state.input_mode = InputMode::Normal;
            state.input_buffer.clear();
            state.editing_session_id = None;
            // Save state
            let _ = persistence::save(&state.workspaces, &state.sessions);
        }

        // Input handling
        Action::InputChar(c) => {
            state.input_buffer.push(c);
        }
        Action::InputBackspace => {
            state.input_buffer.pop();
        }
        Action::InputSubmit => {
            // Legacy - kept for compatibility
        }

        // File browser actions
        Action::FileBrowserUp => {
            if state.file_browser_selected > 0 {
                state.file_browser_selected -= 1;
                // Adjust scroll if needed
                if state.file_browser_selected < state.file_browser_scroll {
                    state.file_browser_scroll = state.file_browser_selected;
                }
            }
        }
        Action::FileBrowserDown => {
            if state.file_browser_selected < state.file_browser_entries.len().saturating_sub(1) {
                state.file_browser_selected += 1;
                // Adjust scroll if needed (assuming ~15 visible items)
                let visible_height = 15;
                if state.file_browser_selected >= state.file_browser_scroll + visible_height {
                    state.file_browser_scroll = state.file_browser_selected - visible_height + 1;
                }
            }
        }
        Action::FileBrowserEnter => {
            state.file_browser_enter_selected();
        }
        Action::FileBrowserBack => {
            state.file_browser_go_up();
        }
        Action::FileBrowserSelect => {
            // Select highlighted directory as workspace (or current dir if nothing highlighted)
            let path = if let Some(selected) = state.file_browser_entries.get(state.file_browser_selected) {
                selected.clone()
            } else {
                state.file_browser_path.clone()
            };
            if path.exists() && path.is_dir() {
                let workspace = Workspace::from_path(path);
                state.add_workspace(workspace);
                state.input_mode = InputMode::Normal;
                // Auto-save
                let _ = persistence::save(&state.workspaces, &state.sessions);
            }
        }

        // Workspace action selection
        Action::SelectNextWorkspaceAction => {
            use crate::app::WorkspaceAction;
            let actions = WorkspaceAction::all();
            let current_idx = actions.iter().position(|a| *a == state.selected_workspace_action).unwrap_or(0);
            if current_idx < actions.len() - 1 {
                state.selected_workspace_action = actions[current_idx + 1];
            }
        }
        Action::SelectPrevWorkspaceAction => {
            use crate::app::WorkspaceAction;
            let actions = WorkspaceAction::all();
            let current_idx = actions.iter().position(|a| *a == state.selected_workspace_action).unwrap_or(0);
            if current_idx > 0 {
                state.selected_workspace_action = actions[current_idx - 1];
            }
        }
        Action::ConfirmWorkspaceAction => {
            use crate::app::WorkspaceAction;
            match state.selected_workspace_action {
                WorkspaceAction::CreateNew => {
                    // Go to file browser first to select parent directory
                    state.workspace_create_mode = true;
                    state.input_mode = InputMode::CreateWorkspace;
                    state.input_buffer.clear();
                    state.file_browser_path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                    state.refresh_file_browser();
                }
                WorkspaceAction::OpenExisting => {
                    // Go directly to file browser for selecting existing
                    state.workspace_create_mode = false;
                    state.input_mode = InputMode::CreateWorkspace;
                    state.input_buffer.clear();
                    state.file_browser_path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                    state.refresh_file_browser();
                }
            }
        }
        Action::CreateNewWorkspace(name) => {
            // Create the new directory in the current file browser path
            let new_path = state.file_browser_path.join(&name);
            if !new_path.exists() {
                if std::fs::create_dir_all(&new_path).is_ok() {
                    let workspace = Workspace::from_path(new_path);
                    state.add_workspace(workspace);
                    state.input_mode = InputMode::Normal;
                    // Auto-save
                    let _ = persistence::save(&state.workspaces, &state.sessions);
                }
            }
        }

        // Utilities pane actions
        Action::SelectNextUtility => {
            match state.utility_section {
                UtilitySection::Utilities => {
                    let utilities = UtilityItem::all();
                    let current_idx = utilities.iter().position(|u| *u == state.selected_utility).unwrap_or(0);
                    if current_idx < utilities.len() - 1 {
                        state.selected_utility = utilities[current_idx + 1];
                    }
                }
                UtilitySection::GlobalConfig => {
                    let configs = ConfigItem::all();
                    let current_idx = configs.iter().position(|c| *c == state.selected_config).unwrap_or(0);
                    if current_idx < configs.len() - 1 {
                        state.selected_config = configs[current_idx + 1];
                    }
                }
                UtilitySection::Notepad => {
                    // Notepad doesn't have items to navigate
                }
            }
        }
        Action::SelectPrevUtility => {
            match state.utility_section {
                UtilitySection::Utilities => {
                    let utilities = UtilityItem::all();
                    let current_idx = utilities.iter().position(|u| *u == state.selected_utility).unwrap_or(0);
                    if current_idx > 0 {
                        state.selected_utility = utilities[current_idx - 1];
                    }
                }
                UtilitySection::GlobalConfig => {
                    let configs = ConfigItem::all();
                    let current_idx = configs.iter().position(|c| *c == state.selected_config).unwrap_or(0);
                    if current_idx > 0 {
                        state.selected_config = configs[current_idx - 1];
                    }
                }
                UtilitySection::Notepad => {
                    // Notepad doesn't have items to navigate
                }
            }
        }
        Action::ActivateUtility => {
            // Special handling for SuggestTodos - trigger analyzer
            if state.selected_utility == UtilityItem::SuggestTodos {
                // Find an idle agent session from the current workspace
                let idle_agent = state.selected_workspace().and_then(|ws| {
                    state.sessions.get(&ws.id).and_then(|sessions| {
                        sessions.iter()
                            .find(|s| s.agent_type.is_agent() && state.idle_queue.contains(&s.id))
                            .map(|s| s.id)
                    })
                });

                if let Some(session_id) = idle_agent {
                    // Mark this session as the analyzer
                    state.analyzer_session_id = Some(session_id);

                    // Send the analysis prompt
                    let prompt = r#"Analyze this codebase and suggest 3-5 potential improvements, new features, or refactoring opportunities.

For each suggestion, output it on its own line in this exact format:
TODO: [DIFFICULTY] [IMPORTANCE] <description>

Where:
- DIFFICULTY is one of: EASY, MED, HARD
- IMPORTANCE is one of: LOW, MED, HIGH, CRITICAL

Examples:
TODO: [EASY] [HIGH] Add input validation for user email fields
TODO: [MED] [CRITICAL] Implement rate limiting on API endpoints
TODO: [HARD] [MED] Refactor database layer to use connection pooling

Focus on practical, actionable items. Be specific about what needs to be done."#;

                    let text_bytes: Vec<u8> = prompt.bytes().collect();
                    let _ = action_tx.send(Action::SendInput(session_id, text_bytes));
                    let _ = action_tx.send(Action::SendInput(session_id, vec![b'\r']));

                    // Remove from idle queue
                    state.idle_queue.retain(|&id| id != session_id);

                    // Switch to viewing the analyzer session
                    state.active_session_id = Some(session_id);
                    state.focus = FocusPanel::OutputPane;
                } else {
                    // No idle agent available, just show info
                    load_utility_content(state);
                    state.active_session_id = None;
                }
            } else {
                // Load utility content based on selected utility
                load_utility_content(state);
                // Clear active session so utility content shows
                state.active_session_id = None;
            }
            // Keep focus on utilities pane - user can navigate to output if needed
        }
        Action::ToggleUtilitySection => {
            state.utility_section = state.utility_section.toggle();
        }
        Action::ToggleConfigItem => {
            match state.selected_config {
                ConfigItem::ToggleBanner => {
                    state.banner_visible = !state.banner_visible;
                }
            }
            // Save config changes
            let config = persistence::GlobalConfig {
                banner_visible: state.banner_visible,
                left_panel_ratio: state.left_panel_ratio,
                workspace_ratio: state.workspace_ratio,
                sessions_ratio: state.sessions_ratio,
                todos_ratio: state.todos_ratio,
                output_split_ratio: state.output_split_ratio,
            };
            let _ = persistence::save_config(&config);
        }
        Action::ToggleBrownNoise => {
            state.brown_noise_playing = !state.brown_noise_playing;
            // Audio player is managed by runtime
        }

        // Notepad operations
        Action::NotepadChar(c) => {
            state.notepad_insert_char(c);
            // Auto-save notepad changes
            let _ = persistence::save_with_notepad(&state.workspaces, &state.sessions, &state.notepad_content);
        }
        Action::NotepadBackspace => {
            state.notepad_backspace();
            let _ = persistence::save_with_notepad(&state.workspaces, &state.sessions, &state.notepad_content);
        }
        Action::NotepadDelete => {
            state.notepad_delete();
            let _ = persistence::save_with_notepad(&state.workspaces, &state.sessions, &state.notepad_content);
        }
        Action::NotepadNewline => {
            state.notepad_insert_char('\n');
            let _ = persistence::save_with_notepad(&state.workspaces, &state.sessions, &state.notepad_content);
        }
        Action::NotepadCursorLeft => {
            state.notepad_cursor_left();
        }
        Action::NotepadCursorRight => {
            state.notepad_cursor_right();
        }
        Action::NotepadCursorHome => {
            state.notepad_cursor_home();
        }
        Action::NotepadCursorEnd => {
            state.notepad_cursor_end();
        }
        Action::NotepadPaste => {
            // Try to paste from clipboard
            if let Ok(mut ctx) = arboard::Clipboard::new() {
                if let Ok(text) = ctx.get_text() {
                    for c in text.chars() {
                        state.notepad_insert_char(c);
                    }
                    // Save after paste
                    let _ = persistence::save_with_notepad(&state.workspaces, &state.sessions, &state.notepad_content);
                }
            }
        }
        Action::NotepadDeleteWord => {
            state.notepad_delete_word();
            let _ = persistence::save_with_notepad(&state.workspaces, &state.sessions, &state.notepad_content);
        }
        Action::NotepadDeleteLine => {
            state.notepad_delete_line();
            let _ = persistence::save_with_notepad(&state.workspaces, &state.sessions, &state.notepad_content);
        }
        Action::NotepadDeleteWordForward => {
            state.notepad_delete_word_forward();
            let _ = persistence::save_with_notepad(&state.workspaces, &state.sessions, &state.notepad_content);
        }
        Action::NotepadDeleteToEnd => {
            state.notepad_delete_to_end();
            let _ = persistence::save_with_notepad(&state.workspaces, &state.sessions, &state.notepad_content);
        }
        Action::NotepadWordLeft => {
            state.notepad_word_left();
        }
        Action::NotepadWordRight => {
            state.notepad_word_right();
        }

        // Workspace operations
        Action::CreateWorkspace(path) => {
            if path.exists() && path.is_dir() {
                let workspace = Workspace::from_path(path);
                state.add_workspace(workspace);
            }
        }
        Action::SelectWorkspace(idx) => {
            if idx < state.workspaces.len() {
                state.selected_workspace_idx = idx;
                state.selected_session_idx = 0;
            }
        }
        Action::DeleteWorkspace(id) => {
            // Remove all sessions for this workspace first
            state.sessions.remove(&id);
            // Remove the workspace
            if let Some(idx) = state.workspaces.iter().position(|w| w.id == id) {
                state.workspaces.remove(idx);
                // Adjust selection
                if state.selected_workspace_idx >= state.workspaces.len() && !state.workspaces.is_empty() {
                    state.selected_workspace_idx = state.workspaces.len() - 1;
                }
                state.selected_session_idx = 0;
            }
            // Auto-save
            let _ = persistence::save(&state.workspaces, &state.sessions);
        }
        Action::ToggleWorkspaceStatus => {
            if let Some(ws) = state.workspaces.get_mut(state.selected_workspace_idx) {
                ws.toggle_status();
                // Auto-save
                let _ = persistence::save(&state.workspaces, &state.sessions);
            }
        }
        Action::InitiateDeleteWorkspace(id, name) => {
            state.pending_delete = Some(PendingDelete::Workspace(id, name));
        }
        Action::ConfirmDeleteWorkspace => {
            if let Some(PendingDelete::Workspace(id, _)) = state.pending_delete.take() {
                // Remove all sessions and PTYs for this workspace
                if let Some(sessions) = state.sessions.remove(&id) {
                    for session in sessions {
                        if let Some(mut handle) = state.pty_handles.remove(&session.id) {
                            let _ = handle.kill();
                        }
                        state.output_buffers.remove(&session.id);
                    }
                }
                // Remove the workspace
                if let Some(idx) = state.workspaces.iter().position(|w| w.id == id) {
                    state.workspaces.remove(idx);
                    if state.selected_workspace_idx >= state.workspaces.len() && !state.workspaces.is_empty() {
                        state.selected_workspace_idx = state.workspaces.len() - 1;
                    }
                    state.selected_session_idx = 0;
                }
                let _ = persistence::save(&state.workspaces, &state.sessions);
            }
        }

        // Session operations
        Action::CreateSession(agent_type) => {
            if let Some(workspace) = state.selected_workspace() {
                let session = Session::new(workspace.id, agent_type.clone());
                let session_id = session.id;
                let workspace_path = workspace.path.clone();
                let ws_idx = state.selected_workspace_idx;

                // Touch the workspace to update last_active_at
                if let Some(ws) = state.workspaces.get_mut(ws_idx) {
                    ws.touch();
                }

                // Calculate PTY size based on output pane (accounting for split view)
                let pty_rows = state.pane_rows();
                let cols = state.output_pane_cols();

                // Create vt100 parser with many more rows for scrollback
                let parser_rows = 500;
                let parser = vt100::Parser::new(parser_rows, cols, 0);
                state.output_buffers.insert(session_id, parser);

                // Spawn PTY with actual viewport size
                match pty_manager.spawn_session(
                    session_id,
                    agent_type,
                    &workspace_path,
                    pty_rows,
                    cols,
                    action_tx.clone(),
                ) {
                    Ok(handle) => {
                        state.pty_handles.insert(session_id, handle);
                        state.add_session(session);
                        state.active_session_id = Some(session_id);
                        // Keep focus on sessions pane when creating new sessions
                        state.focus = FocusPanel::SessionList;
                        // Select the newly created session in the list
                        let session_count = state.sessions_for_selected_workspace().len();
                        if session_count > 0 {
                            state.selected_session_idx = session_count - 1;
                        }
                        // Auto-save
                        let _ = persistence::save(&state.workspaces, &state.sessions);
                    }
                    Err(e) => {
                        eprintln!("Failed to spawn session: {}", e);
                        state.output_buffers.remove(&session_id);
                    }
                }

                state.input_mode = InputMode::Normal;
            }
        }
        Action::CreateTerminal => {
            if let Some(workspace) = state.selected_workspace() {
                // Count existing terminals to auto-generate name
                let terminal_count = state.sessions_for_selected_workspace()
                    .iter()
                    .filter(|s| s.agent_type.is_terminal())
                    .count();
                let name = format!("{}", terminal_count + 1);

                let agent_type = AgentType::Terminal(name);
                let session = Session::new(workspace.id, agent_type.clone());
                let session_id = session.id;
                let workspace_path = workspace.path.clone();
                let ws_idx = state.selected_workspace_idx;

                // Touch the workspace to update last_active_at
                if let Some(ws) = state.workspaces.get_mut(ws_idx) {
                    ws.touch();
                }

                // Calculate PTY size based on output pane (accounting for split view)
                let pty_rows = state.pane_rows();
                let cols = state.output_pane_cols();

                // Create vt100 parser with many more rows for scrollback
                let parser_rows = 500;
                let parser = vt100::Parser::new(parser_rows, cols, 0);
                state.output_buffers.insert(session_id, parser);

                // Spawn PTY (terminals don't resume)
                match pty_manager.spawn_session(
                    session_id,
                    agent_type,
                    &workspace_path,
                    pty_rows,
                    cols,
                    action_tx.clone(),
                ) {
                    Ok(handle) => {
                        state.pty_handles.insert(session_id, handle);
                        state.add_session(session);
                        state.active_session_id = Some(session_id);
                        // Keep focus on sessions pane when creating new sessions
                        state.focus = FocusPanel::SessionList;
                        // Select the newly created session in the list
                        let session_count = state.sessions_for_selected_workspace().len();
                        if session_count > 0 {
                            state.selected_session_idx = session_count - 1;
                        }
                        // Auto-save
                        let _ = persistence::save(&state.workspaces, &state.sessions);
                    }
                    Err(e) => {
                        eprintln!("Failed to spawn terminal: {}", e);
                        state.output_buffers.remove(&session_id);
                    }
                }
            }
        }
        Action::SelectSession(idx) => {
            let session_count = state.sessions_for_selected_workspace().len();
            if idx < session_count {
                state.selected_session_idx = idx;
            }
        }
        Action::ActivateSession(session_id) => {
            state.active_session_id = Some(session_id);
            state.output_scroll_offset = 0;
            // Keep focus on session list - user can navigate to output if needed
        }
        Action::RestartSession(session_id) => {
            // Find the session and its workspace, including start_command for terminals
            let session_info = state.sessions.values().flatten()
                .find(|s| s.id == session_id)
                .map(|s| (s.agent_type.clone(), s.workspace_id, s.start_command.clone()));

            if let Some((agent_type, workspace_id, start_command)) = session_info {
                // Find the workspace path
                let workspace_path = state.workspaces.iter()
                    .find(|w| w.id == workspace_id)
                    .map(|w| w.path.clone());

                if let Some(workspace_path) = workspace_path {
                    // Calculate PTY size based on output pane
                    let pty_rows = state.pane_rows();
                    let cols = state.output_pane_cols();

                    // Create new vt100 parser with large buffer for scrollback
                    let parser_rows = 500;
                    let parser = vt100::Parser::new(parser_rows, cols, 0);
                    state.output_buffers.insert(session_id, parser);

                    // For terminals, don't resume. For agents, resume.
                    let resume = agent_type.is_agent();

                    // Spawn new PTY with resume flag
                    match pty_manager.spawn_session_with_resume(
                        session_id,
                        agent_type.clone(),
                        &workspace_path,
                        pty_rows,
                        cols,
                        action_tx.clone(),
                        resume,
                    ) {
                        Ok(handle) => {
                            state.pty_handles.insert(session_id, handle);
                            // Mark session as running
                            if let Some(session) = state.get_session_mut(session_id) {
                                session.status = crate::models::SessionStatus::Running;
                            }
                            state.active_session_id = Some(session_id);
                            state.focus = FocusPanel::OutputPane;

                            // Send start command for terminals after a short delay
                            if agent_type.is_terminal() {
                                if let Some(cmd) = start_command {
                                    if !cmd.is_empty() {
                                        // Queue the command to be sent
                                        let tx = action_tx.clone();
                                        let sid = session_id;
                                        tokio::spawn(async move {
                                            // Wait for shell to initialize
                                            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
                                            let mut input = cmd.into_bytes();
                                            input.push(b'\n'); // Add newline to execute
                                            let _ = tx.send(Action::SendInput(sid, input));
                                        });
                                    }
                                }
                            }

                            // Auto-save
                            let _ = persistence::save(&state.workspaces, &state.sessions);
                        }
                        Err(e) => {
                            eprintln!("Failed to restart session: {}", e);
                            state.output_buffers.remove(&session_id);
                        }
                    }
                }
            }
        }
        Action::StopSession(session_id) => {
            // Send Ctrl+C to the session
            if let Some(handle) = state.pty_handles.get_mut(&session_id) {
                let _ = handle.send_input(&[0x03]); // Ctrl+C
            }
        }
        Action::KillSession(session_id) => {
            // Kill the PTY process
            if let Some(mut handle) = state.pty_handles.remove(&session_id) {
                let _ = handle.kill();
            }
            if let Some(session) = state.get_session_mut(session_id) {
                session.mark_stopped();
            }
            if state.active_session_id == Some(session_id) {
                state.active_session_id = None;
            }
            // Auto-save
            let _ = persistence::save(&state.workspaces, &state.sessions);
        }
        Action::DeleteSession(session_id) => {
            // Kill PTY if running, then remove session entirely
            if let Some(mut handle) = state.pty_handles.remove(&session_id) {
                let _ = handle.kill();
            }
            state.output_buffers.remove(&session_id);
            state.delete_session(session_id);
            // Adjust selection if needed
            let session_count = state.sessions_for_selected_workspace().len();
            if state.selected_session_idx >= session_count && session_count > 0 {
                state.selected_session_idx = session_count - 1;
            }
            // Auto-save
            let _ = persistence::save(&state.workspaces, &state.sessions);
        }
        Action::InitiateDeleteSession(id, name) => {
            state.pending_delete = Some(PendingDelete::Session(id, name));
        }
        Action::ConfirmDeleteSession => {
            if let Some(PendingDelete::Session(session_id, _)) = state.pending_delete.take() {
                // Kill PTY if running, then remove session entirely
                if let Some(mut handle) = state.pty_handles.remove(&session_id) {
                    let _ = handle.kill();
                }
                state.output_buffers.remove(&session_id);
                state.delete_session(session_id);
                // Clear active session if it was the deleted one
                if state.active_session_id == Some(session_id) {
                    state.active_session_id = None;
                }
                // Adjust selection if needed
                let session_count = state.sessions_for_selected_workspace().len();
                if state.selected_session_idx >= session_count && session_count > 0 {
                    state.selected_session_idx = session_count - 1;
                }
                let _ = persistence::save(&state.workspaces, &state.sessions);
            }
        }
        Action::CancelPendingDelete => {
            state.pending_delete = None;
        }

        // PTY interaction
        Action::SendInput(session_id, data) => {
            if let Some(handle) = state.pty_handles.get_mut(&session_id) {
                let _ = handle.send_input(&data);
            }
            // Touch the workspace containing this session
            if let Some(workspace_id) = state.sessions.iter()
                .find_map(|(ws_id, sessions)| {
                    if sessions.iter().any(|s| s.id == session_id) {
                        Some(*ws_id)
                    } else {
                        None
                    }
                })
            {
                if let Some(ws) = state.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    ws.touch();
                }
            }
        }
        Action::PtyOutput(session_id, data) => {
            if let Some(parser) = state.output_buffers.get_mut(&session_id) {
                parser.process(&data);
            }
            // Track activity
            state.last_activity.insert(session_id, std::time::Instant::now());
        }
        Action::SessionExited(session_id, _exit_code) => {
            state.pty_handles.remove(&session_id);
            if let Some(session) = state.get_session_mut(session_id) {
                session.mark_stopped();
            }
            // Auto-save
            let _ = persistence::save(&state.workspaces, &state.sessions);
        }

        // Todo operations
        Action::SelectNextTodo => {
            if let Some(ws) = state.selected_workspace() {
                // Count todos based on current tab
                let count = ws.todos.iter().filter(|t| {
                    match state.selected_todos_tab {
                        TodosTab::Active => !t.is_archived(),
                        TodosTab::Archived => t.is_archived(),
                    }
                }).count();
                if count > 0 {
                    state.selected_todo_idx = (state.selected_todo_idx + 1).min(count - 1);
                }
            }
        }
        Action::SelectPrevTodo => {
            if state.selected_todo_idx > 0 {
                state.selected_todo_idx -= 1;
            }
        }
        Action::EnterCreateTodoMode => {
            state.input_mode = InputMode::CreateTodo;
            state.input_buffer.clear();
        }
        Action::CreateTodo(description) => {
            if let Some(ws) = state.workspaces.get_mut(state.selected_workspace_idx) {
                ws.add_todo(description);
                let _ = persistence::save(&state.workspaces, &state.sessions);
            }
            state.input_mode = InputMode::Normal;
            state.input_buffer.clear();
        }
        Action::MarkTodoDone => {
            // Get the todo ID from the filtered list (Active tab only)
            let todo_id = state.selected_workspace()
                .and_then(|ws| {
                    ws.todos.iter()
                        .filter(|t| !t.is_archived())
                        .nth(state.selected_todo_idx)
                        .map(|t| t.id)
                });

            if let Some(id) = todo_id {
                if let Some(ws) = state.workspaces.get_mut(state.selected_workspace_idx) {
                    if let Some(todo) = ws.get_todo_mut(id) {
                        todo.mark_done();
                        let _ = persistence::save(&state.workspaces, &state.sessions);
                    }
                }
            }
        }
        Action::RunSelectedTodo => {
            use crate::app::TodoPaneMode;
            use crate::models::SessionStatus;

            // Get the selected todo from filtered list (Active tab only, non-archived)
            let selected_todo = state.selected_workspace()
                .and_then(|ws| {
                    ws.todos.iter()
                        .filter(|t| !t.is_archived())
                        .nth(state.selected_todo_idx)
                        .map(|t| (t.id, t.description.clone(), t.is_pending(), t.is_queued()))
                });

            let (todo_id, description, is_pending, is_queued) = match selected_todo {
                Some(data) => data,
                None => return Ok(()),
            };

            // Check if dispatchable
            if !is_pending && !is_queued {
                return Ok(());
            }

            // Check if there's already an in-progress todo in this workspace
            let has_in_progress = state.workspaces.get(state.selected_workspace_idx)
                .map(|ws| ws.has_in_progress_todo())
                .unwrap_or(false);

            // Count filtered todos for navigation
            let todo_count = state.selected_workspace()
                .map(|ws| ws.todos.iter().filter(|t| !t.is_archived()).count())
                .unwrap_or(0);

            // In autorun mode with an in-progress todo, queue this one instead
            if state.todo_pane_mode == TodoPaneMode::Autorun && has_in_progress {
                if let Some(ws) = state.workspaces.get_mut(state.selected_workspace_idx) {
                    if let Some(todo) = ws.get_todo_mut(todo_id) {
                        if todo.is_pending() {
                            todo.mark_queued();
                            let _ = persistence::save(&state.workspaces, &state.sessions);
                        }
                    }
                }
                // Move to next todo
                if state.selected_todo_idx + 1 < todo_count {
                    state.selected_todo_idx += 1;
                }
                return Ok(());
            }

            // Find an idle session to dispatch to
            let current_workspace_id = state.workspaces.get(state.selected_workspace_idx)
                .map(|ws| ws.id);

            let target_session_id = state.active_session_id
                .filter(|id| state.idle_queue.contains(id))
                .or_else(|| {
                    current_workspace_id.and_then(|ws_id| {
                        state.sessions.get(&ws_id)
                            .and_then(|sessions| {
                                sessions.iter()
                                    .find(|s| {
                                        s.agent_type.is_agent() &&
                                        s.status == SessionStatus::Running &&
                                        state.idle_queue.contains(&s.id)
                                    })
                                    .map(|s| s.id)
                            })
                    })
                });

            if let Some(session_id) = target_session_id {
                // Mark as in-progress and assign to session
                if let Some(ws) = state.workspaces.get_mut(state.selected_workspace_idx) {
                    if let Some(todo) = ws.get_todo_mut(todo_id) {
                        todo.assign_to(session_id);
                    }
                }
                // Remove from idle queue since it's now working
                state.idle_queue.retain(|&id| id != session_id);

                // Queue the text + Enter as SendInput actions
                let text_bytes: Vec<u8> = description.bytes().collect();
                let _ = action_tx.send(Action::SendInput(session_id, text_bytes));
                let _ = action_tx.send(Action::SendInput(session_id, vec![b'\r']));
                let _ = persistence::save(&state.workspaces, &state.sessions);

                // Move to next todo if available
                if state.selected_todo_idx + 1 < todo_count {
                    state.selected_todo_idx += 1;
                }
            } else if state.todo_pane_mode == TodoPaneMode::Autorun {
                // No idle session available, queue this todo for later
                if let Some(ws) = state.workspaces.get_mut(state.selected_workspace_idx) {
                    if let Some(todo) = ws.get_todo_mut(todo_id) {
                        if todo.is_pending() {
                            todo.mark_queued();
                            let _ = persistence::save(&state.workspaces, &state.sessions);
                        }
                    }
                }
                if state.selected_todo_idx + 1 < todo_count {
                    state.selected_todo_idx += 1;
                }
            }
        }
        Action::ToggleTodoPaneMode => {
            state.todo_pane_mode = state.todo_pane_mode.toggle();
        }
        Action::InitiateDeleteTodo(id, desc) => {
            state.pending_delete = Some(PendingDelete::Todo(id, desc));
        }
        Action::ConfirmDeleteTodo => {
            if let Some(PendingDelete::Todo(id, _)) = state.pending_delete.take() {
                if let Some(ws) = state.workspaces.get_mut(state.selected_workspace_idx) {
                    ws.remove_todo(id);
                    // Adjust selection based on filtered list count
                    let filtered_count = ws.todos.iter().filter(|t| {
                        match state.selected_todos_tab {
                            TodosTab::Active => !t.is_archived(),
                            TodosTab::Archived => t.is_archived(),
                        }
                    }).count();
                    if filtered_count > 0 && state.selected_todo_idx >= filtered_count {
                        state.selected_todo_idx = filtered_count - 1;
                    } else if filtered_count == 0 {
                        state.selected_todo_idx = 0;
                    }
                    let _ = persistence::save(&state.workspaces, &state.sessions);
                }
            }
        }

        // Auto-dispatch todo to an idle agent
        Action::DispatchTodoToSession(session_id, todo_id, description) => {
            // Find the workspace containing this session
            if let Some(workspace_id) = state.workspace_id_for_session(session_id) {
                // Mark the todo as in-progress
                if let Some(ws) = state.get_workspace_mut(workspace_id) {
                    if let Some(todo) = ws.get_todo_mut(todo_id) {
                        todo.assign_to(session_id);
                    }
                }

                // Remove from idle queue since it's now working
                state.idle_queue.retain(|&id| id != session_id);

                // Queue the text + Enter as SendInput actions (same path as manual typing)
                let text_bytes: Vec<u8> = description.bytes().collect();
                let _ = action_tx.send(Action::SendInput(session_id, text_bytes));
                let _ = action_tx.send(Action::SendInput(session_id, vec![b'\r']));

                // Save state
                let _ = persistence::save(&state.workspaces, &state.sessions);
            }
        }

        // Mark todo as ready for review (agent went idle after dispatch)
        Action::MarkTodoReadyForReview(todo_id) => {
            // Find the workspace containing this todo
            for ws in state.workspaces.iter_mut() {
                if let Some(todo) = ws.get_todo_mut(todo_id) {
                    todo.mark_ready_for_review();
                    let _ = persistence::save(&state.workspaces, &state.sessions);
                    break;
                }
            }
        }

        // Todo suggestion actions
        Action::TriggerTodoSuggestion => {
            // Handled in utilities.rs via load_utility_content for SuggestTodos
            // This action is sent when SuggestTodos utility is activated
        }
        Action::AddSuggestedTodo(description) => {
            if let Some(ws) = state.workspaces.get_mut(state.selected_workspace_idx) {
                ws.add_suggested_todo(description);
                let _ = persistence::save(&state.workspaces, &state.sessions);
            }
        }
        Action::ApproveSuggestedTodo(todo_id) => {
            if let Some(ws) = state.workspaces.get_mut(state.selected_workspace_idx) {
                if let Some(todo) = ws.get_todo_mut(todo_id) {
                    todo.approve();
                    let _ = persistence::save(&state.workspaces, &state.sessions);
                }
            }
        }
        Action::ApproveAllSuggestedTodos => {
            if let Some(ws) = state.workspaces.get_mut(state.selected_workspace_idx) {
                for todo in ws.todos.iter_mut() {
                    if todo.is_suggested() {
                        todo.approve();
                    }
                }
                let _ = persistence::save(&state.workspaces, &state.sessions);
            }
        }
        Action::ArchiveTodo(todo_id) => {
            if let Some(ws) = state.workspaces.get_mut(state.selected_workspace_idx) {
                if let Some(todo) = ws.get_todo_mut(todo_id) {
                    todo.archive();
                    let _ = persistence::save(&state.workspaces, &state.sessions);
                }
            }
        }

        Action::ToggleTodosTab => {
            state.selected_todos_tab = state.selected_todos_tab.toggle();
            state.selected_todo_idx = 0; // Reset selection when switching tabs
        }
    }

    Ok(())
}
