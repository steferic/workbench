use crate::app::pty_ops::resize_ptys_to_panes;
use crate::app::selection::{
    clear_active_text_selection, clear_all_pinned_selections, copy_active_selection,
    pane_text_position,
};
use crate::app::session_start::start_workspace_sessions;
use crate::app::workspace_nav::{
    cycle_next_working_workspace, cycle_prev_working_workspace, move_workspace_selection,
    set_selected_workspace, workspace_index_at_position,
};
use crate::app::{
    Action, AppState, Divider, FocusPanel, InputMode, TextSelection, UtilityItem, UtilitySection,
};
use crate::persistence::GlobalConfig;
use crate::pty::PtyManager;
use anyhow::Result;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::{report_runtime_error, save_config, save_state_with_notepad};

fn bracketed_paste_payload(text: &str) -> Vec<u8> {
    let mut data = Vec::with_capacity(text.len() + 10);
    data.extend_from_slice(b"\x1b[200~");
    data.extend_from_slice(text.as_bytes());
    data.extend_from_slice(b"\x1b[201~");
    data
}

fn paste_target_session_id(state: &AppState) -> Option<Uuid> {
    match state.ui.focus {
        FocusPanel::PinnedTerminalPane(idx) => state.pinned_terminal_id_at(idx),
        _ => state.ui.active_session_id,
    }
}

fn is_in_area(x: u16, y: u16, area: (u16, u16, u16, u16)) -> bool {
    let (ax, ay, aw, ah) = area;
    x >= ax && x < ax + aw && y >= ay && y < ay + ah
}

