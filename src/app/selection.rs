use crate::app::{AppState, TextSelection};
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionSurface {
    /// Coordinates came from the live vt100 screen currently being rendered.
    Live,
    /// Coordinates came from the replay parser used for deep scrollback.
    Replay,
    /// Coordinates came from reconstructed transcript scrollback.
    Transcript,
}

pub fn pane_text_position(
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

/// Extract selected text from vt100 screen based on selection start and end positions
pub fn extract_selected_text(
    screen: &vt100::Screen,
    start: (usize, usize),
    end: (usize, usize),
) -> String {
    let (rows, cols) = screen.size();
    let rows = rows as usize;
    let cols = cols as usize;

    if rows == 0 || cols == 0 {
        return String::new();
    }

    // Order the selection (start should be before end)
    let (start_row, start_col, end_row, end_col) =
        if start.0 < end.0 || (start.0 == end.0 && start.1 <= end.1) {
            (start.0, start.1, end.0, end.1)
        } else {
            (end.0, end.1, start.0, start.1)
        };

    // Bail if the selection no longer overlaps the current screen.
    // This protects against stale coords from a screen that has since shrunk.
    if start_row >= rows {
        return String::new();
    }

    let mut result = String::new();
    let last_row = end_row.min(rows - 1);

    for row in start_row..=last_row {
        let row_start = if row == start_row {
            start_col.min(cols)
        } else {
            0
        };
        let col_end = if row == end_row {
            end_col.min(cols)
        } else {
            cols
        };

        let mut line = String::new();
        for col in row_start..=col_end {
            if let Some(cell) = screen.cell(row as u16, col as u16) {
                let contents = cell.contents();
                if contents.is_empty() {
                    line.push(' ');
                } else {
                    line.push_str(&contents);
                }
            }
        }

        // Trim trailing whitespace from each line
        let trimmed = line.trim_end();
        result.push_str(trimmed);

        // Add newline between rows (but not at the very end)
        if row < last_row {
            result.push('\n');
        }
    }

    result
}

fn extract_for_surface(
    parser: &vt100::Parser,
    raw_buf: Option<&crate::app::RawOutputBuffer>,
    transcript: Option<&crate::app::TranscriptBuffer>,
    replay_rows: u16,
    start: (usize, usize),
    end: (usize, usize),
    surface: SelectionSurface,
) -> String {
    if surface == SelectionSurface::Transcript {
        if let Some(transcript) = transcript {
            return transcript.extract_text(start, end);
        }
    }

    if surface == SelectionSurface::Replay {
        if let Some(raw_buf) = raw_buf {
            if !raw_buf.bytes.is_empty() {
                let cols = parser.screen().size().1;
                let replay = crate::tui::replay::create_replay_parser(raw_buf, cols, replay_rows);
                return extract_selected_text(replay.screen(), start, end);
            }
        }
    }

    extract_selected_text(parser.screen(), start, end)
}

pub fn copy_active_selection(state: &mut AppState) -> bool {
    fn copy_non_empty(text: String) -> bool {
        if text.is_empty() {
            return false;
        }
        copy_to_clipboard(&text);
        true
    }

    fn extract_for_session(
        state: &AppState,
        session_id: uuid::Uuid,
        start: (usize, usize),
        end: (usize, usize),
        surface: SelectionSurface,
    ) -> Option<String> {
        let parser = state.system.output_buffers.get(&session_id)?;
        Some(extract_for_surface(
            parser,
            state.system.raw_output_buffers.get(&session_id),
            state.system.transcript_buffers.get(&session_id),
            state.system.user_config.replay_parser_rows,
            start,
            end,
            surface,
        ))
    }

    let output_selection = state.text_selection();
    if let (Some(start), Some(end)) = (output_selection.start, output_selection.end) {
        if start != end {
            if let Some(session_id) = state.active_session_id() {
                let surface = if state.output_on_replay() {
                    if state.system.transcript_buffers.contains_key(&session_id) {
                        SelectionSurface::Transcript
                    } else {
                        SelectionSurface::Replay
                    }
                } else {
                    SelectionSurface::Live
                };
                if let Some(text) = extract_for_session(state, session_id, start, end, surface) {
                    if copy_non_empty(text) {
                        return true;
                    }
                }
            }
        }
    }

    for idx in 0..state.pinned_count() {
        let sel = state.pinned_text_selection(idx);
        if let (Some(start), Some(end)) = (sel.start, sel.end) {
            if start != end {
                if let Some(session_id) = state.pinned_terminal_id_at(idx) {
                    let surface = if state.pinned_on_replay(idx) {
                        if state.system.transcript_buffers.contains_key(&session_id) {
                            SelectionSurface::Transcript
                        } else {
                            SelectionSurface::Replay
                        }
                    } else {
                        SelectionSurface::Live
                    };
                    if let Some(text) = extract_for_session(state, session_id, start, end, surface)
                    {
                        if copy_non_empty(text) {
                            return true;
                        }
                    }
                }
            }
        }
    }

    false
}

/// Clear all pinned pane text selections (selected workspace)
pub fn clear_all_pinned_selections(state: &mut AppState) {
    if let Some(ws_ui) = state.ws_ui_mut() {
        for pane in ws_ui.pinned_panes.iter_mut() {
            pane.text_selection = TextSelection::default();
        }
    }
}

/// Clear the active output-pane selection and any in-flight drag tracking.
/// Called when the active session changes so stale coords don't get used to
/// extract text from a different session's screen.
pub fn clear_active_text_selection(state: &mut AppState) {
    state.set_text_selection(TextSelection::default());
    state.set_drag_mouse_pos(None);
}

/// Transition between workspaces. Call AFTER `state.ui.selected_workspace_idx`
/// has been updated to the new workspace.
///
/// All per-workspace UI state lives directly in `ws_ui`, so there is nothing
/// to snapshot or restore anymore — this only seeds the new workspace's entry
/// (honoring `Workspace.last_active_session_id`) so read accessors see it
/// before the first write.
pub fn transition_workspace(state: &mut AppState, _prev_ws_id: Option<Uuid>) {
    state.ws_ui_mut();
}

/// Long-lived clipboard handle. arboard on macOS requires the handle to remain
/// alive for the data to actually publish to the system pasteboard, so we keep
/// a single process-wide instance instead of constructing one per copy.
fn clipboard() -> Option<&'static Mutex<arboard::Clipboard>> {
    static CLIPBOARD: OnceLock<Option<Mutex<arboard::Clipboard>>> = OnceLock::new();
    CLIPBOARD
        .get_or_init(|| arboard::Clipboard::new().ok().map(Mutex::new))
        .as_ref()
}

