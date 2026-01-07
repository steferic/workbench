use crate::app::{AppState, FocusPanel};
use crate::models::WorkspaceStatus;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let is_focused = state.focus == FocusPanel::WorkspaceList;
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // Split area: list + action bar
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(area);

    let list_area = chunks[0];
    let action_area = chunks[1];

    // Separate workspaces into working and paused
    let working_indices: Vec<usize> = state.workspaces.iter()
        .enumerate()
        .filter(|(_, ws)| ws.status == WorkspaceStatus::Working)
        .map(|(i, _)| i)
        .collect();

    let paused_indices: Vec<usize> = state.workspaces.iter()
        .enumerate()
        .filter(|(_, ws)| ws.status == WorkspaceStatus::Paused)
        .map(|(i, _)| i)
        .collect();

    let mut items: Vec<ListItem> = Vec::new();

    // Working section header
    if !working_indices.is_empty() || paused_indices.is_empty() {
        let header_style = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
        items.push(ListItem::new(Line::from(vec![
            Span::styled("── Working ──", header_style),
        ])));
    }

    // Working workspaces
    for &ws_idx in &working_indices {
        let ws = &state.workspaces[ws_idx];
        items.push(create_workspace_item(state, ws_idx, ws, is_focused, false));
    }

    // Paused section header
    if !paused_indices.is_empty() {
        let header_style = Style::default().fg(Color::Rgb(255, 165, 0)).add_modifier(Modifier::BOLD);
        items.push(ListItem::new(Line::from(vec![
            Span::styled("── Paused ──", header_style),
        ])));
    }

    // Paused workspaces (dimmed)
    for &ws_idx in &paused_indices {
        let ws = &state.workspaces[ws_idx];
        items.push(create_workspace_item(state, ws_idx, ws, is_focused, true));
    }

    let title = format!(" Workspaces ({}) ", state.workspaces.len());

    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(border_style),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    frame.render_widget(list, list_area);

    // Render action bar
    let action_style = if is_focused {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let key_style = if is_focused {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let action_bar = Paragraph::new(Line::from(vec![
        Span::styled(" [n]", key_style),
        Span::styled(" New  ", action_style),
        Span::styled("[w]", key_style),
        Span::styled(" Toggle", action_style),
    ]));

    frame.render_widget(action_bar, action_area);
}

fn create_workspace_item<'a>(
    state: &AppState,
    ws_idx: usize,
    ws: &crate::models::Workspace,
    is_focused: bool,
    is_paused: bool,
) -> ListItem<'a> {
    let running = state.workspace_running_count(ws.id);
    let total = state.workspace_session_count(ws.id);

    let name = ws.name.clone();
    let sessions_info = if total > 0 {
        format!(" ({}/{})", running, total)
    } else {
        String::new()
    };

    // Last active timestamp
    let last_active = ws.last_active_display();
    let time_info = format!(" {}", last_active);

    let is_selected = ws_idx == state.selected_workspace_idx;

    // Different styling for selected/focused vs paused
    let style = if is_selected && is_focused {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else if is_selected {
        Style::default().fg(Color::White)
    } else if is_paused {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::Gray)
    };

    let info_style = if is_paused {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // Time style - slightly dimmer, different color for recency
    let time_style = if is_paused {
        Style::default().fg(Color::DarkGray)
    } else if last_active == "just now" || last_active.ends_with("m ago") {
        Style::default().fg(Color::Green)
    } else if last_active.ends_with("h ago") {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let prefix = if is_selected { "> " } else { "  " };

    ListItem::new(Line::from(vec![
        Span::styled(prefix.to_string(), style),
        Span::styled(name, style),
        Span::styled(sessions_info, info_style),
        Span::styled(time_info, time_style),
    ]))
}
