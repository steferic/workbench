use crate::app::{AppState, FocusPanel};
use crate::models::{Session, SessionStatus};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let is_focused = state.ui.focus == FocusPanel::SessionList;
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" Sessions ")
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    // Split inner area: list + action bars (2 rows)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(inner_area);

    let list_area = chunks[0];
    let action_area = chunks[1];

    let sessions = state.sessions_for_selected_workspace();
    let pinned_ids = state.pinned_terminal_ids();

    // Separate sessions into agents and terminals
    let agent_indices: Vec<usize> = sessions
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.agent_type.is_terminal())
        .map(|(i, _)| i)
        .collect();

    let terminal_indices: Vec<usize> = sessions
        .iter()
        .enumerate()
        .filter(|(_, s)| s.agent_type.is_terminal())
        .map(|(i, _)| i)
        .collect();

    let mut items: Vec<ListItem> = Vec::new();
    let mut selected_visual_idx: Option<usize> = None;
    let mut current_visual_idx: usize = 0;

    // Agents section header
    if !agent_indices.is_empty() {
        let header_style = Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD);
        items.push(ListItem::new(Line::from(vec![
            Span::styled("── Agents ──", header_style),
        ])));
        current_visual_idx += 1;
    }

    // Agent sessions
    for &session_idx in &agent_indices {
        let session = &sessions[session_idx];
        let item = create_session_item(state, session_idx, session, is_focused, &pinned_ids);
        items.push(item);
        if session_idx == state.ui.selected_session_idx {
            selected_visual_idx = Some(current_visual_idx);
        }
        current_visual_idx += 1;
    }

    // Terminals section header
    if !terminal_indices.is_empty() {
        let header_style = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
        items.push(ListItem::new(Line::from(vec![
            Span::styled("── Terminals ──", header_style),
        ])));
        current_visual_idx += 1;
    }

    // Terminal sessions
    for &session_idx in &terminal_indices {
        let session = &sessions[session_idx];
        let item = create_session_item(state, session_idx, session, is_focused, &pinned_ids);
        items.push(item);
        if session_idx == state.ui.selected_session_idx {
            selected_visual_idx = Some(current_visual_idx);
        }
        current_visual_idx += 1;
    }

    let list = List::new(items)
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    let mut list_state = ListState::default();
    list_state.select(selected_visual_idx);

    frame.render_stateful_widget(list, list_area, &mut list_state);

    // Render action bars (2 rows)
    let action_style = if is_focused {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::Rgb(60, 60, 60))
    };
    let key_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let action_bar = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("c", key_style),
            Span::styled(":set terminal start cmd ", action_style),
            Span::styled("s", key_style),
            Span::styled(":stop ", action_style),
            Span::styled("d", key_style),
            Span::styled(":del", action_style),
        ]),
        Line::from(vec![
            Span::styled("1", key_style),
            Span::styled(":Claude ", action_style),
            Span::styled("2", key_style),
            Span::styled(":Gemini ", action_style),
            Span::styled("3", key_style),
            Span::styled(":Codex ", action_style),
            Span::styled("4", key_style),
            Span::styled(":Grok ", action_style),
            Span::styled("t", key_style),
            Span::styled(":term ", action_style),
            Span::styled("p", key_style),
            Span::styled(":pin", action_style),
        ]),
    ]);

    frame.render_widget(action_bar, action_area);
}

fn create_session_item<'a>(
    state: &AppState,
    session_idx: usize,
    session: &Session,
    is_focused: bool,
    pinned_ids: &[uuid::Uuid],
) -> ListItem<'a> {
    let is_selected = session_idx == state.ui.selected_session_idx && is_focused;
    let is_active = state.ui.active_session_id == Some(session.id);
    let is_working = state.is_session_working(session.id);
    let is_pinned = pinned_ids.contains(&session.id);

    let status_color = match session.status {
        SessionStatus::Running => {
            if is_working {
                Color::Yellow
            } else {
                Color::Green
            }
        }
        SessionStatus::Stopped => Color::Gray,
        SessionStatus::Errored => Color::Red,
    };

    let activity_indicator = match session.status {
        SessionStatus::Running => {
            if session.agent_type.is_terminal() {
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
    } else if session_idx == state.ui.selected_session_idx {
        "> "
    } else {
        "  "
    };

    let main_line = Line::from(vec![
        Span::styled(prefix.to_string(), name_style),
        Span::styled(session.status_icon(), Style::default().fg(status_color)),
        Span::raw(" "),
        Span::styled(
            format!("[{}] ", session.agent_type.icon()),
            Style::default().fg(Color::Magenta),
        ),
        Span::styled(session.agent_type.display_name().to_string(), name_style),
        Span::styled(
            format!(" ({})", session.short_id()),
            Style::default().fg(Color::DarkGray),
        ),
        activity_indicator,
        pin_indicator,
    ]);

    if let Some(ref cmd) = session.start_command {
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
}
