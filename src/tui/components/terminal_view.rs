use crate::app::{SystemState, TextSelection, TranscriptBuffer};
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
    pub links: Vec<crate::links::Link>,
    /// A window of content rows starting at `window_start` — NOT the full
    /// content. Renderers must scroll by `scroll_offset - window_start`.
    /// Materializing lines for the whole history just so Paragraph::scroll
    /// could skip them cost O(history) allocations per frame.
    pub lines: Vec<Line<'static>>,
    /// Absolute content row of `lines[0]`.
    pub window_start: usize,
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
    if request.scroll_from_bottom > 0
        && system.native_history_sessions.contains(&request.session_id)
        && system.native_history_dirty.remove(&request.session_id)
    {
        let lines = crate::scrollback::terminal_rows(
            system.output_buffers.get(&request.session_id)?.screen(),
        );
        system
            .transcript_buffers
            .entry(request.session_id)
            .or_insert_with(|| TranscriptBuffer::new(system.user_config.transcript_max_lines))
            .set_log_history(Some(lines));
    }
    let parser = system.output_buffers.get(&request.session_id)?;
    let screen = parser.screen();
    let cursor = get_cursor_info(screen);
    let is_alternate = screen.alternate_screen();
    let screen_cols = screen.size().1;

    if let Some(transcript) = system.transcript_buffers.get_mut(&request.session_id) {
        transcript.reflow(screen_cols);
    }

    let live_content_len = if is_alternate {
        request.viewport_height
    } else {
        get_content_length(screen, cursor.row)
    };
    let transcript_len = system
        .transcript_buffers
        .get(&request.session_id)
        .map(|buffer| buffer.len())
        .unwrap_or(0);
    let use_transcript = transcript_len > 0
        && request.scroll_from_bottom > 0
        && (!is_alternate || request.replay_policy == ReplayPolicy::NormalAndAlternate)
        && system
            .transcript_buffers
            .get(&request.session_id)
            .map(|buffer| !buffer.is_empty())
            .unwrap_or(false);
    let view = if use_transcript {
        let transcript = system.transcript_buffers.get_mut(&request.session_id)?;
        let content_len = transcript.len();
        let (scroll_from_bottom, scroll_offset) =
            transcript.reading_position(request.scroll_from_bottom, request.viewport_height);
        let selection = translate_selection(
            request.selection,
            request.was_on_replay,
            true,
            request.prev_content_len,
            content_len,
        );
        let selection_bounds = get_selection_bounds(&selection, content_len, screen_cols);
        TerminalView {
            links: (scroll_offset..(scroll_offset + request.viewport_height).min(transcript.len()))
                .flat_map(|row| {
                    transcript
                        .styled_line(row)
                        .into_iter()
                        .flat_map(move |line| {
                            line.links.iter().cloned().map(move |mut link| {
                                link.row = row - scroll_offset;
                                link
                            })
                        })
                })
                .collect(),
            lines: transcript_lines(
                transcript,
                selection_bounds,
                request.viewport_height,
                scroll_offset,
            ),
            window_start: window_start_for(scroll_offset),
            content_len,
            scrollbar_content_len: content_len,
            scroll_from_bottom,
            scroll_offset,
            on_replay: true,
            selection,
            cursor,
        }
    } else {
        if let Some(transcript) = system.transcript_buffers.get_mut(&request.session_id) {
            transcript.follow_live();
        }
        let content_len = if request.was_on_replay {
            live_content_len
        } else {
            stable_live_len(live_content_len, request.prev_content_len)
        };
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
        let native_len = if system.native_history_sessions.contains(&request.session_id) {
            screen.history_len()
        } else {
            0
        };
        let scrollbar_content_len = native_len.max(transcript_len).max(content_len);

        TerminalView {
            links: crate::links::screen(screen, scroll_offset, request.viewport_height),
            lines: visible_lines(
                screen,
                selection_bounds,
                cursor.row,
                request.viewport_height,
                scroll_offset,
            ),
            window_start: window_start_for(scroll_offset),
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

fn stable_live_len(live_content_len: usize, prev_content_len: usize) -> usize {
    if live_content_len >= prev_content_len || prev_content_len - live_content_len >= 20 {
        live_content_len
    } else {
        prev_content_len
    }
}

fn translate_selection(
    selection: TextSelection,
    was_on_replay: bool,
    on_replay: bool,
    _prev_content_len: usize,
    _current_content_len: usize,
) -> TextSelection {
    // Live terminal cells and a structured transcript have independent row
    // coordinates. An offset cannot translate a selection between them.
    if was_on_replay != on_replay {
        TextSelection::default()
    } else {
        selection
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

/// Absolute content row where the rendered window begins.
fn window_start_for(scroll_offset: usize) -> usize {
    scroll_offset.saturating_sub(VISIBLE_BUFFER_LINES)
}

fn visible_lines(
    screen: &vt100::Screen,
    selection: Option<crate::tui::utils::SelectionBounds>,
    cursor_row: u16,
    viewport_height: usize,
    scroll_offset: usize,
) -> Vec<Line<'static>> {
    let window_start = window_start_for(scroll_offset);
    let visible_count = viewport_height + VISIBLE_BUFFER_LINES * 2;

    let mut lines = convert_vt100_to_lines_visible(
        screen,
        selection,
        cursor_row,
        Some(viewport_height as u16),
        Some(window_start),
        Some(visible_count),
    );

    // Pad so the scrolled viewport always has a full page of rows to show.
    let min_window_len = scroll_offset + viewport_height - window_start;
    while lines.len() < min_window_len {
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
    let window_start = window_start_for(scroll_offset);
    let visible_count = viewport_height + VISIBLE_BUFFER_LINES * 2;
    let visible_end = (window_start + visible_count).min(transcript.len());

    let mut lines = Vec::new();
    for row in window_start..visible_end {
        if let Some(line) = transcript.styled_line(row) {
            lines.push(transcript_line(row, line, selection));
        } else {
            lines.push(Line::raw(""));
        }
    }

    // Pad so the scrolled viewport always has a full page of rows to show.
    let min_window_len = scroll_offset + viewport_height - window_start;
    while lines.len() < min_window_len {
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

    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    let start = if row == bounds.start_row {
        bounds.start_col
    } else {
        0
    };
    let end = if row == bounds.end_row {
        bounds.end_col
    } else {
        usize::MAX
    };
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut column = 0;
    let mut selected = false;
    for span in line.spans() {
        for ch in span.text.graphemes(true) {
            let width = ch.width();
            if width > 0 {
                selected = column <= end && column + width > start;
            }
            let style = if selected {
                span.style.add_modifier(Modifier::REVERSED)
            } else {
                span.style
            };
            match spans.last_mut() {
                Some(last) if last.style == style => last.content.to_mut().push_str(ch),
                _ => spans.push(Span::styled(ch.to_string(), style)),
            }
            column += width;
        }
    }

    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::{build_terminal_view, stable_live_len, ReplayPolicy, TerminalViewRequest};
    use crate::app::{SystemState, TextSelection};
    use crate::models::AgentType;
    use uuid::Uuid;

    #[test]
    fn structured_history_reflows_from_cached_records_without_live_screen_duplicates() {
        use std::io::Write;
        use unicode_width::UnicodeWidthStr;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "{}", serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text":"## Answer\n\nRead [the documentation](https://example.com/docs) for a longer explanation.\n\nThe final answer."}]}})).unwrap();
        let mut system = SystemState::new();
        let id = Uuid::new_v4();
        system.create_session_buffers(id, 4, 40, &AgentType::Codex);
        system
            .output_buffers
            .get_mut(&id)
            .unwrap()
            .process(b"The final answer.\r\n> input chrome");
        system.update_transcript_from_screen(id);
        let history = crate::scrollback::history(
            crate::scrollback::LogFormat::Claude,
            file.path(),
            40,
            crate::theme::Theme::DARK,
        )
        .unwrap();
        system
            .transcript_buffers
            .get_mut(&id)
            .unwrap()
            .set_document(history, 40);
        drop(file); // Resize must use the cached document, not re-read a file.
        let mut request = TerminalViewRequest {
            session_id: id,
            viewport_height: 4,
            scroll_from_bottom: 1,
            prev_content_len: 4,
            was_on_replay: false,
            selection: TextSelection::default(),
            replay_policy: ReplayPolicy::NormalAndAlternate,
        };
        let view = build_terminal_view(&mut system, request).unwrap();
        let shown: String = view
            .lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert_eq!(shown.matches("The final answer.").count(), 1);
        assert!(!shown.contains("input chrome"));
        let anchor = system.transcript_buffers[&id]
            .styled_line(view.scroll_offset)
            .unwrap()
            .anchor;
        request.scroll_from_bottom = view.scroll_from_bottom;
        request.prev_content_len = view.content_len;
        request.was_on_replay = true;
        system.output_buffers.get_mut(&id).unwrap().set_size(4, 20);
        let view = build_terminal_view(&mut system, request).unwrap();
        assert!(view.lines.iter().all(|l| l
            .spans
            .iter()
            .map(|s| s.content.width())
            .sum::<usize>()
            <= 20));
        assert_eq!(
            system.transcript_buffers[&id]
                .styled_line(view.scroll_offset)
                .unwrap()
                .anchor
                .unwrap()
                .0,
            anchor.unwrap().0
        );
        assert!(view
            .links
            .iter()
            .all(|l| l.end <= 20 && l.target == "https://example.com/docs"));
        request.scroll_from_bottom = 0;
        let live = build_terminal_view(&mut system, request).unwrap();
        assert!(!live.on_replay);
        assert!(live.content_len <= 4);
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
        system.update_transcript_from_screen(session_id);

        system
            .output_buffers
            .get_mut(&session_id)
            .unwrap()
            .process(b"\x1b[2J\x1b[Hb\r\nc\r\nd\r\ne\r\n> p\r\nftr");
        system.update_transcript_from_screen(session_id);

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

    #[test]
    fn claude_scrollback_shows_transcript_when_scrolled() {
        let mut system = SystemState::new();
        let session_id = Uuid::new_v4();
        // Claude: a 10-row parser fed many lines scrolls one line per frame;
        // FrameAlign detects the shift and accumulates history beyond the viewport.
        system.create_session_buffers(session_id, 10, 40, &AgentType::Claude);

        for i in 1..=40 {
            system
                .output_buffers
                .get_mut(&session_id)
                .unwrap()
                .process(format!("line {i}\r\n").as_bytes());
            system.update_transcript_from_screen(session_id);
        }

        let transcript_len = system.transcript_buffers.get(&session_id).unwrap().len();
        assert!(
            transcript_len > 10,
            "history not accumulated: {transcript_len}"
        );

        // Scrolling up past the live screen must switch to the transcript.
        let view = build_terminal_view(
            &mut system,
            TerminalViewRequest {
                session_id,
                viewport_height: 8,
                scroll_from_bottom: transcript_len, // scroll all the way up
                prev_content_len: 8,
                was_on_replay: false,
                selection: TextSelection::default(),
                replay_policy: ReplayPolicy::NormalAndAlternate,
            },
        )
        .unwrap();

        assert!(
            view.on_replay,
            "expected transcript (history) view when scrolled"
        );
        assert!(
            view.content_len > 10,
            "history view too short: {}",
            view.content_len
        );
    }
}
