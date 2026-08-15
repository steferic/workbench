use crate::app::{AppState, FocusPanel};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = crate::theme::current();
    let is_focused = state.ui.focus == FocusPanel::WorkspaceList;
    let border_style = if is_focused {
        Style::default().fg(t.border_focused)
    } else {
        Style::default().fg(t.border)
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

    let items: Vec<ListItem> = state
        .data
        .workspaces
        .iter()
        .enumerate()
        .map(|(ws_idx, workspace)| {
            create_workspace_item(state, ws_idx, workspace, is_focused)
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

    // Use ListState for automatic scrolling
    let mut list_state = ListState::default();
    if !state.data.workspaces.is_empty() {
        list_state.select(Some(state.ui.selected_workspace_idx));
    }

    frame.render_stateful_widget(list, list_area, &mut list_state);

    // Render action bar (1 row, inside the border)
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
        Span::styled(":settings", action_style),
    ]));

    frame.render_widget(action_bar, action_area);
}

fn create_workspace_item<'a>(
    state: &AppState,
    ws_idx: usize,
    ws: &crate::models::Workspace,
    is_focused: bool,
) -> ListItem<'a> {
    let t = crate::theme::current();
    let is_working = state.is_workspace_working(ws.id);

    let name = ws.name.clone();

    // Last active timestamp
    let last_active = ws.last_active_display();
    let time_info = format!(" {}", last_active);

    let is_selected = ws_idx == state.ui.selected_workspace_idx;

    // Different styling for selected/focused rows.
    let style = if is_selected && is_focused {
        Style::default()
            .fg(t.accent)
            .add_modifier(Modifier::BOLD)
    } else if is_selected {
        Style::default().fg(t.fg)
    } else {
        Style::default().fg(t.fg_dim)
    };

    // Time style - slightly dimmer, different color for recency
    let time_style = if last_active == "just now" || last_active.ends_with("m ago") {
        Style::default().fg(t.success)
    } else if last_active.ends_with("h ago") {
        Style::default().fg(t.active)
    } else {
        Style::default().fg(t.fg_faint)
    };

    let prefix = if is_selected { "> " } else { "  " };

    // Check if workspace is loading (has pending sessions in startup queue)
    let is_loading = state.is_workspace_loading(ws.id);

    // An agent in this project stopped for the user. It outranks the working
    // spinner: this is the project you should switch to, and it is easy to
    // miss when you are looking at a different one.
    let waiting_here = state
        .sessions_needing_attention()
        .iter()
        .any(|(session_id, _)| state.workspace_id_for_session(*session_id) == Some(ws.id));

    // Working/Loading indicator (spinner) - fixed width so name doesn't shift
    // Blue = loading (sessions starting up), Yellow = working (actively processing)
    let working_indicator = if waiting_here {
        Span::styled("! ", Style::default().fg(t.warning).add_modifier(Modifier::BOLD))
    } else if is_loading {
        Span::styled(
            format!("{} ", state.spinner_char()),
            Style::default().fg(t.info),
        )
    } else if is_working {
        Span::styled(
            format!("{} ", state.spinner_char()),
            Style::default().fg(t.active),
        )
    } else {
        Span::raw("  ")
    };

    ListItem::new(Line::from(vec![
        Span::styled(prefix.to_string(), style),
        working_indicator,
        Span::styled(name, style),
        Span::styled(time_info, time_style),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Workspace;
    use ratatui::{backend::TestBackend, Terminal};

    fn screen(state: &AppState, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), state))
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
    fn every_workspace_renders_in_one_flat_list() {
        let mut state = AppState::default();
        state.data.workspaces = vec![
            Workspace::new("alpha".into(), "/tmp/alpha".into()),
            Workspace::new("beta".into(), "/tmp/beta".into()),
        ];

        let out = screen(&state, 40, 7);

        assert!(out.contains("alpha"), "{out}");
        assert!(out.contains("beta"), "{out}");
        assert!(!out.contains("Working"), "{out}");
        assert!(!out.contains("Paused"), "{out}");
    }
}
