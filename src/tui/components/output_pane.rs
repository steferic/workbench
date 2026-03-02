use crate::app::{AppState, FocusPanel, InputMode, ReplayCache};
use crate::tui::replay::create_replay_parser;
use crate::tui::utils::{convert_vt100_to_lines_visible, get_content_length, get_cursor_info, get_selection_bounds, render_cursor};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        calendar::{CalendarEventStore, Monthly},
        Bar, BarChart, BarGroup, Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Wrap,
    },
    Frame,
};
use time::{Date, Month, OffsetDateTime};

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
        // Render pie chart view with split layout
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
        } else if let Some(parser) = state.system.output_buffers.get(&session_id) {
        let screen = parser.screen();
        let cursor_state = get_cursor_info(screen);
        let inner_area = block.inner(area);
        let viewport_height = inner_area.height as usize;
        let is_alternate = screen.alternate_screen();
        let screen_size = screen.size();

        // Get live parser content length
        let live_content_len = if is_alternate {
            viewport_height
        } else {
            get_content_length(screen, cursor_state.row)
        };

        // Check if we need replay (scrolled beyond what the live parser buffer can show)
        let scroll_from_bottom_raw = state.ui.output_scroll_offset as usize;
        let live_max_scroll = live_content_len.saturating_sub(viewport_height);
        let needs_replay = !is_alternate
            && scroll_from_bottom_raw > live_max_scroll
            && state.system.raw_output_buffers.get(&session_id).map(|b| !b.bytes.is_empty()).unwrap_or(false);

        // Inline sessions (Codex): render styled snapshot history with word wrapping.
        // Uses pre-captured styled Lines (with colors/bold preserved) instead of
        // replaying raw bytes through a vt100 parser.
        if needs_replay && state.system.inline_mode_sessions.contains(&session_id) {
            if let Some(history) = state.system.inline_styled_history.get(&session_id) {
                let text_lines: Vec<Line> = history.clone();

                // Calculate visual line count accounting for word wrapping
                let pane_width = inner_area.width.max(1) as usize;
                let visual_lines: usize = text_lines.iter().map(|line| {
                    let w = line.width();
                    if w == 0 { 1 } else { (w + pane_width - 1) / pane_width }
                }).sum();

                let _ = parser;
                state.ui.output_content_length = visual_lines;

                let max_scroll = visual_lines.saturating_sub(viewport_height);
                let sfb = scroll_from_bottom_raw.min(max_scroll);
                let so = max_scroll.saturating_sub(sfb);

                let session = state.active_session();
                let display_name = session.map(|s| s.agent_type.display_name()).unwrap_or_else(|| "Session".to_string());
                let short_id = session.map(|s| s.short_id()).unwrap_or_default();
                let duration = session.map(|s| s.duration_string()).unwrap_or_default();
                let title = if sfb > 0 {
                    format!(" {} - {} - {} [↑{}] ", display_name, short_id, duration, sfb)
                } else {
                    format!(" {} - {} - {} ", display_name, short_id, duration)
                };

                let block = Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(border_style);

                let paragraph = Paragraph::new(text_lines)
                    .block(block)
                    .wrap(Wrap { trim: true })
                    .scroll((so as u16, 0));

                frame.render_widget(paragraph, area);

                if visual_lines > viewport_height {
                    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
                    let mut scrollbar_state = ScrollbarState::new(max_scroll).position(so);
                    frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
                }
                return;
            }
        }

        let (lines, stable_len, scroll_from_bottom, scroll_offset) = if needs_replay {
            // Replay path: use cached replay parser for deep scrollback.
            // The parser is expensive to create (replays all raw bytes), but
            // rendering visible lines from it each frame is cheap.
            let raw_buf = state.system.raw_output_buffers.get(&session_id).unwrap();
            let generation = raw_buf.generation;
            let cols = screen_size.1;

            // Check if cached parser is still valid (same generation + cols)
            let cache_valid = state.system.replay_caches.get(&session_id).map(|c| {
                c.generation == generation && c.cols == cols
            }).unwrap_or(false);

            if !cache_valid {
                // Create replay parser and cache it
                let replay_parser = create_replay_parser(raw_buf, cols, state.system.user_config.replay_parser_rows);
                let replay_screen = replay_parser.screen();
                let replay_cursor = get_cursor_info(replay_screen);
                let replay_content_len = get_content_length(replay_screen, replay_cursor.row);

                state.system.replay_caches.insert(session_id, ReplayCache {
                    generation,
                    cols,
                    parser: replay_parser,
                    content_length: replay_content_len,
                });
            }

            // Render visible lines from the cached parser (cheap — only visible rows)
            let cache = state.system.replay_caches.get(&session_id).unwrap();
            let replay_content_len = cache.content_length;
            let replay_screen = cache.parser.screen();
            let replay_cursor = get_cursor_info(replay_screen);

            // On live→replay transition, translate selection coordinates.
            // The bottom of both parsers shows the same text, so live row R maps to
            // replay row R + (replay_content_len - live_content_len).
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

            let selection = get_selection_bounds(&state.ui.text_selection, replay_content_len, replay_screen.size().1);
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
            let stable_len = if live_content_len >= prev_len {
                live_content_len
            } else if prev_len - live_content_len >= 20 {
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

        // Show scroll indicator in title if scrolled (cache active_session to avoid repeated lookups)
        let session = state.active_session();
        let display_name = session.map(|s| s.agent_type.display_name()).unwrap_or_else(|| "Session".to_string());
        let short_id = session.map(|s| s.short_id()).unwrap_or_default();
        let duration = session.map(|s| s.duration_string()).unwrap_or_default();
        let title = if scroll_from_bottom > 0 {
            format!(" {} - {} - {} [↑{}] ", display_name, short_id, duration, scroll_from_bottom)
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
        if stable_len > viewport_height {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
            let mut scrollbar_state = ScrollbarState::new(stable_len.saturating_sub(viewport_height)).position(scroll_offset);
            frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
        }

        if is_focused && state.ui.input_mode == InputMode::Normal && scroll_from_bottom == 0 {
            // Terminal sessions and Codex need the terminal cursor shown
            // Claude/Gemini draw their own visual cursor using inverse video
            let needs_terminal_cursor = session
                .map(|s| s.agent_type.is_terminal() || matches!(s.agent_type, crate::models::AgentType::Codex))
                .unwrap_or(false);

            if needs_terminal_cursor {
                render_cursor(frame, inner_area, cursor_state, scroll_offset, true);
            }
        }
        return;
    } } // end of active session block

    // No active session - show utility content or hints
    let lines: Vec<Line> = if !state.ui.utility_content.is_empty() {
        // Show utility content when no active session
        state
            .ui.utility_content
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

    // Calculate max scroll offset (can't scroll past content)
    let max_scroll = content_length.saturating_sub(viewport_height);
    let scroll_offset = (state.ui.output_scroll_offset as usize).min(max_scroll);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((scroll_offset as u16, 0));

    frame.render_widget(paragraph, area);

    // Render scrollbar if content exceeds viewport
    if content_length > viewport_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        let mut scrollbar_state = ScrollbarState::new(max_scroll).position(scroll_offset);
        frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
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

    // Split into bar chart area (top) and legend area (bottom)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(50), // Bar chart
            Constraint::Percentage(50), // Legend/text
        ])
        .split(inner_area);

    let chart_area = chunks[0];
    let legend_area = chunks[1];

    // Build bars from state data
    if !state.ui.pie_chart_data.is_empty() {
        let bars: Vec<Bar> = state
            .ui.pie_chart_data
            .iter()
            .map(|(label, value, color)| {
                // Truncate label to fit
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
    let bullet_indices: Vec<Option<usize>> = state.ui.utility_content.iter().map(|line| {
        if line.contains('●') || line.contains('○') {
            let idx = bullet_count;
            bullet_count += 1;
            Some(idx)
        } else {
            None
        }
    }).collect();

    // Render the text content (legend) below the chart
    let lines: Vec<Line> = state
        .ui.utility_content
        .iter()
        .enumerate()
        .map(|(i, line)| {
            // Color the bullet points to match chart bars
            if let Some(color_idx) = bullet_indices[i] {
                if color_idx < state.ui.pie_chart_data.len() {
                    let (_, _, color) = &state.ui.pie_chart_data[color_idx];
                    // Split at 3rd character boundary using char_indices
                    let split_at = line.char_indices().nth(3).map(|(i, _)| i).unwrap_or(line.len());
                    return Line::from(vec![
                        Span::styled(
                            line[..split_at].to_string(),
                            Style::default().fg(*color),
                        ),
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

    // Split into calendar area (top) and legend area (bottom)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(12),   // Calendar (needs at least 12 rows for 3 months)
            Constraint::Length(8), // Legend/workspace info
        ])
        .split(inner_area);

    let calendar_area = chunks[0];
    let legend_area = chunks[1];

    // Get current date
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let today = now.date();
    let current_year = today.year();
    let current_month = today.month();

    // Create event store with today highlighted
    let events = CalendarEventStore::today(
        Style::default()
            .add_modifier(Modifier::BOLD)
            .bg(Color::Blue)
            .fg(Color::White),
    );

    // Calculate how many months we can fit
    // Each month needs about 22 chars wide and 9 rows tall
    let cols_available = (calendar_area.width / 24).max(1) as usize;
    let rows_available = (calendar_area.height / 9).max(1) as usize;
    let total_months = (cols_available * rows_available).min(12);

    // Create layout for calendar grid
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

    // Calculate starting month (center around current month)
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

            // Calculate the month to display
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

    // Render the legend/workspace info below
    let lines: Vec<Line> = state
        .ui.utility_content
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
