use crate::app::{AppState, FocusPanel, InputMode, ReplayCache};
use crate::tui::replay::create_replay_parser;
use crate::tui::utils::{
    convert_vt100_to_lines_visible, get_content_length, get_cursor_info, get_selection_bounds,
    render_cursor,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        calendar::{CalendarEventStore, Monthly},
        Bar, BarChart, BarGroup, Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState,
    },
    Frame,
};
use time::{Date, Month, OffsetDateTime};
use uuid::Uuid;

pub fn render(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let is_focused = state.ui.focus == FocusPanel::OutputPane;

    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let title = if let Some(session) = state.active_session() {
        format!(
            " {} - {} - {} ",
            session.agent_type.display_name(),
            session.short_id(),
            session.duration_string()
        )
    } else if !state.ui.utility_content.is_empty() {
        format!(" {} ", state.ui.selected_utility.name())
    } else {
        " No Active Session ".to_string()
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    // Check if we should render a pie chart (TopFiles utility)
    let has_pie_chart = !state.ui.pie_chart_data.is_empty() && state.ui.active_session_id.is_none();

    if has_pie_chart {
        state.ui.output_content_length = 0;
        render_pie_chart_view(frame, area, state, block);
        return;
    }

    // Check if we should render a calendar (Calendar utility)
    if state.ui.show_calendar && state.ui.active_session_id.is_none() {
        state.ui.output_content_length = 0;
        render_calendar_view(frame, area, state, block);
        return;
    }

    // Render terminal output with scrolling support
    if let Some(session_id) = state.ui.active_session_id {
        // Don't show pinned terminal in output pane when split view is active
        if state.should_show_split() && state.active_is_pinned() {
            // Fall through to utility/hints rendering below
        } else if state.system.output_buffers.contains_key(&session_id) {
            render_session_output(frame, area, state, session_id, border_style, is_focused);
            return;
        }
    }

    // No active session - show utility content or hints
    let lines: Vec<Line> = if !state.ui.utility_content.is_empty() {
        state
            .ui
            .utility_content
            .iter()
            .map(|line| Line::from(Span::styled(line.clone(), Style::default().fg(Color::Gray))))
            .collect()
    } else {
        render_hints(state)
    };

    let inner_area = block.inner(area);
    let content_length = lines.len();
    state.ui.output_content_length = 0;
    let viewport_height = inner_area.height as usize;

    let max_scroll = content_length.saturating_sub(viewport_height);
    let scroll_offset = (state.ui.output_scroll_offset as usize).min(max_scroll);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((scroll_offset as u16, 0));

    frame.render_widget(paragraph, area);

    if content_length > viewport_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        let mut scrollbar_state = ScrollbarState::new(max_scroll).position(scroll_offset);
        frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
    }
}

fn needs_replay(
    is_alternate: bool,
    scroll_from_bottom: usize,
    live_max_scroll: usize,
    has_raw_data: bool,
) -> bool {
    has_raw_data
        && if is_alternate {
            scroll_from_bottom > 0
        } else {
            scroll_from_bottom > live_max_scroll
        }
}

