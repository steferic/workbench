use crate::app::{AppState, ConfigItem, FocusPanel, UtilityItem, UtilitySection};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let is_focused = state.focus == FocusPanel::UtilitiesPane;
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // Create outer block
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    // Split inner area: tabs row + content
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner_area);

    let tabs_area = chunks[0];
    let content_area = chunks[1];

    // Render horizontal tabs
    let utils_active = state.utility_section == UtilitySection::Utilities;
    let config_active = state.utility_section == UtilitySection::GlobalConfig;

    let utils_style = if utils_active && is_focused {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else if utils_active {
        Style::default()
            .fg(Color::Black)
            .bg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let config_style = if config_active && is_focused {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else if config_active {
        Style::default()
            .fg(Color::Black)
            .bg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let tabs = Paragraph::new(Line::from(vec![
        Span::styled(" Util ", utils_style),
        Span::raw(" "),
        Span::styled(" Config ", config_style),
    ]));
    frame.render_widget(tabs, tabs_area);

    // Render content based on active section
    let items: Vec<ListItem> = match state.utility_section {
        UtilitySection::Utilities => {
            UtilityItem::all()
                .iter()
                .map(|item| {
                    let is_selected = *item == state.selected_utility;

                    let style = if is_selected && is_focused {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else if is_selected {
                        Style::default().fg(Color::White)
                    } else {
                        Style::default().fg(Color::Gray)
                    };

                    let prefix = if is_selected { "> " } else { "  " };

                    ListItem::new(Line::from(vec![
                        Span::styled(prefix, style),
                        Span::raw(format!("{} ", item.icon())),
                        Span::styled(item.name(), style),
                    ]))
                })
                .collect()
        }
        UtilitySection::GlobalConfig => {
            ConfigItem::all()
                .iter()
                .map(|item| {
                    let is_selected = *item == state.selected_config;

                    let style = if is_selected && is_focused {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else if is_selected {
                        Style::default().fg(Color::White)
                    } else {
                        Style::default().fg(Color::Gray)
                    };

                    let prefix = if is_selected { "> " } else { "  " };

                    // Show toggle state
                    let toggle_indicator = match item {
                        ConfigItem::ToggleBanner => {
                            if state.banner_visible {
                                Span::styled(" [ON]", Style::default().fg(Color::Green))
                            } else {
                                Span::styled(" [OFF]", Style::default().fg(Color::Red))
                            }
                        }
                    };

                    ListItem::new(Line::from(vec![
                        Span::styled(prefix, style),
                        Span::raw(format!("{} ", item.icon())),
                        Span::styled(item.name(), style),
                        toggle_indicator,
                    ]))
                })
                .collect()
        }
    };

    let list = List::new(items)
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    frame.render_widget(list, content_area);
}
