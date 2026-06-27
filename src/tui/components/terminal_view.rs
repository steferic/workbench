use crate::app::{ReplayCache, SystemState, TextSelection, TranscriptBuffer};
use crate::tui::replay::create_replay_parser;
use crate::tui::utils::{
    convert_vt100_to_lines_visible, get_content_length, get_cursor_info, get_selection_bounds,
    CursorInfo,
};
use ratatui::{
    style::Modifier,
    text::{Line, Span},
};
use uuid::Uuid;

const VISIBLE_BUFFER_LINES: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReplayPolicy {
    NormalOnly,
    NormalAndAlternate,
}

#[derive(Clone, Copy)]
pub(super) struct TerminalViewRequest {
    pub session_id: Uuid,
    pub viewport_height: usize,
    pub scroll_from_bottom: usize,
    pub prev_content_len: usize,
    pub was_on_replay: bool,
    pub selection: TextSelection,
    pub replay_policy: ReplayPolicy,
}

pub(super) struct TerminalView {
    pub lines: Vec<Line<'static>>,
    pub content_len: usize,
    pub scrollbar_content_len: usize,
    pub scroll_from_bottom: usize,
    pub scroll_offset: usize,
    pub on_replay: bool,
    pub selection: TextSelection,
    pub cursor: CursorInfo,
}

pub(super) fn build_terminal_view(
    system: &mut SystemState,
    request: TerminalViewRequest,
) -> Option<TerminalView> {
    let parser = system.output_buffers.get(&request.session_id)?;
    let screen = parser.screen();
    let cursor = get_cursor_info(screen);
    let is_alternate = screen.alternate_screen();
    let screen_cols = screen.size().1;

    let live_content_len = if is_alternate {
        request.viewport_height
    } else {
        get_content_length(screen, cursor.row)
    };
    let live_max_scroll = live_content_len.saturating_sub(request.viewport_height);
    let transcript_len = system
        .transcript_buffers
        .get(&request.session_id)
        .map(|buffer| buffer.len())
        .unwrap_or(0);
    let use_transcript = transcript_len > request.viewport_height
        && request.scroll_from_bottom > live_max_scroll
        && system
            .transcript_buffers
            .get(&request.session_id)
            .map(|buffer| !buffer.is_empty())
            .unwrap_or(false);
    let has_raw_data = system
        .raw_output_buffers
        .get(&request.session_id)
        .map(|b| !b.bytes.is_empty())
        .unwrap_or(false);

    let use_replay = !use_transcript
        && should_replay(
            is_alternate,
            request.scroll_from_bottom,
            live_max_scroll,
            has_raw_data,
            request.replay_policy,
        );

    let view = if use_transcript {
        let transcript = system.transcript_buffers.get(&request.session_id)?;
        let content_len = transcript.len();
        let selection = translate_selection(
            request.selection,
            request.was_on_replay,
            true,
            request.prev_content_len,
            content_len,
        );
        let selection_bounds = get_selection_bounds(&selection, content_len, screen_cols);
        let (scroll_from_bottom, scroll_offset) = scroll_positions(
            content_len,
            request.viewport_height,
            request.scroll_from_bottom,
        );

        TerminalView {
            lines: transcript_lines(
                transcript,
                selection_bounds,
                request.viewport_height,
                scroll_offset,
            ),
            content_len,
            scrollbar_content_len: content_len,
            scroll_from_bottom,
            scroll_offset,
            on_replay: true,
            selection,
            cursor,
        }
    } else if use_replay {
        let raw_buf = system.raw_output_buffers.get(&request.session_id)?;
        let generation = raw_buf.generation;

        let cache_valid = system
            .replay_caches
            .get(&request.session_id)
            .map(|c| c.generation == generation && c.cols == screen_cols)
            .unwrap_or(false);

        if !cache_valid {
            let replay_parser =
                create_replay_parser(raw_buf, screen_cols, system.user_config.replay_parser_rows);
            let replay_screen = replay_parser.screen();
            let replay_cursor = get_cursor_info(replay_screen);
            let replay_content_len = get_content_length(replay_screen, replay_cursor.row);

            system.replay_caches.insert(
                request.session_id,
                ReplayCache {
                    generation,
                    cols: screen_cols,
                    parser: replay_parser,
                    content_length: replay_content_len,
                },
            );
        }

        let cache = system.replay_caches.get(&request.session_id)?;
        let replay_screen = cache.parser.screen();
        let replay_cursor = get_cursor_info(replay_screen);
        let content_len = cache.content_length;
        let selection = translate_selection(
            request.selection,
            request.was_on_replay,
            true,
            request.prev_content_len,
            content_len,
        );
        let selection_bounds =
            get_selection_bounds(&selection, content_len, replay_screen.size().1);
        let (scroll_from_bottom, scroll_offset) = scroll_positions(
            content_len,
            request.viewport_height,
            request.scroll_from_bottom,
        );

        TerminalView {
            lines: visible_lines(
                replay_screen,
                selection_bounds,
                replay_cursor.row,
                request.viewport_height,
                scroll_offset,
            ),
            content_len,
            scrollbar_content_len: content_len,
            scroll_from_bottom,
            scroll_offset,
            on_replay: true,
            selection,
            cursor,
        }
    } else {
        let content_len = stable_live_len(live_content_len, request.prev_content_len);
        let selection = translate_selection(
            request.selection,
            request.was_on_replay,
            false,
            request.prev_content_len,
            live_content_len,
        );
        let selection_bounds = get_selection_bounds(&selection, content_len, screen_cols);
        let (scroll_from_bottom, scroll_offset) = scroll_positions(
            content_len,
            request.viewport_height,
            request.scroll_from_bottom,
        );
        let scrollbar_content_len = transcript_len.max(
            system
                .replay_caches
                .get(&request.session_id)
                .map(|c| c.content_length.max(content_len))
                .unwrap_or(content_len),
        );

        TerminalView {
            lines: visible_lines(
                screen,
                selection_bounds,
                cursor.row,
                request.viewport_height,
                scroll_offset,
            ),
            content_len,
            scrollbar_content_len,
            scroll_from_bottom,
            scroll_offset,
            on_replay: false,
            selection,
            cursor,
        }
    };

    Some(view)
}