/// Render the terminal output for an active session (live or replay).
fn render_session_output(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    session_id: Uuid,
    border_style: Style,
    is_focused: bool,
) {
    let Some(parser) = state.system.output_buffers.get(&session_id) else {
        return;
    };
    let screen = parser.screen();
    let cursor_state = get_cursor_info(screen);
    let block_for_inner = Block::default().borders(Borders::ALL);
    let inner_area = block_for_inner.inner(area);
    let viewport_height = inner_area.height as usize;
    let is_alternate = screen.alternate_screen();
    let screen_size = screen.size();

    // Get live parser content length
    let scroll_from_bottom_raw = state.ui.output_scroll_offset as usize;
    let live_content_len = if is_alternate {
        viewport_height
    } else {
        get_content_length(screen, cursor_state.row)
    };

    // Check if we need replay (scrolled beyond what the live parser buffer can show)
    let live_max_scroll = live_content_len.saturating_sub(viewport_height);
    let has_raw_data = state
        .system
        .raw_output_buffers
        .get(&session_id)
        .map(|b| !b.bytes.is_empty())
        .unwrap_or(false);

    let needs_replay = needs_replay(
        is_alternate,
        scroll_from_bottom_raw,
        live_max_scroll,
        has_raw_data,
    );

    let (lines, stable_len, scroll_from_bottom, scroll_offset) = if needs_replay {
        // Replay path: use cached replay parser for deep scrollback.
        let Some(raw_buf) = state.system.raw_output_buffers.get(&session_id) else {
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
            let replay_parser =
                create_replay_parser(raw_buf, cols, state.system.user_config.replay_parser_rows);
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

        let Some(cache) = state.system.replay_caches.get(&session_id) else {
            return;
        };
        let replay_content_len = cache.content_length;
        let replay_screen = cache.parser.screen();
        let replay_cursor = get_cursor_info(replay_screen);

        // On live→replay transition, translate selection coordinates.
        if !state.ui.output_on_replay {
            let prev_content_len = state.ui.output_content_length;
            if replay_content_len > prev_content_len && prev_content_len > 0 {
                let offset = replay_content_len - prev_content_len;
                if let Some((row, col)) = state.ui.text_selection.start {
                    state.ui.text_selection.start = Some((row + offset, col));
                }
                if let Some((row, col)) = state.ui.text_selection.end {
                    state.ui.text_selection.end = Some((row + offset, col));
                }
            }
            state.ui.output_on_replay = true;
        }

        let selection = get_selection_bounds(
            &state.ui.text_selection,
            replay_content_len,
            replay_screen.size().1,
        );
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

        (replay_lines, replay_content_len, sfb_clamped, so)
    } else {
        // Live parser path (at bottom or within live parser range)
        // On replay→live transition, translate selection coordinates back
        if state.ui.output_on_replay {
            let prev_content_len = state.ui.output_content_length;
            if prev_content_len > live_content_len && live_content_len > 0 {
                let offset = prev_content_len - live_content_len;
                if let Some((row, col)) = state.ui.text_selection.start {
                    state.ui.text_selection.start = Some((row.saturating_sub(offset), col));
                }
                if let Some((row, col)) = state.ui.text_selection.end {
                    state.ui.text_selection.end = Some((row.saturating_sub(offset), col));
                }
            }
            state.ui.output_on_replay = false;
        }

        let prev_len = state.ui.output_content_length;
        let stable_len = if live_content_len >= prev_len || prev_len - live_content_len >= 20 {
            live_content_len
        } else {
            prev_len
        };

        let max_scroll = stable_len.saturating_sub(viewport_height);
        let sfb = scroll_from_bottom_raw.min(max_scroll);
        let so = max_scroll.saturating_sub(sfb);

        let buffer_lines = 5;
        let visible_start = so.saturating_sub(buffer_lines);
        let visible_count = viewport_height + buffer_lines * 2;

        let selection = get_selection_bounds(&state.ui.text_selection, stable_len, screen_size.1);
        let pane_height = Some(viewport_height as u16);
        let mut lines = convert_vt100_to_lines_visible(
            screen,
            selection,
            cursor_state.row,
            pane_height,
            Some(visible_start),
            Some(visible_count),
        );

        while lines.len() < stable_len {
            lines.push(Line::raw(""));
        }

        (lines, stable_len, sfb, so)
    };

    // Drop parser borrow
    let _ = parser;
    state.ui.output_content_length = stable_len;

    // Show scroll indicator in title if scrolled
    let session = state.active_session();
    let display_name = session
        .map(|s| s.agent_type.display_name())
        .unwrap_or_else(|| "Session".to_string());
    let short_id = session.map(|s| s.short_id()).unwrap_or_default();
    let duration = session.map(|s| s.duration_string()).unwrap_or_default();
    let title = if scroll_from_bottom > 0 {
        format!(
            " {} - {} - {} [↑{}] ",
            display_name, short_id, duration, scroll_from_bottom
        )
    } else {
        format!(" {} - {} - {} ", display_name, short_id, duration)
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((scroll_offset as u16, 0));

    frame.render_widget(paragraph, area);

    // Render scrollbar if content exceeds viewport
    // Use the full content length (from replay cache if available) so the
    // scrollbar thumb stays a consistent size whether we're on the live or
    // replay rendering path.
    let scrollbar_total = state
        .system
        .replay_caches
        .get(&session_id)
        .map(|c| c.content_length.max(stable_len))
        .unwrap_or(stable_len);

    if scrollbar_total > viewport_height {
        let scrollbar_max = scrollbar_total.saturating_sub(viewport_height);
        let scrollbar_sfb = (state.ui.output_scroll_offset as usize).min(scrollbar_max);
        let scrollbar_pos = scrollbar_max.saturating_sub(scrollbar_sfb);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        let mut scrollbar_state = ScrollbarState::new(scrollbar_max).position(scrollbar_pos);
        frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
    }

    if is_focused && state.ui.input_mode == InputMode::Normal && scroll_from_bottom == 0 {
        let needs_terminal_cursor = session
            .map(|s| {
                s.agent_type.is_terminal()
                    || matches!(s.agent_type, crate::models::AgentType::Codex)
            })
            .unwrap_or(false);

        if needs_terminal_cursor {
            render_cursor(frame, inner_area, cursor_state, scroll_offset, true);
        }
    }
}

/// Render hint text when no session is active
fn render_hints(state: &AppState) -> Vec<Line<'static>> {
    if state.data.workspaces.is_empty() {
        vec![
            Line::from(""),
            Line::from(Span::styled(
                "  Welcome to Workbench!",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Press 'n' to create a new workspace",
                Style::default().fg(Color::Gray),
            )),
            Line::from(Span::styled(
                "  Press '?' for help",
                Style::default().fg(Color::Gray),
            )),
        ]
    } else if state.sessions_for_selected_workspace().is_empty() {
        vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No sessions in this workspace",
                Style::default().fg(Color::Gray),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Press 1-4 to start a new session:",
                Style::default().fg(Color::Gray),
            )),
            Line::from(Span::styled(
                "    1 = Claude, 2 = Gemini, 3 = Codex, 4 = Grok",
                Style::default().fg(Color::DarkGray),
            )),
        ]
    } else {
        vec![
            Line::from(""),
            Line::from(Span::styled(
                "  Select a session and press Enter to view output",
                Style::default().fg(Color::Gray),
            )),
        ]
    }
}