pub fn handle_navigation_action(
    state: &mut AppState,
    action: Action,
    pty_manager: &PtyManager,
    pty_tx: &mpsc::Sender<Action>,
) -> Result<()> {
    match action {
        Action::MoveUp => match state.ui.focus {
            FocusPanel::WorkspaceList => {
                move_workspace_selection(state, true, pty_manager, pty_tx);
            }
            FocusPanel::SessionList => {
                state.select_prev_session();
            }
            _ => {}
        },
        Action::MoveDown => match state.ui.focus {
            FocusPanel::WorkspaceList => {
                move_workspace_selection(state, false, pty_manager, pty_tx);
            }
            FocusPanel::SessionList => {
                state.select_next_session();
            }
            _ => {}
        },
        Action::FocusLeft => {
            let pinned_count = state.pinned_count();
            state.ui.focus = match state.ui.focus {
                FocusPanel::WorkspaceList => {
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
            let pinned_count = state.pinned_count();
            let prev_focus = state.ui.focus;
            state.ui.focus = match state.ui.focus {
                FocusPanel::WorkspaceList => FocusPanel::SessionList,
                FocusPanel::SessionList => FocusPanel::TodosPane,
                FocusPanel::TodosPane => FocusPanel::UtilitiesPane,
                FocusPanel::UtilitiesPane => FocusPanel::OutputPane,
                FocusPanel::OutputPane => {
                    if state.should_show_split() && pinned_count > 0 {
                        FocusPanel::PinnedTerminalPane(0)
                    } else {
                        FocusPanel::WorkspaceList
                    }
                }
                FocusPanel::PinnedTerminalPane(idx) => {
                    if idx + 1 < pinned_count {
                        FocusPanel::PinnedTerminalPane(idx + 1)
                    } else {
                        FocusPanel::WorkspaceList
                    }
                }
            };
            if prev_focus == FocusPanel::WorkspaceList && state.ui.focus == FocusPanel::SessionList
            {
                start_workspace_sessions(state, pty_manager, pty_tx);
            }
        }
        Action::NextPinnedPane => {
            let count = state.pinned_count();
            if count > 0 {
                state.ui.focused_pinned_pane = (state.ui.focused_pinned_pane + 1) % count;
                state.ui.focus = FocusPanel::PinnedTerminalPane(state.ui.focused_pinned_pane);
            }
        }
        Action::PrevPinnedPane => {
            let count = state.pinned_count();
            if count > 0 {
                state.ui.focused_pinned_pane = if state.ui.focused_pinned_pane == 0 {
                    count - 1
                } else {
                    state.ui.focused_pinned_pane - 1
                };
                state.ui.focus = FocusPanel::PinnedTerminalPane(state.ui.focused_pinned_pane);
            }
        }
        Action::ScrollOutputUp => {
            if let FocusPanel::PinnedTerminalPane(idx) = state.ui.focus {
                if let Some(offset) = state.ui.pinned_scroll_offsets.get_mut(idx) {
                    *offset = offset.saturating_add(3);
                }
            } else {
                state.ui.output_scroll_offset = state.ui.output_scroll_offset.saturating_add(3);
            }
        }
        Action::ScrollOutputDown => {
            if let FocusPanel::PinnedTerminalPane(idx) = state.ui.focus {
                if let Some(offset) = state.ui.pinned_scroll_offsets.get_mut(idx) {
                    *offset = offset.saturating_sub(3);
                }
            } else {
                state.ui.output_scroll_offset = state.ui.output_scroll_offset.saturating_sub(3);
            }
        }
        Action::MouseScrollUp(x, y) => {
            if let Some(area) = state.ui.workspace_area {
                if is_in_area(x, y, area) {
                    state.ui.focus = FocusPanel::WorkspaceList;
                    move_workspace_selection(state, true, pty_manager, pty_tx);
                    return Ok(());
                }
            }

            for (idx, area_opt) in state.ui.pinned_pane_areas.iter().enumerate() {
                if let Some(area) = *area_opt {
                    if is_in_area(x, y, area) {
                        state.ui.focus = FocusPanel::PinnedTerminalPane(idx);
                        state.ui.focused_pinned_pane = idx;
                        if let Some(offset) = state.ui.pinned_scroll_offsets.get_mut(idx) {
                            *offset = offset.saturating_add(3);
                        }
                        return Ok(());
                    }
                }
            }

            if let Some(area) = state.ui.output_pane_area {
                if is_in_area(x, y, area) {
                    state.ui.focus = FocusPanel::OutputPane;
                    state.ui.output_scroll_offset = state.ui.output_scroll_offset.saturating_add(3);
                }
            }
        }
        Action::MouseScrollDown(x, y) => {
            if let Some(area) = state.ui.workspace_area {
                if is_in_area(x, y, area) {
                    state.ui.focus = FocusPanel::WorkspaceList;
                    move_workspace_selection(state, false, pty_manager, pty_tx);
                    return Ok(());
                }
            }

            for (idx, area_opt) in state.ui.pinned_pane_areas.iter().enumerate() {
                if let Some(area) = *area_opt {
                    if is_in_area(x, y, area) {
                        state.ui.focus = FocusPanel::PinnedTerminalPane(idx);
                        state.ui.focused_pinned_pane = idx;
                        if let Some(offset) = state.ui.pinned_scroll_offsets.get_mut(idx) {
                            *offset = offset.saturating_sub(3);
                        }
                        return Ok(());
                    }
                }
            }

            if let Some(area) = state.ui.output_pane_area {
                if is_in_area(x, y, area) {
                    state.ui.focus = FocusPanel::OutputPane;
                    state.ui.output_scroll_offset = state.ui.output_scroll_offset.saturating_sub(3);
                }
            }
        }
        Action::CycleNextWorkspace => {
            cycle_next_working_workspace(state);
        }
        Action::CyclePrevWorkspace => {
            cycle_prev_working_workspace(state);
        }
        Action::CycleNextSession => {
            // Cycle through agents only (Agents -> Parallel), skip terminals
            // Get parallel task session IDs first
            let parallel_session_ids: Vec<Uuid> = state
                .selected_workspace()
                .map(|ws| {
                    ws.parallel_tasks
                        .iter()
                        .flat_map(|t| t.attempts.iter().map(|a| a.session_id))
                        .collect()
                })
                .unwrap_or_default();

            // Build visual order indices (agents only)
            let session_info: Option<(usize, Uuid)> = {
                let sessions = state.sessions_for_selected_workspace();

                // Agents: non-terminal, non-parallel
                let agent_indices: Vec<usize> = sessions
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| {
                        !s.agent_type.is_terminal() && !parallel_session_ids.contains(&s.id)
                    })
                    .map(|(i, _)| i)
                    .collect();

                // Parallel sessions (these are also agents)
                let parallel_indices: Vec<usize> = sessions
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| parallel_session_ids.contains(&s.id))
                    .map(|(i, _)| i)
                    .collect();

                // Combined visual order (agents only, no terminals)
                let visual_order: Vec<usize> =
                    agent_indices.into_iter().chain(parallel_indices).collect();

                if !visual_order.is_empty() {
                    // Find current position in visual order
                    let current_pos = visual_order
                        .iter()
                        .position(|&idx| idx == state.ui.selected_session_idx);

                    // Move to next in visual order (or first if not found)
                    let next_visual_pos = match current_pos {
                        Some(pos) => (pos + 1) % visual_order.len(),
                        None => 0,
                    };

                    let next_idx = visual_order[next_visual_pos];
                    sessions.get(next_idx).map(|s| (next_idx, s.id))
                } else {
                    None
                }
            };

            if let Some((next_idx, session_id)) = session_info {
                if state.ui.active_session_id != Some(session_id) {
                    clear_active_text_selection(state);
                }
                state.ui.selected_session_idx = next_idx;
                state.ui.active_session_id = Some(session_id);
                state.ui.output_scroll_offset = 0;
                state.ui.focus = FocusPanel::OutputPane;
            }
        }
        Action::CyclePrevSession => {
            let parallel_session_ids: Vec<Uuid> = state
                .selected_workspace()
                .map(|ws| {
                    ws.parallel_tasks
                        .iter()
                        .flat_map(|t| t.attempts.iter().map(|a| a.session_id))
                        .collect()
                })
                .unwrap_or_default();

            let session_info: Option<(usize, Uuid)> = {
                let sessions = state.sessions_for_selected_workspace();

                let agent_indices: Vec<usize> = sessions
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| {
                        !s.agent_type.is_terminal() && !parallel_session_ids.contains(&s.id)
                    })
                    .map(|(i, _)| i)
                    .collect();

                let parallel_indices: Vec<usize> = sessions
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| parallel_session_ids.contains(&s.id))
                    .map(|(i, _)| i)
                    .collect();

                let visual_order: Vec<usize> =
                    agent_indices.into_iter().chain(parallel_indices).collect();

                if !visual_order.is_empty() {
                    let current_pos = visual_order
                        .iter()
                        .position(|&idx| idx == state.ui.selected_session_idx);

                    let prev_visual_pos = match current_pos {
                        Some(pos) => (pos + visual_order.len() - 1) % visual_order.len(),
                        None => visual_order.len() - 1,
                    };

                    let prev_idx = visual_order[prev_visual_pos];
                    sessions.get(prev_idx).map(|s| (prev_idx, s.id))
                } else {
                    None
                }
            };

            if let Some((prev_idx, session_id)) = session_info {
                if state.ui.active_session_id != Some(session_id) {
                    clear_active_text_selection(state);
                }
                state.ui.selected_session_idx = prev_idx;
                state.ui.active_session_id = Some(session_id);
                state.ui.output_scroll_offset = 0;
                state.ui.focus = FocusPanel::OutputPane;
            }
        }
        Action::MouseClick(x, y) => {
            // Simplified MouseClick logic using stored areas
            let (w, h) = state.system.terminal_size;
            let main_height = h.saturating_sub(1);
            let divider_tolerance = 1u16;

            let left_width = (w as f32 * state.ui.left_panel_ratio) as u16;
            if x >= left_width.saturating_sub(divider_tolerance)
                && x <= left_width + divider_tolerance
                && y < main_height
            {
                state.ui.dragging_divider = Some(Divider::LeftRight);
                state.ui.drag_start_pos = Some((x, y));
                state.ui.drag_start_ratio = state.ui.left_panel_ratio;
                return Ok(());
            }

            let workspace_height = (main_height as f32 * state.ui.workspace_ratio) as u16;
            if x < left_width
                && y >= workspace_height.saturating_sub(divider_tolerance)
                && y <= workspace_height + divider_tolerance
            {
                state.ui.dragging_divider = Some(Divider::WorkspaceSession);
                state.ui.drag_start_pos = Some((x, y));
                state.ui.drag_start_ratio = state.ui.workspace_ratio;
                return Ok(());
            }

            let lower_left_height = main_height.saturating_sub(workspace_height);
            let sessions_height = (lower_left_height as f32 * state.ui.sessions_ratio) as u16;
            let sessions_todos_divider_y = workspace_height + sessions_height;

            if x < left_width
                && y >= sessions_todos_divider_y.saturating_sub(divider_tolerance)
                && y <= sessions_todos_divider_y + divider_tolerance
            {
                state.ui.dragging_divider = Some(Divider::SessionsTodos);
                state.ui.drag_start_pos = Some((x, y));
                state.ui.drag_start_ratio = state.ui.sessions_ratio;
                return Ok(());
            }

            let remaining_height = lower_left_height.saturating_sub(sessions_height);
            let todos_height = (remaining_height as f32 * state.ui.todos_ratio) as u16;
            let todos_utilities_divider_y = sessions_todos_divider_y + todos_height;

            if x < left_width
                && y >= todos_utilities_divider_y.saturating_sub(divider_tolerance)
                && y <= todos_utilities_divider_y + divider_tolerance
            {
                state.ui.dragging_divider = Some(Divider::TodosUtilities);
                state.ui.drag_start_pos = Some((x, y));
                state.ui.drag_start_ratio = state.ui.todos_ratio;
                return Ok(());
            }

            if state.should_show_split() {
                if let Some((ox, _, ow, _)) = state.ui.output_pane_area {
                    let divider_x = ox + ow;
                    if x >= divider_x.saturating_sub(divider_tolerance)
                        && x <= divider_x + divider_tolerance
                        && y < main_height
                    {
                        state.ui.dragging_divider = Some(Divider::OutputPinned);
                        state.ui.drag_start_pos = Some((x, y));
                        state.ui.drag_start_ratio = state.ui.output_split_ratio;
                        return Ok(());
                    }
                }

                let pinned_count = state.pinned_count();
                if pinned_count > 1 {
                    for pane_idx in 0..(pinned_count - 1) {
                        if let Some((_, py, _, ph)) = state.ui.pinned_pane_areas[pane_idx] {
                            let divider_y = py + ph;
                            if y >= divider_y.saturating_sub(divider_tolerance)
                                && y <= divider_y + divider_tolerance
                            {
                                if let Some((px, _, pw, _)) = state.ui.pinned_pane_areas[0] {
                                    if x >= px && x < px + pw {
                                        state.ui.dragging_divider =
                                            Some(Divider::PinnedPanes(pane_idx));
                                        state.ui.drag_start_pos = Some((x, y));
                                        state.ui.drag_start_ratio =
                                            state.ui.pinned_pane_ratios[pane_idx];
                                        return Ok(());
                                    }
                                }
                            }
                        }
                    }
                }
            }

            state.ui.text_selection = TextSelection::default();
            clear_all_pinned_selections(state);

            if let Some(area) = state.ui.workspace_area {
                if is_in_area(x, y, area) {
                    state.ui.focus = FocusPanel::WorkspaceList;
                    if let Some(workspace_idx) = workspace_index_at_position(state, x, y) {
                        set_selected_workspace(state, workspace_idx, pty_manager, pty_tx);
                    }
                    return Ok(());
                }
            }

            if let Some(area) = state.ui.session_area {
                if is_in_area(x, y, area) {
                    state.ui.focus = FocusPanel::SessionList;
                    return Ok(());
                }
            }

            if let Some(area) = state.ui.todos_area {
                if is_in_area(x, y, area) {
                    state.ui.focus = FocusPanel::TodosPane;
                    return Ok(());
                }
            }

            if let Some(area) = state.ui.utilities_area {
                if is_in_area(x, y, area) {
                    state.ui.focus = FocusPanel::UtilitiesPane;
                    return Ok(());
                }
            }

            for (idx, area_opt) in state.ui.pinned_pane_areas.iter().enumerate() {
                if let Some(area) = *area_opt {
                    if is_in_area(x, y, area) {
                        state.ui.focus = FocusPanel::PinnedTerminalPane(idx);
                        state.ui.focused_pinned_pane = idx;
                        if state.pinned_terminal_output_at(idx).is_some() {
                            if let Some((row, col)) = pane_text_position(
                                area,
                                x,
                                y,
                                state.ui.pinned_content_lengths[idx],
                                state.ui.pinned_scroll_offsets[idx],
                            ) {
                                if let Some(sel) = state.ui.pinned_text_selections.get_mut(idx) {
                                    *sel = TextSelection {
                                        start: Some((row, col)),
                                        end: Some((row, col)),
                                        is_dragging: true,
                                    };
                                }
                            }
                        }
                        return Ok(());
                    }
                }
            }

            if let Some(area) = state.ui.output_pane_area {
                if is_in_area(x, y, area) {
                    state.ui.focus = FocusPanel::OutputPane;
                    if state.active_output().is_some() {
                        if let Some((row, col)) = pane_text_position(
                            area,
                            x,
                            y,
                            state.ui.output_content_length,
                            state.ui.output_scroll_offset,
                        ) {
                            state.ui.text_selection = TextSelection {
                                start: Some((row, col)),
                                end: Some((row, col)),
                                is_dragging: true,
                            };
                        }
                    }
                    return Ok(());
                }
            }
        }
        Action::MouseDrag(x, y) => {
            if let Some(divider) = state.ui.dragging_divider {
                let (w, h) = state.system.terminal_size;
                let main_height = h.saturating_sub(1);

                match divider {
                    Divider::LeftRight => {
                        let new_ratio = (x as f32 / w as f32).clamp(0.15, 0.50);
                        state.ui.left_panel_ratio = new_ratio;
                    }
                    Divider::WorkspaceSession => {
                        let new_ratio = (y as f32 / main_height as f32).clamp(0.20, 0.80);
                        state.ui.workspace_ratio = new_ratio;
                    }
                    Divider::SessionsTodos => {
                        let workspace_height =
                            (main_height as f32 * state.ui.workspace_ratio) as u16;
                        let lower_left_height = main_height.saturating_sub(workspace_height);
                        let y_in_lower_left = y.saturating_sub(workspace_height);
                        let new_ratio =
                            (y_in_lower_left as f32 / lower_left_height as f32).clamp(0.15, 0.70);
                        state.ui.sessions_ratio = new_ratio;
                    }
                    Divider::TodosUtilities => {
                        let workspace_height =
                            (main_height as f32 * state.ui.workspace_ratio) as u16;
                        let lower_left_height = main_height.saturating_sub(workspace_height);
                        let sessions_height =
                            (lower_left_height as f32 * state.ui.sessions_ratio) as u16;
                        let remaining_height = lower_left_height.saturating_sub(sessions_height);
                        let y_in_remaining = y
                            .saturating_sub(workspace_height)
                            .saturating_sub(sessions_height);
                        let new_ratio =
                            (y_in_remaining as f32 / remaining_height as f32).clamp(0.20, 0.80);
                        state.ui.todos_ratio = new_ratio;
                    }
                    Divider::OutputPinned => {
                        let left_width = (w as f32 * state.ui.left_panel_ratio) as u16;
                        let right_panel_width = w.saturating_sub(left_width);
                        let x_in_right = x.saturating_sub(left_width);
                        let new_ratio =
                            (x_in_right as f32 / right_panel_width as f32).clamp(0.20, 0.80);
                        state.ui.output_split_ratio = new_ratio;
                    }
                    Divider::PinnedPanes(pane_idx) => {
                        let count = state.pinned_count();
                        if count > 1 && pane_idx < count - 1 {
                            let mut ratios = state.ui.pinned_pane_ratios;
                            let sum: f32 = ratios.iter().take(count).sum();

                            if let Some((_, py, _, _)) = state.ui.pinned_pane_areas[0] {
                                let pinned_total_height = state
                                    .ui
                                    .pinned_pane_areas
                                    .iter()
                                    .take(count)
                                    .filter_map(|a| a.map(|(_, _, _, h)| h))
                                    .sum::<u16>();

                                let y_in_pinned = y.saturating_sub(py) as f32;
                                let new_split = y_in_pinned / pinned_total_height as f32;

                                let combined_ratio = ratios[pane_idx] + ratios[pane_idx + 1];
                                let ratio_above: f32 = ratios.iter().take(pane_idx).sum();

                                let new_upper_ratio = ((new_split - ratio_above / sum) * sum)
                                    .clamp(0.1, combined_ratio - 0.1);
                                ratios[pane_idx] = new_upper_ratio;
                                ratios[pane_idx + 1] = combined_ratio - new_upper_ratio;

                                state.ui.pinned_pane_ratios = ratios;
                            }
                        }
                    }
                }
                return Ok(());
            }

            // Store mouse position for tick-based smooth scrolling
            state.ui.drag_mouse_pos = Some((x, y));

            // Update selection end position for main output pane
            if state.ui.text_selection.is_dragging {
                if let Some((ax, ay, aw, ah)) = state.ui.output_pane_area {
                    if let Some((row, col)) = pane_text_position(
                        (ax, ay, aw, ah),
                        x,
                        y,
                        state.ui.output_content_length,
                        state.ui.output_scroll_offset,
                    ) {
                        state.ui.text_selection.end = Some((row, col));
                    }
                }
            }

            // Update selection end position for pinned panes
            for (idx, sel) in state.ui.pinned_text_selections.iter_mut().enumerate() {
                if sel.is_dragging {
                    if let Some((ax, ay, aw, ah)) = state.ui.pinned_pane_areas[idx] {
                        if let Some((row, col)) = pane_text_position(
                            (ax, ay, aw, ah),
                            x,
                            y,
                            state.ui.pinned_content_lengths[idx],
                            state.ui.pinned_scroll_offsets[idx],
                        ) {
                            sel.end = Some((row, col));
                        }
                    }
                }
            }
        }
        Action::MouseUp(x, y) => {
            if state.ui.dragging_divider.is_some() {
                state.ui.dragging_divider = None;
                state.ui.drag_start_pos = None;
                resize_ptys_to_panes(state);
                let config = GlobalConfig {
                    banner_visible: state.ui.banner_visible,
                    left_panel_ratio: state.ui.left_panel_ratio,
                    workspace_ratio: state.ui.workspace_ratio,
                    sessions_ratio: state.ui.sessions_ratio,
                    todos_ratio: state.ui.todos_ratio,
                    output_split_ratio: state.ui.output_split_ratio,
                    agent_done_sound_enabled: state.system.agent_done_sound_enabled,
                };
                save_config(state, &config, "failed to save pane layout config");
                return Ok(());
            }

            if state.ui.text_selection.is_dragging {
                if let Some(area) = state.ui.output_pane_area {
                    if let Some((row, col)) = pane_text_position(
                        area,
                        x,
                        y,
                        state.ui.output_content_length,
                        state.ui.output_scroll_offset,
                    ) {
                        state.ui.text_selection.end = Some((row, col));
                    }
                }
                state.ui.text_selection.is_dragging = false;
                if state.ui.text_selection.start == state.ui.text_selection.end {
                    state.ui.text_selection = TextSelection::default();
                }
            }

            for (idx, sel) in state.ui.pinned_text_selections.iter_mut().enumerate() {
                if sel.is_dragging {
                    if let Some(area) = state.ui.pinned_pane_areas[idx] {
                        if let Some((row, col)) = pane_text_position(
                            area,
                            x,
                            y,
                            state.ui.pinned_content_lengths[idx],
                            state.ui.pinned_scroll_offsets[idx],
                        ) {
                            sel.end = Some((row, col));
                        }
                    }
                    sel.is_dragging = false;
                    if sel.start == sel.end {
                        *sel = TextSelection::default();
                    }
                }
            }

            // Clear drag position tracking
            state.ui.drag_mouse_pos = None;
        }
        Action::CopySelection => {
            let _ = copy_active_selection(state);
            state.ui.text_selection = TextSelection::default();
            clear_all_pinned_selections(state);
        }
        Action::Paste(text) => {
            if state.ui.input_mode != InputMode::Normal {
                return Ok(());
            }
            // Check if focused on Notepad section - paste to TextArea instead of PTY
            if state.ui.focus == FocusPanel::UtilitiesPane
                && state.ui.utility_section == UtilitySection::Notepad
            {
                if let Some(textarea) = state.current_notepad() {
                    textarea.insert_str(&text);
                }
                save_state_with_notepad(state, "failed to save notepad paste");
            } else if let Some(session_id) = paste_target_session_id(state) {
                let data = bracketed_paste_payload(&text);
                let send_error = state
                    .system
                    .pty_handles
                    .get_mut(&session_id)
                    .and_then(|handle| handle.send_input(&data).err());
                if let Some(err) = send_error {
                    report_runtime_error(
                        state,
                        "failed to paste into PTY",
                        err,
                        "Failed to paste into session",
                    );
                }
                if let Some(workspace_id) = state.workspace_id_for_session(session_id) {
                    if let Some(ws) = state.get_workspace_mut(workspace_id) {
                        ws.touch();
                    }
                }
            }
        }
        Action::ClearSelection => {
            state.ui.text_selection = TextSelection::default();
            clear_all_pinned_selections(state);
        }
        Action::SelectNextUtility => {
            match state.ui.utility_section {
                UtilitySection::Utilities => {
                    let tools = UtilityItem::tools();
                    let current_idx = tools
                        .iter()
                        .position(|u| *u == state.ui.selected_utility)
                        .unwrap_or(0);
                    if current_idx < tools.len() - 1 {
                        state.ui.selected_utility = tools[current_idx + 1];
                    }
                }
                UtilitySection::Sounds => {
                    let sounds = UtilityItem::sounds();
                    let current_idx = sounds
                        .iter()
                        .position(|u| *u == state.ui.selected_sound)
                        .unwrap_or(0);
                    if current_idx < sounds.len() - 1 {
                        state.ui.selected_sound = sounds[current_idx + 1];
                    }
                }
                UtilitySection::GlobalConfig => {
                    // Navigate config tree
                    if state.ui.config_tree_selected
                        < state.ui.config_tree_nodes.len().saturating_sub(1)
                    {
                        state.ui.config_tree_selected += 1;
                    }
                }
                UtilitySection::Notepad => {}
            }
        }
        Action::SelectPrevUtility => {
            match state.ui.utility_section {
                UtilitySection::Utilities => {
                    let tools = UtilityItem::tools();
                    let current_idx = tools
                        .iter()
                        .position(|u| *u == state.ui.selected_utility)
                        .unwrap_or(0);
                    if current_idx > 0 {
                        state.ui.selected_utility = tools[current_idx - 1];
                    }
                }
                UtilitySection::Sounds => {
                    let sounds = UtilityItem::sounds();
                    let current_idx = sounds
                        .iter()
                        .position(|u| *u == state.ui.selected_sound)
                        .unwrap_or(0);
                    if current_idx > 0 {
                        state.ui.selected_sound = sounds[current_idx - 1];
                    }
                }
                UtilitySection::GlobalConfig => {
                    // Navigate config tree
                    if state.ui.config_tree_selected > 0 {
                        state.ui.config_tree_selected -= 1;
                    }
                }
                UtilitySection::Notepad => {}
            }
        }
        Action::ToggleUtilitySection => {
            state.ui.utility_section = state.ui.utility_section.toggle();
            // Initialize config tree when switching to GlobalConfig section
            if state.ui.utility_section == UtilitySection::GlobalConfig
                && state.ui.config_tree_nodes.is_empty()
            {
                crate::app::utilities::init_config_tree(state);
            }
        }
        Action::ToggleConfigItem => {
            // Open a terminal in the selected config directory
            if state.ui.config_tree_nodes.is_empty() {
                return Ok(());
            }

            let selected = state.ui.config_tree_selected;
            if selected >= state.ui.config_tree_nodes.len() {
                return Ok(());
            }

            let node = &state.ui.config_tree_nodes[selected];
            // Store the config directory path for creating terminal (handled in handler.rs)
            state.system.pending_config_terminal = Some(node.path().to_path_buf());
        }
        Action::ToggleBrownNoise => {
            state.system.brown_noise_playing = !state.system.brown_noise_playing;
        }
        Action::ToggleClassicalRadio => {
            state.system.classical_radio_playing = !state.system.classical_radio_playing;
        }
        Action::ToggleOceanWaves => {
            state.system.ocean_waves_playing = !state.system.ocean_waves_playing;
        }
        Action::ToggleWindChimes => {
            state.system.wind_chimes_playing = !state.system.wind_chimes_playing;
        }
        Action::ToggleRainforestRain => {
            state.system.rainforest_rain_playing = !state.system.rainforest_rain_playing;
        }
        _ => {}
    }
    Ok(())
}