fn should_replay(
    is_alternate: bool,
    scroll_from_bottom: usize,
    live_max_scroll: usize,
    has_raw_data: bool,
    policy: ReplayPolicy,
) -> bool {
    if !has_raw_data {
        return false;
    }

    if is_alternate {
        return policy == ReplayPolicy::NormalAndAlternate && scroll_from_bottom > 0;
    }

    scroll_from_bottom > live_max_scroll
}

fn stable_live_len(live_content_len: usize, prev_content_len: usize) -> usize {
    if live_content_len >= prev_content_len || prev_content_len - live_content_len >= 20 {
        live_content_len
    } else {
        prev_content_len
    }
}

fn translate_selection(
    mut selection: TextSelection,
    was_on_replay: bool,
    on_replay: bool,
    prev_content_len: usize,
    current_content_len: usize,
) -> TextSelection {
    if !was_on_replay && on_replay {
        if current_content_len > prev_content_len && prev_content_len > 0 {
            shift_selection(&mut selection, current_content_len - prev_content_len);
        }
    } else if was_on_replay && !on_replay {
        if prev_content_len > current_content_len && current_content_len > 0 {
            unshift_selection(&mut selection, prev_content_len - current_content_len);
        }
    }

    selection
}

fn shift_selection(selection: &mut TextSelection, offset: usize) {
    if let Some((row, col)) = selection.start {
        selection.start = Some((row + offset, col));
    }
    if let Some((row, col)) = selection.end {
        selection.end = Some((row + offset, col));
    }
}

fn unshift_selection(selection: &mut TextSelection, offset: usize) {
    if let Some((row, col)) = selection.start {
        selection.start = Some((row.saturating_sub(offset), col));
    }
    if let Some((row, col)) = selection.end {
        selection.end = Some((row.saturating_sub(offset), col));
    }
}

