use crate::app::{Action, AppState, ConfigItem, Divider, FocusPanel, InputMode, TextSelection, UtilityItem, UtilitySection};
use crate::app::pty_ops::resize_ptys_to_panes;
use crate::app::selection::{clear_all_pinned_selections, copy_to_clipboard, extract_selected_text};
use crate::app::session_start::start_workspace_sessions;
use crate::persistence;
use crate::pty::PtyManager;
use anyhow::Result;
use tokio::sync::mpsc;
use uuid::Uuid;

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

fn pane_text_position(
    area: (u16, u16, u16, u16),
    x: u16,
    y: u16,
    content_length: usize,
    scroll_from_bottom: u16,
) -> Option<(usize, usize)> {
    let (area_x, area_y, area_w, area_h) = area;
    let inner_w = area_w.saturating_sub(2);
    let inner_h = area_h.saturating_sub(2);

    if inner_w == 0 || inner_h == 0 || content_length == 0 {
        return None;
    }

    let right_edge = area_x.saturating_add(area_w).saturating_sub(1);
    let bottom_edge = area_y.saturating_add(area_h).saturating_sub(1);
    if x <= area_x || x >= right_edge || y <= area_y || y >= bottom_edge {
        return None;
    }

    let col_in_view = x.saturating_sub(area_x).saturating_sub(1) as usize;
    let row_in_view = y.saturating_sub(area_y).saturating_sub(1) as usize;

    if col_in_view >= inner_w as usize || row_in_view >= inner_h as usize {
        return None;
    }

    let viewport_height = inner_h as usize;
    let max_scroll = content_length.saturating_sub(viewport_height);
    let scroll_from_bottom = (scroll_from_bottom as usize).min(max_scroll);
    let scroll_offset = max_scroll.saturating_sub(scroll_from_bottom);

    let row = scroll_offset
        .saturating_add(row_in_view)
        .min(content_length.saturating_sub(1));

    Some((row, col_in_view))
}

fn is_in_area(x: u16, y: u16, area: (u16, u16, u16, u16)) -> bool {
    let (ax, ay, aw, ah) = area;
    x >= ax && x < ax + aw && y >= ay && y < ay + ah
}

