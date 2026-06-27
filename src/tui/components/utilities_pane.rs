use crate::app::{AppState, FocusPanel, UtilityItem, UtilitySection};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let t = crate::theme::current();
    let is_focused = state.ui.focus == FocusPanel::UtilitiesPane;
    let border_style = if is_focused {
        Style::default().fg(t.border_focused)
    } else {
        Style::default().fg(t.border)
    };

    // Create outer block
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    // Split inner area: tabs row + content + action bar (1 row)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner_area);

    let tabs_area = chunks[0];
    let content_area = chunks[1];
    let action_area = chunks[2];

    // Render horizontal tabs
    let utils_active = state.ui.utility_section == UtilitySection::Utilities;
    let sounds_active = state.ui.utility_section == UtilitySection::Sounds;
    let config_active = state.ui.utility_section == UtilitySection::GlobalConfig;
    let notepad_active = state.ui.utility_section == UtilitySection::Notepad;

    let tab_style = |active: bool| {
        if active && is_focused {
            Style::default()
                .fg(t.on_accent)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD)
        } else if active {
            Style::default().fg(t.on_accent).bg(t.fg)
        } else {
            Style::default().fg(t.fg_faint)
        }
    };

    let tabs = Paragraph::new(Line::from(vec![
        Span::styled(" Util ", tab_style(utils_active)),
        Span::styled("|", Style::default().fg(t.fg_faint)),
        Span::styled(" Sounds ", tab_style(sounds_active)),
        Span::styled("|", Style::default().fg(t.fg_faint)),
        Span::styled(" Cfg ", tab_style(config_active)),
        Span::styled("|", Style::default().fg(t.fg_faint)),
        Span::styled(" Notes ", tab_style(notepad_active)),
    ]));
    frame.render_widget(tabs, tabs_area);

    // Render content based on active section
    match state.ui.utility_section {
        UtilitySection::Utilities => {
            render_utilities_list(frame, content_area, state, is_focused);
        }
        UtilitySection::Sounds => {
            render_sounds_list(frame, content_area, state, is_focused);
        }
        UtilitySection::GlobalConfig => {
            render_config_list(frame, content_area, state, is_focused);
        }
        UtilitySection::Notepad => {
            render_notepad(frame, content_area, state, is_focused);
        }
    }

    // Render action bar (1 row, compact)
    let action_style = if is_focused {
        Style::default().fg(t.fg_faint)
    } else {
        Style::default().fg(t.inactive)
    };
    let key_style = if is_focused {
        Style::default().fg(t.accent)
    } else {
        Style::default().fg(t.fg_faint)
    };

    let action_bar = Paragraph::new(Line::from(vec![
        Span::styled("h", key_style),
        Span::styled(":help", action_style),
    ]));

    frame.render_widget(action_bar, action_area);
}

