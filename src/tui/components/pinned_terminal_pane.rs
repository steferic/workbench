use crate::app::{AppState, FocusPanel, InputMode, ReplayCache};
use crate::tui::replay::create_replay_parser;
use crate::tui::utils::{
    convert_vt100_to_lines_visible, get_content_length, get_cursor_info, get_selection_bounds,
    render_cursor,
};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

/// Render a specific pinned terminal pane at the given index
pub fn render_at(frame: &mut Frame, area: Rect, state: &mut AppState, pane_index: usize) {
    let is_focused =
        matches!(state.ui.focus, FocusPanel::PinnedTerminalPane(idx) if idx == pane_index);

    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::Magenta)
    };

    let title = state
        .pinned_terminal_session_at(pane_index)
        .map(|s| {
            format!(
                " {} [pinned {}] ",
                s.agent_type.display_name(),
                pane_index + 1
            )
        })
        .unwrap_or_else(|| format!(" Pinned {} ", pane_index + 1));

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner_area = block.inner(area);

    #[allow(unused_assignments)]
    let mut cursor_state: Option<crate::tui::utils::CursorInfo> = None;
    let viewport_height = inner_area.height as usize;

    // Get the pinned session ID for replay buffer lookups
    let pinned_session_id = state.pinned_terminal_id_at(pane_index);

    // Convert vt100 parser output to ratatui Lines
    if let Some(parser) = state.pinned_terminal_output_at(pane_index) {
        let screen = parser.screen();
        let info = get_cursor_info(screen);
        cursor_state = Some(info);
        let is_alternate = screen.alternate_screen();
        let screen_size = screen.size();

        let live_content_len = if is_alternate {
            viewport_height
        } else {
            get_content_length(screen, info.row)
        };

        let scroll_from_bottom_raw = state.ui.pinned_scroll_offsets[pane_index] as usize;
        let Some(session_id) = pinned_session_id else {
            frame.render_widget(Paragraph::new("").block(block), area);
            return;
        };

        let live_max_scroll = live_content_len.saturating_sub(viewport_height);
        let needs_replay = !is_alternate
            && scroll_from_bottom_raw > live_max_scroll
            && state
                .system
                .raw_output_buffers
                .get(&session_id)
                .map(|b| !b.bytes.is_empty())
                .unwrap_or(false);

        let (lines, stable_len, scroll_from_bottom, scroll_offset, sel_translated) = if needs_replay
        {
            let Some(raw_buf) = state.system.raw_output_buffers.get(&session_id) else {
                frame.render_widget(Paragraph::new("").block(block), area);
                return;
            };
            let generation = raw_buf.generation;
            let cols = screen_size.1;

            // Check if cached parser is still valid (same generation + cols)
            let cache_valid = state
                .system
                .replay_caches
                .get(&session_id)
                .map(|c| c.generation == generation && c.cols == cols)
                .unwrap_or(false);

            if !cache_valid {
                let replay_parser = create_replay_parser(
                    raw_buf,
                    cols,
                    state.system.user_config.replay_parser_rows,
                );
                let replay_screen = replay_parser.screen();
                let replay_cursor = get_cursor_info(replay_screen);
                let replay_content_len = get_content_length(replay_screen, replay_cursor.row);

                state.system.replay_caches.insert(
                    session_id,
                    ReplayCache {
                        generation,
                        cols,
                        parser: replay_parser,
                        content_length: replay_content_len,
                    },
                );
            }

            // Render visible lines from the cached parser
            let Some(cache) = state.system.replay_caches.get(&session_id) else {
                frame.render_widget(Paragraph::new("").block(block), area);
                return;
            };
            let replay_content_len = cache.content_length;
            let replay_screen = cache.parser.screen();
            let replay_cursor = get_cursor_info(replay_screen);

            // On live→replay transition, translate selection coordinates in a local copy
            let mut sel_copy = state
                .ui
                .pinned_text_selections
                .get(pane_index)
                .copied()
                .unwrap_or_default();
            if !state.ui.pinned_on_replay[pane_index] {
                let prev_content_len = state.ui.pinned_content_lengths[pane_index];
                if replay_content_len > prev_content_len && prev_content_len > 0 {
                    let offset = replay_content_len - prev_content_len;
                    if let Some((row, col)) = sel_copy.start {
                        sel_copy.start = Some((row + offset, col));
                    }
                    if let Some((row, col)) = sel_copy.end {
                        sel_copy.end = Some((row + offset, col));
                    }
                }
            }

            let selection =
                get_selection_bounds(&sel_copy, replay_content_len, replay_screen.size().1);
            let pane_height = Some(viewport_height as u16);

            let max_scroll = replay_content_len.saturating_sub(viewport_height);
            let sfb_clamped = scroll_from_bottom_raw.min(max_scroll);
            let so = max_scroll.saturating_sub(sfb_clamped);

            let buffer_lines = 5;
            let visible_start = so.saturating_sub(buffer_lines);
            let visible_count = viewport_height + buffer_lines * 2;

            let mut replay_lines = convert_vt100_to_lines_visible(
                replay_screen,
                selection,
                replay_cursor.row,
                pane_height,
                Some(visible_start),
                Some(visible_count),
            );

            while replay_lines.len() < replay_content_len {
                replay_lines.push(Line::raw(""));
            }

            (replay_lines, replay_content_len, sfb_clamped, so, sel_copy)
        } else {
            // Live parser path
            let pane_height = Some(viewport_height as u16);

            let prev_len = state.ui.pinned_content_lengths[pane_index];
            let stable_len = if live_content_len >= prev_len || prev_len - live_content_len >= 20 {
                live_content_len
            } else {
                prev_len
            };

            // On replay→live transition, translate selection coordinates back (local copy)
            let mut sel_copy = state
                .ui
                .pinned_text_selections
                .get(pane_index)
                .copied()
                .unwrap_or_default();
            if state.ui.pinned_on_replay[pane_index] {
                let prev_content_len = state.ui.pinned_content_lengths[pane_index];
                if prev_content_len > live_content_len && live_content_len > 0 {
                    let offset = prev_content_len - live_content_len;
                    if let Some((row, col)) = sel_copy.start {
                        sel_copy.start = Some((row.saturating_sub(offset), col));
                    }
                    if let Some((row, col)) = sel_copy.end {
                        sel_copy.end = Some((row.saturating_sub(offset), col));
                    }
                }
            }

            let selection = get_selection_bounds(&sel_copy, stable_len, screen_size.1);

            let max_scroll = stable_len.saturating_sub(viewport_height);
            let sfb = scroll_from_bottom_raw.min(max_scroll);
            let so = max_scroll.saturating_sub(sfb);

            let buffer_lines = 5;
            let visible_start = so.saturating_sub(buffer_lines);
            let visible_count = viewport_height + buffer_lines * 2;

            let mut lines = convert_vt100_to_lines_visible(
                screen,
                selection,
                info.row,
                pane_height,
                Some(visible_start),
                Some(visible_count),
            );

            while lines.len() < stable_len {
                lines.push(Line::raw(""));
            }

            (lines, stable_len, sfb, so, sel_copy)
        };

        // Update replay flag and persist translated selection after parser borrow is released
        state.ui.pinned_on_replay[pane_index] = needs_replay;
        if sel_translated.start.is_some() {
            state.ui.pinned_text_selections[pane_index] = sel_translated;
        }
        state.ui.pinned_content_lengths[pane_index] = stable_len;

        let paragraph = Paragraph::new(lines)
            .block(block)
            .scroll((scroll_offset as u16, 0));

        frame.render_widget(paragraph, area);

        // Render scrollbar if content exceeds viewport
        if stable_len > viewport_height {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
            let mut scrollbar_state =
                ScrollbarState::new(stable_len.saturating_sub(viewport_height))
                    .position(scroll_offset);
            frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
        }

        if is_focused && state.ui.input_mode == InputMode::Normal && scroll_from_bottom == 0 {
            if let Some(info) = cursor_state {
                let needs_terminal_cursor = state
                    .pinned_terminal_session_at(pane_index)
                    .map(|s| {
                        s.agent_type.is_terminal()
                            || matches!(s.agent_type, crate::models::AgentType::Codex)
                    })
                    .unwrap_or(false);

                if needs_terminal_cursor {
                    render_cursor(frame, inner_area, info, scroll_offset, true);
                }
            }
        }
    } else {
        state.ui.pinned_content_lengths[pane_index] = 0;
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No terminal in this slot",
                Style::default().fg(Color::Gray),
            )),
        ];

        let paragraph = Paragraph::new(lines).block(block);

        frame.render_widget(paragraph, area);
    }
}