fn copy_active_selection(state: &mut AppState) -> bool {
    if let (Some(parser), Some(start), Some(end)) = (
        state.active_output(),
        state.ui.text_selection.start,
        state.ui.text_selection.end,
    ) {
        if start != end {
            let text = extract_selected_text(parser.screen(), start, end);
            copy_to_clipboard(&text);
            return true;
        }
    }

    for idx in 0..state.pinned_count() {
        let sel = &state.ui.pinned_text_selections[idx];
        if let (Some(start), Some(end)) = (sel.start, sel.end) {
            if start != end {
                if let Some(parser) = state.pinned_terminal_output_at(idx) {
                    let text = extract_selected_text(parser.screen(), start, end);
                    copy_to_clipboard(&text);
                    return true;
                }
            }
        }
    }

    false
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
                let prev_idx = state.ui.selected_workspace_idx;
                state.select_prev_workspace();
                if state.ui.selected_workspace_idx != prev_idx {
                    start_workspace_sessions(state, pty_manager, pty_tx);
                }
            }
            FocusPanel::SessionList => {
                state.select_prev_session();
            }
            _ => {}
        },
        Action::MoveDown => match state.ui.focus {
            FocusPanel::WorkspaceList => {
                let prev_idx = state.ui.selected_workspace_idx;
                state.select_next_workspace();
                if state.ui.selected_workspace_idx != prev_idx {
                    start_workspace_sessions(state, pty_manager, pty_tx);
                }
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
            if prev_focus == FocusPanel::WorkspaceList && state.ui.focus == FocusPanel::SessionList {
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
        Action::CycleNextWorkspace => {
            use crate::models::WorkspaceStatus;

            // Only cycle through "Working" workspaces, in visual order
            let working_indices: Vec<usize> = state.data.workspaces.iter()
                .enumerate()
                .filter(|(_, ws)| ws.status == WorkspaceStatus::Working)
                .map(|(idx, _)| idx)
                .collect();

            if !working_indices.is_empty() {
                // Find current position in the working list
                let current_pos = working_indices.iter()
                    .position(|&idx| idx == state.ui.selected_workspace_idx);

                // Move to next working workspace (or first if not currently on a working one)
                let next_idx = match current_pos {
                    Some(pos) => working_indices[(pos + 1) % working_indices.len()],
                    None => working_indices[0],
                };

                state.ui.selected_workspace_idx = next_idx;
                state.ui.selected_session_idx = 0;

                // Also activate the first session in the new workspace
                if let Some(ws) = state.data.workspaces.get(next_idx) {
                    if let Some(sessions) = state.data.sessions.get(&ws.id) {
                        if let Some(session) = sessions.first() {
                            state.ui.active_session_id = Some(session.id);
                            state.ui.output_scroll_offset = 0;
                        }
                    }
                }
            }
        }
        Action::CycleNextSession => {
            // Cycle through agents only (Agents -> Parallel), skip terminals
            // Get parallel task session IDs first
            let parallel_session_ids: Vec<Uuid> = state.selected_workspace()
                .map(|ws| {
                    ws.parallel_tasks.iter()
                        .flat_map(|t| t.attempts.iter().map(|a| a.session_id))
                        .collect()
                })
                .unwrap_or_default();

            // Build visual order indices (agents only)
            let session_info: Option<(usize, Uuid)> = {
                let sessions = state.sessions_for_selected_workspace();

                // Agents: non-terminal, non-parallel
                let agent_indices: Vec<usize> = sessions.iter()
                    .enumerate()
                    .filter(|(_, s)| !s.agent_type.is_terminal() && !parallel_session_ids.contains(&s.id))
                    .map(|(i, _)| i)
                    .collect();

                // Parallel sessions (these are also agents)
                let parallel_indices: Vec<usize> = sessions.iter()
                    .enumerate()
                    .filter(|(_, s)| parallel_session_ids.contains(&s.id))
                    .map(|(i, _)| i)
                    .collect();

                // Combined visual order (agents only, no terminals)
                let visual_order: Vec<usize> = agent_indices.into_iter()
                    .chain(parallel_indices)
                    .collect();

                if !visual_order.is_empty() {
                    // Find current position in visual order
                    let current_pos = visual_order.iter()
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
                state.ui.selected_session_idx = next_idx;
                state.ui.active_session_id = Some(session_id);
                state.ui.output_scroll_offset = 0;
                state.ui.focus = FocusPanel::OutputPane;
            }
        }
        Action::MouseClick(x, y) => {
            // Simplified MouseClick logic using stored areas
            let (w, h) = state.system.terminal_size;
            let main_height = h.saturating_sub(1);
            let divider_tolerance = 2u16;

            let left_width = (w as f32 * state.ui.left_panel_ratio) as u16;
            if x >= left_width.saturating_sub(divider_tolerance) && x <= left_width + divider_tolerance && y < main_height {
                state.ui.dragging_divider = Some(Divider::LeftRight);
                state.ui.drag_start_pos = Some((x, y));
                state.ui.drag_start_ratio = state.ui.left_panel_ratio;
                return Ok(());
            }

            let workspace_height = (main_height as f32 * state.ui.workspace_ratio) as u16;
            if x < left_width && y >= workspace_height.saturating_sub(divider_tolerance) && y <= workspace_height + divider_tolerance {
                state.ui.dragging_divider = Some(Divider::WorkspaceSession);
                state.ui.drag_start_pos = Some((x, y));
                state.ui.drag_start_ratio = state.ui.workspace_ratio;
                return Ok(());
            }

            let lower_left_height = main_height.saturating_sub(workspace_height);
            let sessions_height = (lower_left_height as f32 * state.ui.sessions_ratio) as u16;
            let sessions_todos_divider_y = workspace_height + sessions_height;

            if x < left_width && y >= sessions_todos_divider_y.saturating_sub(divider_tolerance) && y <= sessions_todos_divider_y + divider_tolerance {
                state.ui.dragging_divider = Some(Divider::SessionsTodos);
                state.ui.drag_start_pos = Some((x, y));
                state.ui.drag_start_ratio = state.ui.sessions_ratio;
                return Ok(());
            }

            let remaining_height = lower_left_height.saturating_sub(sessions_height);
            let todos_height = (remaining_height as f32 * state.ui.todos_ratio) as u16;
            let todos_utilities_divider_y = sessions_todos_divider_y + todos_height;

            if x < left_width && y >= todos_utilities_divider_y.saturating_sub(divider_tolerance) && y <= todos_utilities_divider_y + divider_tolerance {
                state.ui.dragging_divider = Some(Divider::TodosUtilities);
                state.ui.drag_start_pos = Some((x, y));
                state.ui.drag_start_ratio = state.ui.todos_ratio;
                return Ok(());
            }

            if state.should_show_split() {
                if let Some((ox, _, ow, _)) = state.ui.output_pane_area {
                    let divider_x = ox + ow;
                    if x >= divider_x.saturating_sub(divider_tolerance) && x <= divider_x + divider_tolerance && y < main_height {
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
                            if y >= divider_y.saturating_sub(divider_tolerance) && y <= divider_y + divider_tolerance {
                                if let Some((px, _, pw, _)) = state.ui.pinned_pane_areas[0] {
                                    if x >= px && x < px + pw {
                                        state.ui.dragging_divider = Some(Divider::PinnedPanes(pane_idx));
                                        state.ui.drag_start_pos = Some((x, y));
                                        state.ui.drag_start_ratio = state.ui.pinned_pane_ratios[pane_idx];
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
                        let workspace_height = (main_height as f32 * state.ui.workspace_ratio) as u16;
                        let lower_left_height = main_height.saturating_sub(workspace_height);
                        let y_in_lower_left = y.saturating_sub(workspace_height);
                        let new_ratio = (y_in_lower_left as f32 / lower_left_height as f32).clamp(0.15, 0.70);
                        state.ui.sessions_ratio = new_ratio;
                    }
                    Divider::TodosUtilities => {
                        let workspace_height = (main_height as f32 * state.ui.workspace_ratio) as u16;
                        let lower_left_height = main_height.saturating_sub(workspace_height);
                        let sessions_height = (lower_left_height as f32 * state.ui.sessions_ratio) as u16;
                        let remaining_height = lower_left_height.saturating_sub(sessions_height);
                        let y_in_remaining = y.saturating_sub(workspace_height).saturating_sub(sessions_height);
                        let new_ratio = (y_in_remaining as f32 / remaining_height as f32).clamp(0.20, 0.80);
                        state.ui.todos_ratio = new_ratio;
                    }
                    Divider::OutputPinned => {
                        let left_width = (w as f32 * state.ui.left_panel_ratio) as u16;
                        let right_panel_width = w.saturating_sub(left_width);
                        let x_in_right = x.saturating_sub(left_width);
                        let new_ratio = (x_in_right as f32 / right_panel_width as f32).clamp(0.20, 0.80);
                        state.ui.output_split_ratio = new_ratio;
                    }
                    Divider::PinnedPanes(pane_idx) => {
                        let count = state.pinned_count();
                        if count > 1 && pane_idx < count - 1 {
                            let mut ratios = state.ui.pinned_pane_ratios;
                            let sum: f32 = ratios.iter().take(count).sum();

                            if let Some((_, py, _, _)) = state.ui.pinned_pane_areas[0] {
                                let pinned_total_height = state.ui.pinned_pane_areas.iter()
                                    .take(count)
                                    .filter_map(|a| a.map(|(_, _, _, h)| h))
                                    .sum::<u16>();

                                let y_in_pinned = y.saturating_sub(py) as f32;
                                let new_split = y_in_pinned / pinned_total_height as f32;

                                let combined_ratio = ratios[pane_idx] + ratios[pane_idx + 1];
                                let ratio_above: f32 = ratios.iter().take(pane_idx).sum();

                                let new_upper_ratio = ((new_split - ratio_above / sum) * sum).clamp(0.1, combined_ratio - 0.1);
                                ratios[pane_idx] = new_upper_ratio;
                                ratios[pane_idx + 1] = combined_ratio - new_upper_ratio;

                                state.ui.pinned_pane_ratios = ratios;
                            }
                        }
                    }
                }
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
                }
            }
        }
        Action::MouseUp(x, y) => {
            if state.ui.dragging_divider.is_some() {
                state.ui.dragging_divider = None;
                state.ui.drag_start_pos = None;
                resize_ptys_to_panes(state);
                let config = persistence::GlobalConfig {
                    banner_visible: state.ui.banner_visible,
                    left_panel_ratio: state.ui.left_panel_ratio,
                    workspace_ratio: state.ui.workspace_ratio,
                    sessions_ratio: state.ui.sessions_ratio,
                    todos_ratio: state.ui.todos_ratio,
                    output_split_ratio: state.ui.output_split_ratio,
                };
                let _ = persistence::save_config(&config);
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
                // Save notepad contents after paste
                let notepad_contents = state.notepad_content_for_persistence();
                let _ = persistence::save_with_notepad(&state.data.workspaces, &state.data.sessions, &notepad_contents);
            } else if let Some(session_id) = paste_target_session_id(state) {
                let data = bracketed_paste_payload(&text);
                if let Some(handle) = state.system.pty_handles.get_mut(&session_id) {
                    let _ = handle.send_input(&data);
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
                    let current_idx = tools.iter().position(|u| *u == state.ui.selected_utility).unwrap_or(0);
                    if current_idx < tools.len() - 1 {
                        state.ui.selected_utility = tools[current_idx + 1];
                    }
                }
                UtilitySection::Sounds => {
                    let sounds = UtilityItem::sounds();
                    let current_idx = sounds.iter().position(|u| *u == state.ui.selected_sound).unwrap_or(0);
                    if current_idx < sounds.len() - 1 {
                        state.ui.selected_sound = sounds[current_idx + 1];
                    }
                }
                UtilitySection::GlobalConfig => {
                    let configs = ConfigItem::all();
                    let current_idx = configs.iter().position(|c| *c == state.ui.selected_config).unwrap_or(0);
                    if current_idx < configs.len() - 1 {
                        state.ui.selected_config = configs[current_idx + 1];
                    }
                }
                UtilitySection::Notepad => {}
            }
        }
        Action::SelectPrevUtility => {
            match state.ui.utility_section {
                UtilitySection::Utilities => {
                    let tools = UtilityItem::tools();
                    let current_idx = tools.iter().position(|u| *u == state.ui.selected_utility).unwrap_or(0);
                    if current_idx > 0 {
                        state.ui.selected_utility = tools[current_idx - 1];
                    }
                }
                UtilitySection::Sounds => {
                    let sounds = UtilityItem::sounds();
                    let current_idx = sounds.iter().position(|u| *u == state.ui.selected_sound).unwrap_or(0);
                    if current_idx > 0 {
                        state.ui.selected_sound = sounds[current_idx - 1];
                    }
                }
                UtilitySection::GlobalConfig => {
                    let configs = ConfigItem::all();
                    let current_idx = configs.iter().position(|c| *c == state.ui.selected_config).unwrap_or(0);
                    if current_idx > 0 {
                        state.ui.selected_config = configs[current_idx - 1];
                    }
                }
                UtilitySection::Notepad => {}
            }
        }
        Action::ToggleUtilitySection => {
            state.ui.utility_section = state.ui.utility_section.toggle();
        }
        Action::ToggleConfigItem => {
            match state.ui.selected_config {
                ConfigItem::ToggleBanner => {
                    state.ui.banner_visible = !state.ui.banner_visible;
                }
            }
            let config = persistence::GlobalConfig {
                banner_visible: state.ui.banner_visible,
                left_panel_ratio: state.ui.left_panel_ratio,
                workspace_ratio: state.ui.workspace_ratio,
                sessions_ratio: state.ui.sessions_ratio,
                todos_ratio: state.ui.todos_ratio,
                output_split_ratio: state.ui.output_split_ratio,
            };
            let _ = persistence::save_config(&config);
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
