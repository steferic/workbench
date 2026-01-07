use crate::app::{AppState, FocusPanel};
use crate::models::SessionStatus;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let is_focused = state.focus == FocusPanel::SessionList;
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // Split area: list + model buttons + actions
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(4)])
        .split(area);

    let list_area = chunks[0];
    let buttons_area = chunks[1];

    let sessions = state.sessions_for_selected_workspace();
    let session_count = sessions.len();

    let pinned_ids = state.pinned_terminal_ids();

    let items: Vec<ListItem> = sessions
        .iter()
        .enumerate()
        .map(|(i, session)| {
            let is_selected = i == state.selected_session_idx && is_focused;
            let is_active = state.active_session_id == Some(session.id);
            let is_working = state.is_session_working(session.id);
            let is_pinned = pinned_ids.contains(&session.id);

            let status_color = match session.status {
                SessionStatus::Running => {
                    if is_working {
                        Color::Yellow // Working/busy
                    } else {
                        Color::Green // Idle
                    }
                }
                SessionStatus::Stopped => Color::Gray,
                SessionStatus::Errored => Color::Red,
            };

            // Activity indicator for running sessions (only for agents, not terminals)
            let activity_indicator = match session.status {
                SessionStatus::Running => {
                    if session.agent_type.is_terminal() {
                        // Terminals don't show idle/working - they're always "running"
                        Span::styled(" ◆ running", Style::default().fg(Color::Green))
                    } else if is_working {
                        Span::styled(
                            format!(" {} working", state.spinner_char()),
                            Style::default().fg(Color::Yellow),
                        )
                    } else {
                        Span::styled(" ◆ idle", Style::default().fg(Color::DarkGray))
                    }
                }
                _ => Span::raw(""),
            };

            // Pin indicator
            let pin_indicator = if is_pinned {
                Span::styled(" [pinned]", Style::default().fg(Color::Magenta))
            } else {
                Span::raw("")
            };

            let name_style = if is_selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else if is_active {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let prefix = if is_active {
                "* "
            } else if i == state.selected_session_idx {
                "> "
            } else {
                "  "
            };

            // Build main line
            let main_line = Line::from(vec![
                Span::styled(prefix, name_style),
                Span::styled(session.status_icon(), Style::default().fg(status_color)),
                Span::raw(" "),
                Span::styled(
                    format!("[{}] ", session.agent_type.icon()),
                    Style::default().fg(Color::Magenta),
                ),
                Span::styled(session.agent_type.display_name(), name_style),
                Span::styled(
                    format!(" ({})", session.short_id()),
                    Style::default().fg(Color::DarkGray),
                ),
                activity_indicator,
                pin_indicator,
            ]);

            // If terminal has a start command, show it on second line
            if let Some(ref cmd) = session.start_command {
                // Truncate command if too long
                let max_len = 40;
                let display_cmd = if cmd.len() > max_len {
                    format!("{}...", &cmd[..max_len])
                } else {
                    cmd.clone()
                };
                let cmd_line = Line::from(vec![
                    Span::raw("      "),
                    Span::styled("$ ", Style::default().fg(Color::DarkGray)),
                    Span::styled(display_cmd, Style::default().fg(Color::Rgb(255, 165, 0))),
                ]);
                ListItem::new(Text::from(vec![main_line, cmd_line]))
            } else {
                ListItem::new(main_line)
            }
        })
        .collect();

    let workspace_name = state
        .selected_workspace()
        .map(|w| w.name.as_str())
        .unwrap_or("None");

    let title = format!(" Sessions - {} ({}) ", workspace_name, session_count);

    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(border_style),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    let mut list_state = ListState::default();
    if !sessions.is_empty() {
        list_state.select(Some(state.selected_session_idx));
    }

    frame.render_stateful_widget(list, list_area, &mut list_state);

    // Render model buttons
    let has_workspace = state.selected_workspace().is_some();

    let key_style = if is_focused && has_workspace {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let label_style = if is_focused && has_workspace {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let has_session = !sessions.is_empty();
    let action_key_style = if is_focused && has_session {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let action_label_style = if is_focused && has_session {
        Style::default().fg(Color::Gray)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let buttons = Paragraph::new(vec![
        Line::from(Span::styled(
            " + New Session:",
            if is_focused && has_workspace {
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        )),
        Line::from(vec![
            Span::styled(" [1]", key_style),
            Span::styled(" Claude ", label_style),
            Span::styled("[2]", key_style),
            Span::styled(" Gemini ", label_style),
            Span::styled("[3]", key_style),
            Span::styled(" Codex ", label_style),
            Span::styled("[4]", key_style),
            Span::styled(" Grok ", label_style),
            Span::styled("[t]", key_style),
            Span::styled(" Terminal", label_style),
        ]),
        Line::from(vec![
            Span::styled(" [r/Enter]", action_key_style),
            Span::styled(" Restart ", action_label_style),
            Span::styled("[d]", action_key_style),
            Span::styled(" Delete ", action_label_style),
            Span::styled("[p]", action_key_style),
            Span::styled(" Pin ", action_label_style),
            Span::styled("[\\]", action_key_style),
            Span::styled(" Split", action_label_style),
        ]),
    ]);

    frame.render_widget(buttons, buttons_area);
}