fn render_utilities_list(frame: &mut Frame, area: Rect, state: &AppState, is_focused: bool) {
    let t = crate::theme::current();
    let tools = UtilityItem::tools();

    let items: Vec<ListItem> = tools
        .iter()
        .map(|item| {
            let is_selected = *item == state.ui.selected_utility;

            let style = if is_selected && is_focused {
                Style::default()
                    .fg(t.accent)
                    .add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default().fg(t.fg)
            } else {
                Style::default().fg(t.fg_dim)
            };

            let prefix = if is_selected { "> " } else { "  " };

            // Show ON/OFF indicator for toggles
            let toggle_indicator = match item {
                UtilityItem::ToggleBanner => {
                    if state.ui.banner_visible {
                        Span::styled(" [ON]", Style::default().fg(t.success))
                    } else {
                        Span::styled(" [OFF]", Style::default().fg(t.error))
                    }
                }
                UtilityItem::AgentDoneSound => {
                    if state.system.agent_done_sound_enabled {
                        Span::styled(" [ON]", Style::default().fg(t.success))
                    } else {
                        Span::styled(" [OFF]", Style::default().fg(t.error))
                    }
                }
                UtilityItem::ToggleTheme => Span::styled(
                    format!(" [{}]", state.ui.theme_mode.label()),
                    Style::default().fg(t.accent),
                ),
                _ => Span::raw(""),
            };

            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::raw(format!("{} ", item.icon())),
                Span::styled(item.name(), style),
                toggle_indicator,
            ]))
        })
        .collect();

    // Highlight style with full row background when focused
    let highlight_style = if is_focused {
        Style::default()
            .bg(t.selection_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    let list = List::new(items).highlight_style(highlight_style);

    let mut list_state = ListState::default();
    let selected_idx = tools.iter().position(|i| *i == state.ui.selected_utility);
    list_state.select(selected_idx);

    frame.render_stateful_widget(list, area, &mut list_state);
}

fn render_sounds_list(frame: &mut Frame, area: Rect, state: &AppState, is_focused: bool) {
    let t = crate::theme::current();
    let sounds = UtilityItem::sounds();

    let items: Vec<ListItem> = sounds
        .iter()
        .map(|item| {
            let is_selected = *item == state.ui.selected_sound;

            let style = if is_selected && is_focused {
                Style::default()
                    .fg(t.accent)
                    .add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default().fg(t.fg)
            } else {
                Style::default().fg(t.fg_dim)
            };

            let prefix = if is_selected { "> " } else { "  " };

            // Show ON/OFF indicator for sounds
            let toggle_indicator = match item {
                UtilityItem::BrownNoise => {
                    if state.system.brown_noise_playing {
                        Span::styled(" [ON]", Style::default().fg(t.success))
                    } else {
                        Span::styled(" [OFF]", Style::default().fg(t.error))
                    }
                }
                UtilityItem::ClassicalRadio => {
                    if state.system.classical_radio_playing {
                        Span::styled(" [ON]", Style::default().fg(t.success))
                    } else {
                        Span::styled(" [OFF]", Style::default().fg(t.error))
                    }
                }
                UtilityItem::OceanWaves => {
                    if state.system.ocean_waves_playing {
                        Span::styled(" [ON]", Style::default().fg(t.success))
                    } else {
                        Span::styled(" [OFF]", Style::default().fg(t.error))
                    }
                }
                UtilityItem::WindChimes => {
                    if state.system.wind_chimes_playing {
                        Span::styled(" [ON]", Style::default().fg(t.success))
                    } else {
                        Span::styled(" [OFF]", Style::default().fg(t.error))
                    }
                }
                UtilityItem::RainforestRain => {
                    if state.system.rainforest_rain_playing {
                        Span::styled(" [ON]", Style::default().fg(t.success))
                    } else {
                        Span::styled(" [OFF]", Style::default().fg(t.error))
                    }
                }
                _ => Span::raw(""),
            };

            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::raw(format!("{} ", item.icon())),
                Span::styled(item.name(), style),
                toggle_indicator,
            ]))
        })
        .collect();

    // Highlight style with full row background when focused
    let highlight_style = if is_focused {
        Style::default()
            .bg(t.selection_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    let list = List::new(items).highlight_style(highlight_style);

    let mut list_state = ListState::default();
    let selected_idx = sounds.iter().position(|i| *i == state.ui.selected_sound);
    list_state.select(selected_idx);

    frame.render_stateful_widget(list, area, &mut list_state);
}

fn render_config_list(frame: &mut Frame, area: Rect, state: &AppState, is_focused: bool) {
    let t = crate::theme::current();
    // Render simple config directory list
    if state.ui.config_tree_nodes.is_empty() {
        let placeholder = Paragraph::new("No config directories found")
            .style(Style::default().fg(t.fg_faint));
        frame.render_widget(placeholder, area);
        return;
    }

    let items: Vec<ListItem> = state
        .ui
        .config_tree_nodes
        .iter()
        .enumerate()
        .map(|(idx, node)| {
            let is_selected = idx == state.ui.config_tree_selected;

            let style = if is_selected && is_focused {
                Style::default()
                    .fg(t.accent)
                    .add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default().fg(t.fg)
            } else {
                Style::default().fg(t.fg_dim)
            };

            let prefix = if is_selected { "> " } else { "  " };
            let icon = node.icon();
            let name = node.name();

            // Show hint to open terminal
            let hint = Span::styled(
                " [Enter: open terminal]",
                Style::default().fg(t.fg_faint),
            );

            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::raw(format!("{} ", icon)),
                Span::styled(name, style),
                hint,
            ]))
        })
        .collect();

    // Highlight style with full row background when focused
    let highlight_style = if is_focused {
        Style::default()
            .bg(t.selection_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    let list = List::new(items).highlight_style(highlight_style);

    let mut list_state = ListState::default();
    list_state.select(Some(state.ui.config_tree_selected));

    frame.render_stateful_widget(list, area, &mut list_state);
}

fn render_notepad(frame: &mut Frame, area: Rect, state: &mut AppState, is_focused: bool) {
    let t = crate::theme::current();
    // Get or create the TextArea for current workspace
    if let Some(textarea) = state.current_notepad() {
        // Style the textarea based on focus
        let cursor_style = if is_focused {
            Style::default().fg(t.on_accent).bg(t.accent)
        } else {
            Style::default().fg(t.fg_faint).bg(t.fg_faint)
        };

        let cursor_line_style = if is_focused {
            Style::default().bg(t.selection_bg)
        } else {
            Style::default()
        };

        // Line number style - dimmer when not focused
        let line_number_style = if is_focused {
            Style::default().fg(t.fg_faint)
        } else {
            Style::default().fg(t.inactive)
        };

        textarea.set_cursor_style(cursor_style);
        textarea.set_cursor_line_style(cursor_line_style);
        textarea.set_line_number_style(line_number_style);

        // Render the widget
        frame.render_widget(&*textarea, area);
    } else {
        // No workspace selected - show placeholder
        let placeholder = Paragraph::new("Select a workspace to use notepad")
            .style(Style::default().fg(t.fg_faint));
        frame.render_widget(placeholder, area);
    }
}
