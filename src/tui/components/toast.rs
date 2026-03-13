use crate::app::{AppState, ToastLevel};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
    Frame,
};

const MAX_TOAST_WIDTH: u16 = 52;

pub fn render(frame: &mut Frame, state: &AppState) {
    if state.ui.toasts.is_empty() {
        return;
    }

    // Position: top-right corner of the center (output) pane
    let (pane_x, pane_y, pane_w, _pane_h) = match state.ui.output_pane_area {
        Some(area) => area,
        None => return,
    };
    let pane_right = pane_x + pane_w;
    let start_y = pane_y + 1; // +1 to clear the border

    for (i, toast) in state.ui.toasts.iter().enumerate() {
        let (icon, color) = match toast.level {
            ToastLevel::Info => (" i ", Color::Cyan),
            ToastLevel::Success => (" \u{2713} ", Color::Green),
            ToastLevel::Warning => (" ! ", Color::Yellow),
            ToastLevel::Error => (" x ", Color::Red),
        };

        // Truncate message to fit
        let max_msg_len = (MAX_TOAST_WIDTH as usize).saturating_sub(5); // icon(3) + spaces(2)
        let msg = if toast.message.len() > max_msg_len {
            format!("{}...", &toast.message[..max_msg_len.saturating_sub(3)])
        } else {
            toast.message.clone()
        };

        // Actual display width: icon is always 3 columns wide, msg is ASCII
        let content_width = (3 + 1 + msg.len() + 1) as u16;
        let toast_x = pane_right.saturating_sub(content_width + 1); // +1 for border

        let toast_rect = Rect {
            x: toast_x,
            y: start_y + i as u16,
            width: content_width,
            height: 1,
        };

        let line = Line::from(vec![
            Span::styled(
                icon,
                Style::default()
                    .fg(Color::Black)
                    .bg(color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {} ", msg),
                Style::default()
                    .fg(Color::White)
                    .bg(Color::DarkGray),
            ),
        ]);

        frame.render_widget(Clear, toast_rect);
        frame.render_widget(Paragraph::new(line), toast_rect);
    }
}
