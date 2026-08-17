use crate::app::{AppState, FocusPanel, UtilityItem, UtilitySection};
use crate::theme::ThemeMode;
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
    let themes_active = state.ui.utility_section == UtilitySection::Themes;
    let sounds_active = state.ui.utility_section == UtilitySection::Sounds;
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
        Span::styled(" Themes ", tab_style(themes_active)),
        Span::styled("|", Style::default().fg(t.fg_faint)),
        Span::styled(" Sounds ", tab_style(sounds_active)),
        Span::styled("|", Style::default().fg(t.fg_faint)),
        Span::styled(" Notes ", tab_style(notepad_active)),
    ]));
    frame.render_widget(tabs, tabs_area);

    // Render content based on active section
    match state.ui.utility_section {
        UtilitySection::Utilities => {
            render_utilities_list(frame, content_area, state, is_focused);
        }
        UtilitySection::Themes => {
            render_themes_list(frame, content_area, state, is_focused);
        }
        UtilitySection::Sounds => {
            render_sounds_list(frame, content_area, state, is_focused);
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

    let action_bar = if state.ui.utility_section == UtilitySection::Themes {
        Paragraph::new(Line::from(vec![
            Span::styled("enter", key_style),
            Span::styled(":apply  ", action_style),
            Span::styled("tab", key_style),
            Span::styled(":next", action_style),
        ]))
    } else {
        Paragraph::new(Line::from(vec![
            Span::styled("h", key_style),
            Span::styled(":help", action_style),
        ]))
    };

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
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default().fg(t.fg)
            } else {
                Style::default().fg(t.fg_dim)
            };

            let prefix = if is_selected { "> " } else { "  " };

            let toggle_indicator = match item {
                UtilityItem::ToggleBanner => {
                    if state.ui.banner_visible {
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
    let selected_idx = tools.iter().position(|i| *i == state.ui.selected_utility);
    list_state.select(selected_idx);

    frame.render_stateful_widget(list, area, &mut list_state);
}

fn render_themes_list(frame: &mut Frame, area: Rect, state: &AppState, is_focused: bool) {
    let t = crate::theme::current();

    let items: Vec<ListItem> = ThemeMode::ALL
        .iter()
        .map(|theme| {
            let is_selected = *theme == state.ui.selected_theme;
            let is_active = *theme == state.ui.theme_mode;
            let style = if is_selected && is_focused {
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
            } else if is_selected || is_active {
                Style::default().fg(t.fg)
            } else {
                Style::default().fg(t.fg_dim)
            };
            let kind = if theme.is_dark() { "DARK" } else { "LIGHT" };
            let kind_color = if theme.is_dark() { t.info } else { t.warning };

            ListItem::new(Line::from(vec![
                Span::styled(if is_selected { "> " } else { "  " }, style),
                Span::styled(
                    if is_active { "● " } else { "  " },
                    Style::default().fg(t.accent),
                ),
                Span::styled(format!("{:<11}", theme.label()), style),
                Span::styled(
                    format!("[{kind}]"),
                    Style::default().fg(kind_color).add_modifier(Modifier::BOLD),
                ),
            ]))
        })
        .collect();

    let highlight_style = if is_focused {
        Style::default()
            .bg(t.selection_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let list = List::new(items).highlight_style(highlight_style);
    let mut list_state = ListState::default();
    list_state.select(
        ThemeMode::ALL
            .iter()
            .position(|theme| *theme == state.ui.selected_theme),
    );

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
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn screen(mut state: AppState, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &mut state))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn themes_tab_lists_every_theme_and_labels_its_ground() {
        let mut state = AppState::default();
        state.ui.focus = FocusPanel::UtilitiesPane;
        state.ui.utility_section = UtilitySection::Themes;
        state.ui.theme_mode = ThemeMode::Botan;
        state.ui.selected_theme = ThemeMode::Kinu;

        let out = screen(state, 38, 20);

        assert!(out.contains(" Util | Themes | Sounds | Notes "), "{out}");
        for theme in ThemeMode::ALL {
            let row = out
                .lines()
                .find(|line| line.contains(theme.label()))
                .unwrap_or_else(|| panic!("{} is missing from:\n{out}", theme.label()));
            let kind = if theme.is_dark() { "[DARK]" } else { "[LIGHT]" };
            assert!(
                row.contains(kind),
                "{} is not labelled in: {row}",
                theme.label()
            );
        }

        let active = out.lines().find(|line| line.contains("Botan")).unwrap();
        assert!(active.contains('●'), "active theme is not marked: {active}");
        let selected = out.lines().find(|line| line.contains("Kinu")).unwrap();
        assert!(
            selected.contains('>'),
            "selected theme is not marked: {selected}"
        );
        assert!(out.contains("enter:apply  tab:next"), "{out}");
    }

    #[test]
    fn prompt_log_is_an_item_in_the_existing_util_tab() {
        let mut state = AppState::default();
        state.ui.focus = FocusPanel::UtilitiesPane;
        state.ui.selected_utility = UtilityItem::PromptLog;

        let out = screen(state, 38, 14);

        assert!(out.contains(" Util | Themes | Sounds | Notes "), "{out}");
        assert!(out.contains("> ✎ Prompt Log"), "{out}");
    }

    #[test]
    fn banner_utility_shows_its_current_state() {
        let mut visible = AppState::default();
        visible.ui.focus = FocusPanel::UtilitiesPane;
        visible.ui.selected_utility = UtilityItem::ToggleBanner;
        let out = screen(visible, 38, 10);
        assert!(out.contains("Banner Bar [ON]"), "{out}");

        let mut hidden = AppState::default();
        hidden.ui.focus = FocusPanel::UtilitiesPane;
        hidden.ui.selected_utility = UtilityItem::ToggleBanner;
        hidden.ui.banner_visible = false;
        let out = screen(hidden, 38, 10);
        assert!(out.contains("Banner Bar [OFF]"), "{out}");
    }
}
