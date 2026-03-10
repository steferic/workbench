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

    let area = frame.area();
    let toast_count = state.ui.toasts.len().min(5) as u16;

    // Position: bottom-right, above status bar (1 line for status bar)
    let start_y = area.height.saturating_sub(1 + toast_count);
    let toast_width = MAX_TOAST_WIDTH.min(area.width.saturating_sub(2));
    let start_x = area.width.saturating_sub(toast_width + 1);

    for (i, toast) in state.ui.toasts.iter().enumerate() {
        let toast_rect = Rect {
            x: start_x,
            y: start_y + i as u16,
            width: toast_width,
            height: 1,
        };

        let (icon, color) = match toast.level {
            ToastLevel::Info => (" i ", Color::Cyan),
            ToastLevel::Success => (" \u{2713} ", Color::Green),
            ToastLevel::Warning => (" ! ", Color::Yellow),
            ToastLevel::Error => (" x ", Color::Red),
        };

        // Truncate message to fit
        let max_msg_len = (toast_width as usize).saturating_sub(5); // icon(3) + spaces(2)
        let msg = if toast.message.len() > max_msg_len {
            format!("{}...", &toast.message[..max_msg_len.saturating_sub(3)])
        } else {
            toast.message.clone()
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