/// Render a bar chart view with chart on top and legend below
fn render_pie_chart_view(frame: &mut Frame, area: Rect, state: &AppState, block: Block) {
    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner_area);

    let chart_area = chunks[0];
    let legend_area = chunks[1];

    if !state.ui.pie_chart_data.is_empty() {
        let bars: Vec<Bar> = state
            .ui
            .pie_chart_data
            .iter()
            .map(|(label, value, color)| {
                let short_label: String = label.chars().take(8).collect();
                Bar::default()
                    .value(*value as u64)
                    .label(Line::from(short_label))
                    .style(Style::default().fg(*color))
            })
            .collect();

        let bar_chart = BarChart::default()
            .data(BarGroup::default().bars(&bars))
            .bar_width(6)
            .bar_gap(1)
            .bar_set(symbols::bar::NINE_LEVELS)
            .direction(Direction::Vertical);

        frame.render_widget(bar_chart, chart_area);
    }

    // Pre-compute bullet color indices to avoid O(n²) scanning
    let mut bullet_count = 0usize;
    let bullet_indices: Vec<Option<usize>> = state
        .ui
        .utility_content
        .iter()
        .map(|line| {
            if line.contains('●') || line.contains('○') {
                let idx = bullet_count;
                bullet_count += 1;
                Some(idx)
            } else {
                None
            }
        })
        .collect();

    let lines: Vec<Line> = state
        .ui
        .utility_content
        .iter()
        .enumerate()
        .map(|(i, line)| {
            if let Some(color_idx) = bullet_indices[i] {
                if color_idx < state.ui.pie_chart_data.len() {
                    let (_, _, color) = &state.ui.pie_chart_data[color_idx];
                    let split_at = line
                        .char_indices()
                        .nth(3)
                        .map(|(i, _)| i)
                        .unwrap_or(line.len());
                    return Line::from(vec![
                        Span::styled(line[..split_at].to_string(), Style::default().fg(*color)),
                        Span::styled(
                            line[split_at..].to_string(),
                            Style::default().fg(Color::Gray),
                        ),
                    ]);
                }
            }
            Line::from(Span::styled(line.clone(), Style::default().fg(Color::Gray)))
        })
        .collect();

    let paragraph = Paragraph::new(lines).scroll((state.ui.utility_scroll_offset as u16, 0));

    frame.render_widget(paragraph, legend_area);
}

