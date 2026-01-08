use crate::app::AppState;
use chrono::Local;
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

    let teal_style = Style::default()
        .fg(Color::Cyan)
        .bg(Color::Black)
        .add_modifier(Modifier::BOLD);

    let dim_teal_style = Style::default()
        .fg(Color::DarkGray)
        .bg(Color::Black);

    // Get active workspace name
    let workspace_name = state
        .data.workspaces
        .get(state.ui.selected_workspace_idx)
        .map(|w| w.name.clone())
        .unwrap_or_else(|| "No Workspace".to_string());

    // Format: " workspace_name "
    let left_text = format!(" {} ", workspace_name);
    let left_len = left_text.chars().count();

    // Get current date/time
    let now = Local::now();
    let right_text = now.format(" %b %d %H:%M ").to_string();
    let right_len = right_text.chars().count();

    // Calculate middle section width
    let separator_len = 3; // " | " on each side
    let fixed_width = left_len + right_len + (separator_len * 2);
    let middle_width = width.saturating_sub(fixed_width);

    // Build the scrolling middle section
    let middle_content = if middle_width > 0 {
        let text = &state.ui.banner_text;
        let text_chars: Vec<char> = text.chars().collect();
        let text_len = text_chars.len();

        if text_len == 0 {
            " ".repeat(middle_width)
        } else {
            let mut visible = String::with_capacity(middle_width);
            for i in 0..middle_width {
                let char_idx = (state.ui.banner_offset + i) % text_len;
                visible.push(text_chars[char_idx]);
            }
            visible
        }
    } else {
        String::new()
    };

    // Build spans
    let spans = vec![
        Span::styled(left_text, teal_style),
        Span::styled(" | ", dim_teal_style),
        Span::styled(middle_content, teal_style),
        Span::styled(" | ", dim_teal_style),
        Span::styled(right_text, teal_style),
    ];

    let paragraph = Paragraph::new(Line::from(spans))
        .style(Style::default().bg(Color::Black));

    frame.render_widget(paragraph, area);
}
