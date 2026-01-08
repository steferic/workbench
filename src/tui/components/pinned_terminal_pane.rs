use crate::app::{AppState, FocusPanel, InputMode, TextSelection};
use crate::tui::utils::{convert_vt100_to_lines, get_cursor_info, get_selection_bounds, render_cursor};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

/// Render a specific pinned terminal pane at the given index
pub fn render_at(frame: &mut Frame, area: Rect, state: &mut AppState, pane_index: usize) {
    let is_focused = matches!(state.ui.focus, FocusPanel::PinnedTerminalPane(idx) if idx == pane_index);

    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::Magenta)
    };

    let title = state
        .pinned_terminal_session_at(pane_index)
        .map(|s| format!(" {} [pinned {}] ", s.agent_type.display_name(), pane_index + 1))
        .unwrap_or_else(|| format!(" Pinned {} ", pane_index + 1));

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner_area = block.inner(area);

    let mut cursor_state = None;
    // Convert vt100 parser output to ratatui Lines
    let lines: Vec<Line> = if let Some(parser) = state.pinned_terminal_output_at(pane_index) {
        let screen = parser.screen();
        cursor_state = Some(get_cursor_info(screen));
        let default_selection = TextSelection::default();
        let selection = get_selection_bounds(
            state
                .ui.pinned_text_selections
                .get(pane_index)
                .unwrap_or(&default_selection),
            screen.size(),
        );
        convert_vt100_to_lines(screen, selection)
    } else {
        state.ui.pinned_content_lengths[pane_index] = 0;
        vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No terminal in this slot",
                Style::default().fg(Color::Gray),
            )),
        ]
    };

    let content_length = lines.len();
    state.ui.pinned_content_lengths[pane_index] = content_length;
    let viewport_height = inner_area.height as usize;

    // Calculate scroll position - scroll_offset is offset from bottom
    // 0 = show bottom (latest), higher = scroll up to see older content
    let max_scroll = content_length.saturating_sub(viewport_height);
    let scroll_from_bottom = (state.ui.pinned_scroll_offsets[pane_index] as usize).min(max_scroll);
    let scroll_offset = max_scroll.saturating_sub(scroll_from_bottom);

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

    if is_focused && state.ui.input_mode == InputMode::Normal && scroll_from_bottom == 0 {
        if let Some(info) = cursor_state {
            render_cursor(frame, inner_area, info, scroll_offset);
        }
    }
}
