use crate::app::AppState;
use crate::tui::utils::{
    convert_vt100_to_lines, get_content_length, get_cursor_info, get_selection_bounds,
};
use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

/// Debug overlay showing terminal/pane dimensions at each layer
/// Toggle with F11
pub fn render(frame: &mut Frame, state: &AppState) {
    let area = frame.area();

    // Create a small overlay in the top-right corner
    let width = 50;
    let height = 26;
    let x = area.width.saturating_sub(width + 2);
    let y = 2;

    let overlay_area = Rect::new(x, y, width.min(area.width), height.min(area.height));

    let t = crate::theme::current();

    // Clear background
    frame.render_widget(Clear, overlay_area);

    let mut lines = vec![
        Line::from(Span::styled(
            "Debug: Terminal Dimensions",
            Style::default()
                .fg(t.active)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    // Terminal size (raw)
    let (term_w, term_h) = state.system.terminal_size;
    lines.push(Line::from(vec![
        Span::styled("Terminal Size: ", Style::default().fg(t.accent)),
        Span::raw(format!("{}x{}", term_w, term_h)),
    ]));

    // Calculated pane dimensions
    let pane_rows = state.pane_rows();
    let output_cols = state.output_pane_cols();
    let pinned_cols = state.pinned_pane_cols();
    lines.push(Line::from(vec![
        Span::styled("Pane Rows: ", Style::default().fg(t.accent)),
        Span::raw(format!("{}", pane_rows)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Output Cols: ", Style::default().fg(t.accent)),
        Span::raw(format!("{}", output_cols)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Pinned Cols: ", Style::default().fg(t.accent)),
        Span::raw(format!("{}", pinned_cols)),
    ]));

    lines.push(Line::from(""));

    // Actual rendered areas
    if let Some((x, y, w, h)) = state.ui.output_pane_area {
        lines.push(Line::from(vec![
            Span::styled("Output Pane Area: ", Style::default().fg(t.success)),
            Span::raw(format!("{}x{} at ({},{})", w, h, x, y)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Inner (computed): ", Style::default().fg(t.fg_faint)),
            Span::raw(format!("{}x{}", w.saturating_sub(2), h.saturating_sub(2))),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("Output Pane Area: ", Style::default().fg(t.error)),
            Span::raw("Not rendered yet"),
        ]));
    }

    lines.push(Line::from(""));

    // Active session's vt100 parser size
    if let Some(parser) = state
        .ui
        .active_session_id
        .and_then(|id| state.system.output_buffers.get(&id))
    {
        let screen = parser.screen();
        let (rows, cols) = screen.size();
        lines.push(Line::from(vec![
            Span::styled("vt100 Parser Size: ", Style::default().fg(t.special)),
            Span::raw(format!("{}x{}", cols, rows)),
        ]));

        // Check for column mismatch (rows being larger is intentional for scrollback)
        let expected_cols = output_cols;
        if cols != expected_cols {
            lines.push(Line::from(vec![
                Span::styled(
                    "  ⚠ COLS MISMATCH! ",
                    Style::default().fg(t.error).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("Expected {} cols", expected_cols),
                    Style::default().fg(t.error),
                ),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled("  ✓ ", Style::default().fg(t.success)),
                Span::raw(format!("Cols match, rows={} (buffer)", rows)),
            ]));
        }

        // Cursor position
        let cursor_info = get_cursor_info(screen);
        lines.push(Line::from(vec![
            Span::styled("Cursor Position: ", Style::default().fg(t.fg_faint)),
            Span::raw(format!("row={}, col={}", cursor_info.row, cursor_info.col)),
        ]));

        // Alternate screen status
        let is_alternate = screen.alternate_screen();
        lines.push(Line::from(vec![
            Span::styled("Alternate Screen: ", Style::default().fg(t.fg_faint)),
            Span::raw(if is_alternate { "Yes" } else { "No" }),
        ]));

        // Count actual rendered lines
        let content_len = get_content_length(screen, cursor_info.row);
        let selection =
            get_selection_bounds(&state.ui.text_selection, content_len, screen.size().1);
        let rendered_lines = convert_vt100_to_lines(screen, selection, cursor_info.row);
        lines.push(Line::from(vec![
            Span::styled("Rendered Lines: ", Style::default().fg(t.fg_faint)),
            Span::raw(format!("{}", rendered_lines.len())),
        ]));

        // Count non-empty lines
        let non_empty = rendered_lines
            .iter()
            .filter(|l| !l.spans.is_empty())
            .count();
        lines.push(Line::from(vec![
            Span::styled("Non-empty Lines: ", Style::default().fg(t.fg_faint)),
            Span::raw(format!("{}", non_empty)),
        ]));

        // Check if cursor is hidden
        lines.push(Line::from(vec![
            Span::styled("Cursor Hidden: ", Style::default().fg(t.fg_faint)),
            Span::raw(if cursor_info.hidden { "Yes" } else { "No" }),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("vt100 Parser: ", Style::default().fg(t.error)),
            Span::raw("No active session"),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("Banner: ", Style::default().fg(t.fg_faint)),
        Span::raw(if state.ui.banner_visible {
            "Visible"
        } else {
            "Hidden"
        }),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Split View: ", Style::default().fg(t.fg_faint)),
        Span::raw(if state.should_show_split() {
            "Active"
        } else {
            "Disabled"
        }),
    ]));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press F11 to close",
        Style::default().fg(t.fg_faint),
    )));

    let block = Block::default()
        .title(" Debug Info ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.active))
        .style(Style::default().bg(t.bg));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .alignment(Alignment::Left);

    frame.render_widget(paragraph, overlay_area);
}