fn scroll_positions(
    content_len: usize,
    viewport_height: usize,
    scroll_from_bottom: usize,
) -> (usize, usize) {
    let max_scroll = content_len.saturating_sub(viewport_height);
    let scroll_from_bottom = scroll_from_bottom.min(max_scroll);
    let scroll_offset = max_scroll.saturating_sub(scroll_from_bottom);
    (scroll_from_bottom, scroll_offset)
}

fn visible_lines(
    screen: &vt100::Screen,
    selection: Option<crate::tui::utils::SelectionBounds>,
    cursor_row: u16,
    viewport_height: usize,
    scroll_offset: usize,
) -> Vec<Line<'static>> {
    let visible_start = scroll_offset.saturating_sub(VISIBLE_BUFFER_LINES);
    let visible_count = viewport_height + VISIBLE_BUFFER_LINES * 2;

    let mut lines = convert_vt100_to_lines_visible(
        screen,
        selection,
        cursor_row,
        Some(viewport_height as u16),
        Some(visible_start),
        Some(visible_count),
    );

    let min_visible_len = scroll_offset.saturating_add(viewport_height);
    while lines.len() < min_visible_len {
        lines.push(Line::raw(""));
    }

    lines
}

fn transcript_lines(
    transcript: &TranscriptBuffer,
    selection: Option<crate::tui::utils::SelectionBounds>,
    viewport_height: usize,
    scroll_offset: usize,
) -> Vec<Line<'static>> {
    let visible_start = scroll_offset.saturating_sub(VISIBLE_BUFFER_LINES);
    let visible_count = viewport_height + VISIBLE_BUFFER_LINES * 2;
    let visible_end = (visible_start + visible_count).min(transcript.len());

    let mut lines = Vec::new();
    for _ in 0..visible_start {
        lines.push(Line::raw(""));
    }
    for row in visible_start..visible_end {
        if let Some(line) = transcript.styled_line(row) {
            lines.push(transcript_line(row, line, selection));
        } else {
            lines.push(Line::raw(""));
        }
    }

    let min_visible_len = scroll_offset.saturating_add(viewport_height);
    while lines.len() < min_visible_len {
        lines.push(Line::raw(""));
    }

    lines
}

fn transcript_line(
    row: usize,
    line: &crate::app::TranscriptLine,
    selection: Option<crate::tui::utils::SelectionBounds>,
) -> Line<'static> {
    let Some(bounds) = selection else {
        return Line::from(
            line.spans()
                .iter()
                .map(|span| Span::styled(span.text.clone(), span.style))
                .collect::<Vec<_>>(),
        );
    };
    if row < bounds.start_row || row > bounds.end_row {
        return Line::from(
            line.spans()
                .iter()
                .map(|span| Span::styled(span.text.clone(), span.style))
                .collect::<Vec<_>>(),
        );
    }

    let char_count = line.text().chars().count();
    if char_count == 0 {
        return Line::raw("");
    }

    let start = if row == bounds.start_row {
        bounds.start_col.min(char_count)
    } else {
        0
    };
    let end = if row == bounds.end_row {
        bounds.end_col.min(char_count.saturating_sub(1))
    } else {
        char_count.saturating_sub(1)
    };

    if start >= char_count || start > end {
        return Line::from(
            line.spans()
                .iter()
                .map(|span| Span::styled(span.text.clone(), span.style))
                .collect::<Vec<_>>(),
        );
    }

    let mut spans = Vec::new();
    let mut col = 0usize;
    for span in line.spans() {
        let span_len = span.text.chars().count();
        let span_start = col;
        let span_end = col + span_len;

        if span_end <= start || span_start > end {
            spans.push(Span::styled(span.text.clone(), span.style));
            col = span_end;
            continue;
        }

        let selected_start = start.saturating_sub(span_start);
        let selected_end_exclusive = (end + 1).saturating_sub(span_start).min(span_len);

        if selected_start > 0 {
            spans.push(Span::styled(
                line_slice(&span.text, 0, selected_start),
                span.style,
            ));
        }
        if selected_start < selected_end_exclusive {
            spans.push(Span::styled(
                line_slice(&span.text, selected_start, selected_end_exclusive),
                span.style.add_modifier(Modifier::REVERSED),
            ));
        }
        if selected_end_exclusive < span_len {
            spans.push(Span::styled(
                line_slice(&span.text, selected_end_exclusive, span_len),
                span.style,
            ));
        }

        col = span_end;
    }

    Line::from(spans)
}