/// Copy text to clipboard (cross-platform via arboard)
pub fn copy_to_clipboard(text: &str) {
    if text.is_empty() {
        return;
    }
    let Some(cb) = clipboard() else {
        crate::logger::warn("failed to initialize clipboard");
        return;
    };
    let Ok(mut guard) = cb.lock() else {
        crate::logger::warn("clipboard lock poisoned");
        return;
    };
    if let Err(err) = guard.set_text(text.to_owned()) {
        crate::logger::warn(format!("failed to copy selection to clipboard: {err}"));
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_for_surface, pane_text_position, transition_workspace, SelectionSurface};
    use crate::app::{AppState, RawOutputBuffer};
    use crate::models::Workspace;
    use std::path::PathBuf;

    /// `output_scroll_offset` lives per-workspace in `ws_ui`. Switching
    /// workspaces must preserve each workspace's own scroll position — and
    /// crucially the snapshot on the way out must not reset it to the default.
    #[test]
    fn output_scroll_offset_is_preserved_per_workspace_across_switches() {
        let mut state = AppState::default();
        let ws_a = Workspace::new("a".into(), PathBuf::from("/tmp/a"));
        let ws_b = Workspace::new("b".into(), PathBuf::from("/tmp/b"));
        let (id_a, id_b) = (ws_a.id, ws_b.id);
        state.data.workspaces = vec![ws_a, ws_b];

        // Workspace A: scroll to 5.
        state.ui.selected_workspace_idx = 0;
        state.set_output_scroll_offset(5);
        assert_eq!(state.output_scroll_offset(), 5);

        // Switch to B — independent, starts at 0.
        state.ui.selected_workspace_idx = 1;
        transition_workspace(&mut state, Some(id_a));
        assert_eq!(state.output_scroll_offset(), 0);
        state.set_output_scroll_offset(9);

        // Back to A — its 5 survived the round trip.
        state.ui.selected_workspace_idx = 0;
        transition_workspace(&mut state, Some(id_b));
        assert_eq!(state.output_scroll_offset(), 5);

        // And B still remembers 9.
        state.ui.selected_workspace_idx = 1;
        transition_workspace(&mut state, Some(id_a));
        assert_eq!(state.output_scroll_offset(), 9);
    }

    /// `output_on_replay` is also per-workspace; switching must preserve each
    /// workspace's flag and not reset it on snapshot.
    #[test]
    fn output_on_replay_is_preserved_per_workspace_across_switches() {
        let mut state = AppState::default();
        let ws_a = Workspace::new("a".into(), PathBuf::from("/tmp/a"));
        let ws_b = Workspace::new("b".into(), PathBuf::from("/tmp/b"));
        let (id_a, id_b) = (ws_a.id, ws_b.id);
        state.data.workspaces = vec![ws_a, ws_b];

        state.ui.selected_workspace_idx = 0;
        state.set_output_on_replay(true);
        assert!(state.output_on_replay());

        state.ui.selected_workspace_idx = 1;
        transition_workspace(&mut state, Some(id_a));
        assert!(!state.output_on_replay()); // B defaults to live

        state.ui.selected_workspace_idx = 0;
        transition_workspace(&mut state, Some(id_b));
        assert!(state.output_on_replay()); // A's flag survived
    }

    #[test]
    fn pane_text_position_rejects_border_and_empty_content() {
        let area = (10, 5, 20, 8);

        assert_eq!(pane_text_position(area, 11, 6, 0, 0), None);
        assert_eq!(pane_text_position(area, 10, 6, 10, 0), None);
        assert_eq!(pane_text_position(area, 11, 5, 10, 0), None);
        assert_eq!(pane_text_position(area, 29, 6, 10, 0), None);
        assert_eq!(pane_text_position(area, 11, 12, 10, 0), None);
    }

    #[test]
    fn pane_text_position_maps_visible_cell_without_scroll() {
        let area = (10, 5, 20, 8);

        assert_eq!(pane_text_position(area, 11, 6, 4, 0), Some((0, 0)));
        assert_eq!(pane_text_position(area, 14, 8, 4, 0), Some((2, 3)));
    }

    #[test]
    fn pane_text_position_accounts_for_scroll_from_bottom() {
        let area = (10, 5, 20, 8);

        assert_eq!(pane_text_position(area, 11, 6, 20, 0), Some((14, 0)));
        assert_eq!(pane_text_position(area, 11, 6, 20, 3), Some((11, 0)));
        assert_eq!(pane_text_position(area, 11, 6, 20, 99), Some((0, 0)));
    }

    #[test]
    fn live_surface_selection_does_not_read_from_replay_history() {
        let mut live_parser = vt100::Parser::new(4, 40, 0);
        live_parser.process(b"current short words");

        let mut raw = RawOutputBuffer::new(1024);
        raw.append(b"history alpha\r\nhistory beta\r\ncurrent short words");

        let live_text = extract_for_surface(
            &live_parser,
            Some(&raw),
            None,
            4,
            (0, 0),
            (0, 6),
            SelectionSurface::Live,
        );
        let replay_text = extract_for_surface(
            &live_parser,
            Some(&raw),
            None,
            4,
            (0, 0),
            (0, 6),
            SelectionSurface::Replay,
        );

        assert_eq!(live_text, "current");
        assert_eq!(replay_text, "history");
    }
}