/// Render a calendar view with monthly calendars
fn render_calendar_view(frame: &mut Frame, area: Rect, state: &AppState, block: Block) {
    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(12), Constraint::Length(8)])
        .split(inner_area);

    let calendar_area = chunks[0];
    let legend_area = chunks[1];

    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let today = now.date();
    let current_year = today.year();
    let current_month = today.month();

    let events = CalendarEventStore::today(
        Style::default()
            .add_modifier(Modifier::BOLD)
            .bg(Color::Blue)
            .fg(Color::White),
    );

    let cols_available = (calendar_area.width / 24).max(1) as usize;
    let rows_available = (calendar_area.height / 9).max(1) as usize;
    let total_months = (cols_available * rows_available).min(12);

    let row_constraints: Vec<Constraint> = (0..rows_available)
        .map(|_| Constraint::Ratio(1, rows_available as u32))
        .collect();
    let col_constraints: Vec<Constraint> = (0..cols_available)
        .map(|_| Constraint::Ratio(1, cols_available as u32))
        .collect();

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(row_constraints)
        .split(calendar_area);

    let months_before = total_months / 2;
    let mut month_offset = -(months_before as i32);

    let default_style = Style::default()
        .add_modifier(Modifier::BOLD)
        .bg(Color::Rgb(40, 40, 40));

    let header_style = Style::default()
        .add_modifier(Modifier::BOLD)
        .fg(Color::Cyan);

    let weekday_style = Style::default()
        .add_modifier(Modifier::DIM)
        .fg(Color::DarkGray);

    for row in rows.iter() {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(col_constraints.clone())
            .split(*row);

        for col in cols.iter() {
            if month_offset >= (total_months as i32 - months_before as i32) {
                break;
            }

            let (year, month) = offset_month(current_year, current_month, month_offset);

            if let Ok(first_day) = Date::from_calendar_date(year, month, 1) {
                let is_current_month = year == current_year && month == current_month;

                let cal = Monthly::new(first_day, &events)
                    .show_month_header(if is_current_month {
                        header_style.fg(Color::Yellow)
                    } else {
                        header_style
                    })
                    .show_weekdays_header(weekday_style)
                    .default_style(default_style);

                frame.render_widget(cal, *col);
            }

            month_offset += 1;
        }
    }

    let lines: Vec<Line> = state
        .ui
        .utility_content
        .iter()
        .map(|line| Line::from(Span::styled(line.clone(), Style::default().fg(Color::Gray))))
        .collect();

    let paragraph = Paragraph::new(lines).scroll((state.ui.utility_scroll_offset as u16, 0));
    frame.render_widget(paragraph, legend_area);
}

/// Calculate year and month with an offset from the given month
fn offset_month(year: i32, month: Month, offset: i32) -> (i32, Month) {
    let month_num = month as i32; // 1-12
    let total_months = (year * 12) + month_num + offset;
    let new_year = (total_months - 1) / 12;
    let new_month_num = ((total_months - 1) % 12) + 1;

    let new_month = match new_month_num {
        1 => Month::January,
        2 => Month::February,
        3 => Month::March,
        4 => Month::April,
        5 => Month::May,
        6 => Month::June,
        7 => Month::July,
        8 => Month::August,
        9 => Month::September,
        10 => Month::October,
        11 => Month::November,
        _ => Month::December,
    };

    (new_year, new_month)
}

#[cfg(test)]
mod tests {
    use super::needs_replay;

    #[test]
    fn normal_screen_uses_live_parser_with_raw_data_when_within_live_scrollback() {
        assert!(!needs_replay(false, 0, 10, true));
        assert!(!needs_replay(false, 10, 10, true));
    }

    #[test]
    fn normal_screen_replays_only_beyond_live_scrollback() {
        assert!(needs_replay(false, 11, 10, true));
        assert!(!needs_replay(false, 11, 10, false));
    }

    #[test]
    fn alternate_screen_replays_when_scrolled_and_raw_data_exists() {
        assert!(!needs_replay(true, 0, 0, true));
        assert!(needs_replay(true, 1, 0, true));
        assert!(!needs_replay(true, 1, 0, false));
    }
}
