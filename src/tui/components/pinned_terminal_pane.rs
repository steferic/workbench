use super::terminal_view::{build_terminal_view, ReplayPolicy, TerminalViewRequest};
use crate::app::{AppState, FocusPanel, InputMode};
use crate::tui::utils::render_cursor;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

/// Render a specific pinned terminal pane at the given index
pub fn render_at(frame: &mut Frame, area: Rect, state: &mut AppState, pane_index: usize) {
    let t = crate::theme::current();
    let is_focused =
        matches!(state.ui.focus, FocusPanel::PinnedTerminalPane(idx) if idx == pane_index);

    let border_style = if is_focused {
        Style::default().fg(t.border_focused)
    } else {
        Style::default().fg(t.special)
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
    let viewport_height = inner_area.height as usize;

    let Some(session_id) = state.pinned_terminal_id_at(pane_index) else {
        state.ui.pinned_content_lengths[pane_index] = 0;
        render_empty_slot(frame, area, block);
        return;
    };

    let Some(view) = build_terminal_view(
        &mut state.system,
        TerminalViewRequest {
            session_id,
            viewport_height,
            scroll_from_bottom: state.ui.pinned_scroll_offsets[pane_index] as usize,
            prev_content_len: state.ui.pinned_content_lengths[pane_index],
            was_on_replay: state.ui.pinned_on_replay[pane_index],
            selection: state.ui.pinned_text_selections[pane_index],
            replay_policy: ReplayPolicy::NormalOnly,
        },
    ) else {
        state.ui.pinned_content_lengths[pane_index] = 0;
        render_empty_slot(frame, area, block);
        return;
    };

    state.ui.pinned_on_replay[pane_index] = view.on_replay;
    state.ui.pinned_text_selections[pane_index] = view.selection;
    state.ui.pinned_content_lengths[pane_index] = view.content_len;

    let paragraph = Paragraph::new(view.lines)
        .block(block)
        .scroll((view.scroll_offset as u16, 0));

    frame.render_widget(paragraph, area);

    if view.scrollbar_content_len > viewport_height {
        let scrollbar_max = view.scrollbar_content_len.saturating_sub(viewport_height);
        let scrollbar_sfb =
            (state.ui.pinned_scroll_offsets[pane_index] as usize).min(scrollbar_max);
        let scrollbar_pos = scrollbar_max.saturating_sub(scrollbar_sfb);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        let mut scrollbar_state = ScrollbarState::new(scrollbar_max).position(scrollbar_pos);
        frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
    }

    if is_focused && state.ui.input_mode == InputMode::Normal && view.scroll_from_bottom == 0 {
        let needs_terminal_cursor = state
            .pinned_terminal_session_at(pane_index)
            .map(|s| s.agent_type.is_terminal() || s.agent_type.is_redraw_style())
            .unwrap_or(false);

        if needs_terminal_cursor {
            render_cursor(frame, inner_area, view.cursor, view.scroll_offset, true);
        }
    }
}

fn render_empty_slot(frame: &mut Frame, area: Rect, block: Block) {
    let t = crate::theme::current();
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  No terminal in this slot",
            Style::default().fg(t.fg_dim),
        )),
    ];

    frame.render_widget(Paragraph::new(lines).block(block), area);
}