fn line_slice(line: &str, start: usize, end: usize) -> String {
    line.chars().skip(start).take(end - start).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        build_terminal_view, should_replay, stable_live_len, ReplayPolicy, TerminalViewRequest,
    };
    use crate::app::{SystemState, TextSelection, TranscriptMode};
    use crate::models::AgentType;
    use uuid::Uuid;

    #[test]
    fn normal_screen_uses_live_parser_with_raw_data_when_within_live_scrollback() {
        assert!(!should_replay(
            false,
            0,
            10,
            true,
            ReplayPolicy::NormalAndAlternate
        ));
        assert!(!should_replay(
            false,
            10,
            10,
            true,
            ReplayPolicy::NormalAndAlternate
        ));
    }

    #[test]
    fn normal_screen_replays_only_beyond_live_scrollback() {
        assert!(should_replay(
            false,
            11,
            10,
            true,
            ReplayPolicy::NormalAndAlternate
        ));
        assert!(!should_replay(
            false,
            11,
            10,
            false,
            ReplayPolicy::NormalAndAlternate
        ));
    }

    #[test]
    fn alternate_screen_replay_depends_on_policy() {
        assert!(!should_replay(
            true,
            0,
            0,
            true,
            ReplayPolicy::NormalAndAlternate
        ));
        assert!(should_replay(
            true,
            1,
            0,
            true,
            ReplayPolicy::NormalAndAlternate
        ));
        assert!(!should_replay(true, 1, 0, true, ReplayPolicy::NormalOnly));
    }

    #[test]
    fn stable_live_len_ignores_small_transient_shrinks() {
        assert_eq!(stable_live_len(25, 30), 30);
        assert_eq!(stable_live_len(10, 30), 10);
        assert_eq!(stable_live_len(31, 30), 31);
    }

    #[test]
    fn scrolled_view_prefers_transcript_buffer_when_present() {
        let mut system = SystemState::new();
        let session_id = Uuid::new_v4();
        // Codex-style: repaints a fixed 6-row viewport with the input box ("> p")
        // and footer pinned at the bottom while content slides up. FrameAlign
        // commits the line that scrolls off the top.
        system.create_session_buffers(session_id, 6, 40, &AgentType::Codex);

        system
            .output_buffers
            .get_mut(&session_id)
            .unwrap()
            .process(b"\x1b[2J\x1b[Ha\r\nb\r\nc\r\nd\r\n> p\r\nftr");
        system.update_transcript_from_screen(session_id, TranscriptMode::FrameAlign);

        system
            .output_buffers
            .get_mut(&session_id)
            .unwrap()
            .process(b"\x1b[2J\x1b[Hb\r\nc\r\nd\r\ne\r\n> p\r\nftr");
        system.update_transcript_from_screen(session_id, TranscriptMode::FrameAlign);

        // History = committed ["a"] ++ visible 6-row frame = 7 lines.
        let view = build_terminal_view(
            &mut system,
            TerminalViewRequest {
                session_id,
                viewport_height: 2,
                scroll_from_bottom: 6,
                prev_content_len: 2,
                was_on_replay: false,
                selection: TextSelection::default(),
                replay_policy: ReplayPolicy::NormalAndAlternate,
            },
        )
        .unwrap();

        assert!(view.on_replay);
        assert_eq!(view.content_len, 7);
        assert_eq!(view.scroll_from_bottom, 5);
    }
}