/// Handle smooth auto-scrolling during text selection drag.
/// Called on each tick to provide continuous scrolling when cursor is near pane edges.
/// Uses acceleration: the closer to the edge, the faster the scroll.
pub fn handle_drag_auto_scroll(state: &mut AppState) {
    // Edge zone where scrolling activates (in rows from edge)
    const SCROLL_EDGE_ZONE: u16 = 5;
    // Base scroll speed (lines per tick)
    const BASE_SCROLL_SPEED: u16 = 2;
    // Max scroll speed when at the very edge
    const MAX_SCROLL_SPEED: u16 = 8;

    let Some((mouse_x, mouse_y)) = state.ui.drag_mouse_pos else {
        return;
    };

    // Calculate scroll speed based on distance from edge (acceleration)
    // Returns (should_scroll_up, should_scroll_down, speed)
    let calc_scroll = |y: u16, pane_top: u16, pane_bottom: u16| -> (bool, bool, u16) {
        let top_threshold = pane_top.saturating_add(SCROLL_EDGE_ZONE);
        let bottom_threshold = pane_bottom.saturating_sub(SCROLL_EDGE_ZONE);

        if y < top_threshold {
            // In top edge zone - scroll up
            // Speed increases as we get closer to the edge
            let distance_from_edge = y.saturating_sub(pane_top);
            let speed = if distance_from_edge == 0 {
                MAX_SCROLL_SPEED
            } else {
                let ratio = (SCROLL_EDGE_ZONE.saturating_sub(distance_from_edge)) as f32
                    / SCROLL_EDGE_ZONE as f32;
                (BASE_SCROLL_SPEED as f32 + (MAX_SCROLL_SPEED - BASE_SCROLL_SPEED) as f32 * ratio)
                    as u16
            };
            (true, false, speed.max(BASE_SCROLL_SPEED))
        } else if y >= bottom_threshold {
            // In bottom edge zone - scroll down
            let distance_from_edge = pane_bottom.saturating_sub(y);
            let speed = if distance_from_edge == 0 {
                MAX_SCROLL_SPEED
            } else {
                let ratio = (SCROLL_EDGE_ZONE.saturating_sub(distance_from_edge)) as f32
                    / SCROLL_EDGE_ZONE as f32;
                (BASE_SCROLL_SPEED as f32 + (MAX_SCROLL_SPEED - BASE_SCROLL_SPEED) as f32 * ratio)
                    as u16
            };
            (false, true, speed.max(BASE_SCROLL_SPEED))
        } else {
            (false, false, 0)
        }
    };

    // Handle main output pane auto-scroll
    if state.ui.text_selection.is_dragging {
        if let Some((ax, ay, aw, ah)) = state.ui.output_pane_area {
            let pane_top = ay;
            let pane_bottom = ay.saturating_add(ah);
            let (scroll_up, scroll_down, speed) = calc_scroll(mouse_y, pane_top, pane_bottom);

            if scroll_up {
                state.ui.output_scroll_offset = state.ui.output_scroll_offset.saturating_add(speed);
                if let Some((row, col)) = pane_text_position(
                    (ax, ay, aw, ah),
                    mouse_x,
                    mouse_y,
                    state.ui.output_content_length,
                    state.ui.output_scroll_offset,
                ) {
                    state.ui.text_selection.end = Some((row, col));
                }
            } else if scroll_down {
                state.ui.output_scroll_offset = state.ui.output_scroll_offset.saturating_sub(speed);
                if let Some((row, col)) = pane_text_position(
                    (ax, ay, aw, ah),
                    mouse_x,
                    mouse_y,
                    state.ui.output_content_length,
                    state.ui.output_scroll_offset,
                ) {
                    state.ui.text_selection.end = Some((row, col));
                }
            }
        }
    }

    // Handle pinned panes auto-scroll
    for idx in 0..state.ui.pinned_text_selections.len() {
        if state.ui.pinned_text_selections[idx].is_dragging {
            if let Some((ax, ay, aw, ah)) = state.ui.pinned_pane_areas[idx] {
                let pane_top = ay;
                let pane_bottom = ay.saturating_add(ah);
                let (scroll_up, scroll_down, speed) = calc_scroll(mouse_y, pane_top, pane_bottom);

                if scroll_up {
                    state.ui.pinned_scroll_offsets[idx] =
                        state.ui.pinned_scroll_offsets[idx].saturating_add(speed);
                    if let Some((row, col)) = pane_text_position(
                        (ax, ay, aw, ah),
                        mouse_x,
                        mouse_y,
                        state.ui.pinned_content_lengths[idx],
                        state.ui.pinned_scroll_offsets[idx],
                    ) {
                        state.ui.pinned_text_selections[idx].end = Some((row, col));
                    }
                } else if scroll_down {
                    state.ui.pinned_scroll_offsets[idx] =
                        state.ui.pinned_scroll_offsets[idx].saturating_sub(speed);
                    if let Some((row, col)) = pane_text_position(
                        (ax, ay, aw, ah),
                        mouse_x,
                        mouse_y,
                        state.ui.pinned_content_lengths[idx],
                        state.ui.pinned_scroll_offsets[idx],
                    ) {
                        state.ui.pinned_text_selections[idx].end = Some((row, col));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::handle_navigation_action;
    use crate::app::{Action, AppState, FocusPanel};
    use crate::models::{Workspace, WorkspaceStatus};
    use crate::pty::PtyManager;
    use std::path::PathBuf;
    use tokio::sync::mpsc;

    fn workspace(name: &str, status: WorkspaceStatus) -> Workspace {
        let mut workspace = Workspace::new(name.to_string(), PathBuf::from(format!("/tmp/{name}")));
        workspace.status = status;
        workspace
    }

    #[test]
    fn mouse_click_on_paused_workspace_selects_it() {
        let mut state = AppState::default();
        state.data.workspaces = vec![
            workspace("alpha", WorkspaceStatus::Working),
            workspace("beta", WorkspaceStatus::Paused),
        ];
        state.ui.workspace_area = Some((0, 0, 20, 7));
        state.ui.focus = FocusPanel::OutputPane;

        let pty_manager = PtyManager::new();
        let (pty_tx, _) = mpsc::channel(1);

        handle_navigation_action(&mut state, Action::MouseClick(2, 4), &pty_manager, &pty_tx)
            .unwrap();

        assert_eq!(state.ui.focus, FocusPanel::WorkspaceList);
        assert_eq!(state.ui.selected_workspace_idx, 1);
    }

    #[test]
    fn mouse_scroll_in_workspace_moves_workspace_selection() {
        let mut state = AppState::default();
        state.data.workspaces = vec![
            workspace("alpha", WorkspaceStatus::Working),
            workspace("beta", WorkspaceStatus::Paused),
        ];
        state.ui.workspace_area = Some((0, 0, 20, 7));

        let pty_manager = PtyManager::new();
        let (pty_tx, _) = mpsc::channel(1);

        handle_navigation_action(
            &mut state,
            Action::MouseScrollDown(2, 2),
            &pty_manager,
            &pty_tx,
        )
        .unwrap();

        assert_eq!(state.ui.focus, FocusPanel::WorkspaceList);
        assert_eq!(state.ui.selected_workspace_idx, 1);
    }
}
