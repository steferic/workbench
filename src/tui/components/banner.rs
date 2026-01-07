use crate::app::AppState;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let width = area.width as usize;
    if width == 0 {
        return;
    }

    // Get the banner text and create a scrolling view
    let text = &state.banner_text;
    let text_chars: Vec<char> = text.chars().collect();
    let text_len = text_chars.len();

    if text_len == 0 {
        return;
    }

    // Create the visible portion by rotating the text
    let mut visible: String = String::with_capacity(width);
    for i in 0..width {
        let char_idx = (state.banner_offset + i) % text_len;
        visible.push(text_chars[char_idx]);
    }

    // Create gradient effect with colors cycling through
    let colors = [
        Color::Magenta,
        Color::LightMagenta,
        Color::Cyan,
        Color::LightCyan,
        Color::Blue,
        Color::LightBlue,
    ];

    let mut spans = Vec::new();
    for (i, ch) in visible.chars().enumerate() {
        // Cycle through colors based on position + animation offset for movement effect
        let color_idx = (i + state.banner_offset / 2) % colors.len();
        let style = Style::default()
            .fg(colors[color_idx])
            .add_modifier(Modifier::BOLD);
        spans.push(Span::styled(ch.to_string(), style));
    }

    let paragraph = Paragraph::new(Line::from(spans))
        .style(Style::default().bg(Color::Black));

    frame.render_widget(paragraph, area);
}
