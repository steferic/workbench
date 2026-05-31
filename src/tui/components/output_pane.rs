use super::terminal_view::{build_terminal_view, ReplayPolicy, TerminalViewRequest};
use crate::app::{AppState, FocusPanel, InputMode};
use crate::tui::utils::render_cursor;
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
    let scroll_offset = (state.output_scroll_offset() as usize).min(max_scroll);

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

/// Render the terminal output for an active session (live or replay).
fn render_session_output(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    session_id: Uuid,
    border_style: Style,
    is_focused: bool,
) {
    let block_for_inner = Block::default().borders(Borders::ALL);
    let inner_area = block_for_inner.inner(area);
    let viewport_height = inner_area.height as usize;
    let scroll_from_bottom = state.output_scroll_offset() as usize;

    let Some(view) = build_terminal_view(
        &mut state.system,
        TerminalViewRequest {
            session_id,
            viewport_height,
            scroll_from_bottom,
            prev_content_len: state.ui.output_content_length,
            was_on_replay: state.ui.output_on_replay,
            selection: state.ui.text_selection,
            replay_policy: ReplayPolicy::NormalAndAlternate,
        },
    ) else {
        return;
    };

    state.ui.output_content_length = view.content_len;
    state.ui.output_on_replay = view.on_replay;
    state.ui.text_selection = view.selection;

    // Show scroll indicator in title if scrolled
    let session = state.active_session();
    let display_name = session
        .map(|s| s.agent_type.display_name())
        .unwrap_or_else(|| "Session".to_string());
    let short_id = session.map(|s| s.short_id()).unwrap_or_default();
    let duration = session.map(|s| s.duration_string()).unwrap_or_default();
    let title = if view.scroll_from_bottom > 0 {
        format!(
            " {} - {} - {} [↑{}] ",
            display_name, short_id, duration, view.scroll_from_bottom
        )
    } else {
        format!(" {} - {} - {} ", display_name, short_id, duration)
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let paragraph = Paragraph::new(view.lines)
        .block(block)
        .scroll((view.scroll_offset as u16, 0));

    frame.render_widget(paragraph, area);

    if view.scrollbar_content_len > viewport_height {
        let scrollbar_max = view.scrollbar_content_len.saturating_sub(viewport_height);
        let scrollbar_sfb = (state.output_scroll_offset() as usize).min(scrollbar_max);
        let scrollbar_pos = scrollbar_max.saturating_sub(scrollbar_sfb);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        let mut scrollbar_state = ScrollbarState::new(scrollbar_max).position(scrollbar_pos);
        frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
    }

    if is_focused && state.ui.input_mode == InputMode::Normal && view.scroll_from_bottom == 0 {
        let needs_terminal_cursor = session
            .map(|s| s.agent_type.is_terminal() || s.agent_type.is_codex_like())
            .unwrap_or(false);

        if needs_terminal_cursor {
            render_cursor(frame, inner_area, view.cursor, view.scroll_offset, true);
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
