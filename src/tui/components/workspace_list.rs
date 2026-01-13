use crate::app::{AppState, FocusPanel};
use crate::models::WorkspaceStatus;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let is_focused = state.ui.focus == FocusPanel::WorkspaceList;
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let title = format!(" Workspaces ({}) ", state.data.workspaces.len());

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    // Split inner area: list + action bar (1 row)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner_area);

    let list_area = chunks[0];
    let action_area = chunks[1];

    // Separate workspaces into working and paused
    let working_indices: Vec<usize> = state.data.workspaces.iter()
        .enumerate()
        .filter(|(_, ws)| ws.status == WorkspaceStatus::Working)
        .map(|(i, _)| i)
        .collect();

    let paused_indices: Vec<usize> = state.data.workspaces.iter()
        .enumerate()
        .filter(|(_, ws)| ws.status == WorkspaceStatus::Paused)
        .map(|(i, _)| i)
        .collect();

    let mut items: Vec<ListItem> = Vec::new();
    let mut selected_visual_idx: Option<usize> = None;
    let mut current_visual_idx: usize = 0;

    // Working section header
    if !working_indices.is_empty() || paused_indices.is_empty() {
        let header_style = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
        items.push(ListItem::new(Line::from(vec![
            Span::styled("── Working ──", header_style),
        ])));
        current_visual_idx += 1;
    }

    // Working workspaces
    for &ws_idx in &working_indices {
        let ws = &state.data.workspaces[ws_idx];
        items.push(create_workspace_item(state, ws_idx, ws, is_focused, false));
        if ws_idx == state.ui.selected_workspace_idx {
            selected_visual_idx = Some(current_visual_idx);
        }
        current_visual_idx += 1;
    }

    // Paused section header
    if !paused_indices.is_empty() {
        let header_style = Style::default().fg(Color::Rgb(255, 165, 0)).add_modifier(Modifier::BOLD);
        items.push(ListItem::new(Line::from(vec![
            Span::styled("── Paused ──", header_style),
        ])));
        current_visual_idx += 1;
    }

    // Paused workspaces (dimmed)
    for &ws_idx in &paused_indices {
        let ws = &state.data.workspaces[ws_idx];
        items.push(create_workspace_item(state, ws_idx, ws, is_focused, true));
        if ws_idx == state.ui.selected_workspace_idx {
            selected_visual_idx = Some(current_visual_idx);
        }
        current_visual_idx += 1;
    }

    // Highlight style with full row background when focused
    let highlight_style = if is_focused {
        Style::default()
            .bg(Color::Rgb(40, 50, 60))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    let list = List::new(items)
        .highlight_style(highlight_style);

    // Use ListState for automatic scrolling
    let mut list_state = ListState::default();
    list_state.select(selected_visual_idx);

    frame.render_stateful_widget(list, list_area, &mut list_state);

    // Render action bar (1 row, inside the border)
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

    let action_bar = Paragraph::new(Line::from(vec![
        Span::styled("n", key_style),
        Span::styled(":new ", action_style),
        Span::styled("w", key_style),
        Span::styled(":work ", action_style),
        Span::styled("d", key_style),
        Span::styled(":del", action_style),
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
    let is_working = state.is_workspace_working(ws.id);

    let name = ws.name.clone();
    let sessions_info = if total > 0 {
        format!(" ({}/{})", running, total)
    } else {
        String::new()
    };

    // Last active timestamp
    let last_active = ws.last_active_display();
    let time_info = format!(" {}", last_active);

    let is_selected = ws_idx == state.ui.selected_workspace_idx;

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

    let info_style = Style::default().fg(Color::DarkGray);

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

    // Working indicator (spinner) when any agent in workspace is working
    let working_indicator = if is_working && !is_paused {
        Span::styled(
            format!("{} ", state.spinner_char()),
            Style::default().fg(Color::Yellow),
        )
    } else {
        Span::raw("")
    };

    ListItem::new(Line::from(vec![
        Span::styled(prefix.to_string(), style),
        working_indicator,
        Span::styled(name, style),
        Span::styled(sessions_info, info_style),
        Span::styled(time_info, time_style),
    ]))
}
