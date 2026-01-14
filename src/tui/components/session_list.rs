use crate::app::{AppState, FocusPanel};
use crate::git;
use crate::models::{Session, SessionStatus};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use uuid::Uuid;

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

    // Split inner area: list + action bar (1 row)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner_area);

    let list_area = chunks[0];
    let action_area = chunks[1];

    let sessions = state.sessions_for_selected_workspace();
    let pinned_ids = state.pinned_terminal_ids();

    // Get parallel task info for this workspace
    let parallel_session_ids: Vec<Uuid> = state.selected_workspace()
        .map(|ws| {
            ws.parallel_tasks.iter()
                .flat_map(|t| t.attempts.iter().map(|a| a.session_id))
                .collect()
        })
        .unwrap_or_default();

    // Separate sessions into agents (non-parallel), parallel, and terminals
    let agent_indices: Vec<usize> = sessions
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.agent_type.is_terminal() && !parallel_session_ids.contains(&s.id))
        .map(|(i, _)| i)
        .collect();

    let parallel_indices: Vec<usize> = sessions
        .iter()
        .enumerate()
        .filter(|(_, s)| parallel_session_ids.contains(&s.id))
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

    // Branch indicator at the top - show active worktree branch if one is selected
    if let Some(workspace) = state.selected_workspace() {
        // Check if there's an active worktree session
        let (branch_name, is_worktree) = if let Some(worktree_session_id) = workspace.active_worktree_session_id {
            // Get the branch from the session's worktree
            state.data.sessions.values()
                .flatten()
                .find(|s| s.id == worktree_session_id)
                .and_then(|s| s.worktree_branch.clone())
                .map(|b| (b, true))
                .unwrap_or_else(|| {
                    // Fallback to main branch if session not found
                    let main = git::get_current_branch(&workspace.path)
                        .unwrap_or_else(|_| "unknown".to_string());
                    (main, false)
                })
        } else {
            // No worktree active - show main branch
            let main = git::get_current_branch(&workspace.path)
                .unwrap_or_else(|_| "unknown".to_string());
            (main, false)
        };

        let branch_style = if is_worktree {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        };
        let icon_style = if is_worktree {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::Green)
        };

        let hint = if is_worktree {
            Span::styled(" (w:back)", Style::default().fg(Color::DarkGray))
        } else {
            Span::raw("")
        };

        items.push(ListItem::new(Line::from(vec![
            Span::styled("⎇ ", icon_style),
            Span::styled(branch_name, branch_style),
            hint,
        ])));
        current_visual_idx += 1;
    }

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
        let item = create_session_item(state, session_idx, session, is_focused, &pinned_ids, false);
        items.push(item);
        if session_idx == state.ui.selected_session_idx {
            selected_visual_idx = Some(current_visual_idx);
        }
        current_visual_idx += 1;
    }

    // Parallel task section
    if !parallel_indices.is_empty() {
        // Get task prompt preview
        let task_preview = state.selected_workspace()
            .and_then(|ws| ws.active_parallel_task())
            .map(|t| {
                let preview: String = t.prompt.chars().take(30).collect();
                if t.prompt.len() > 30 {
                    format!("{}...", preview)
                } else {
                    preview
                }
            })
            .unwrap_or_else(|| "Parallel Task".to_string());

        let header_style = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
        items.push(ListItem::new(Line::from(vec![
            Span::styled(format!("── {} ──", task_preview), header_style),
        ])));
        current_visual_idx += 1;

        // Parallel sessions
        for &session_idx in &parallel_indices {
            let session = &sessions[session_idx];
            let item = create_session_item(state, session_idx, session, is_focused, &pinned_ids, true);
            items.push(item);
            if session_idx == state.ui.selected_session_idx {
                selected_visual_idx = Some(current_visual_idx);
            }
            current_visual_idx += 1;
        }
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
        let item = create_session_item(state, session_idx, session, is_focused, &pinned_ids, false);
        items.push(item);
        if session_idx == state.ui.selected_session_idx {
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

    let action_bar = Paragraph::new(Line::from(vec![
        Span::styled("h", key_style),
        Span::styled(":help", action_style),
    ]));

    frame.render_widget(action_bar, action_area);
}

fn create_session_item<'a>(
    state: &AppState,
    session_idx: usize,
    session: &Session,
    is_focused: bool,
    pinned_ids: &[Uuid],
    is_parallel: bool,
) -> ListItem<'a> {
    let is_selected = session_idx == state.ui.selected_session_idx && is_focused;
    let is_active = state.ui.active_session_id == Some(session.id);
    let is_working = state.is_session_working(session.id);
    let is_pinned = pinned_ids.contains(&session.id);
    let is_worktree_active = state.selected_workspace()
        .and_then(|ws| ws.active_worktree_session_id)
        .map(|id| id == session.id)
        .unwrap_or(false);

    // Status icon: spinner when working, diamond when idle, circle for stopped/errored
    let (status_icon, status_color) = match session.status {
        SessionStatus::Running => {
            if session.agent_type.is_terminal() {
                ("◆", Color::Green)
            } else if is_working {
                (state.spinner_char(), Color::Yellow)
            } else {
                ("◆", Color::DarkGray)
            }
        }
        SessionStatus::Stopped => ("○", Color::Gray),
        SessionStatus::Errored => ("✗", Color::Red),
    };

    let pin_indicator = if is_pinned {
        Span::styled(" [pinned]", Style::default().fg(Color::Magenta))
    } else {
        Span::raw("")
    };

    // Show worktree indicator with short ID for parallel or regular worktree sessions
    let branch_indicator = if is_parallel {
        // Get the branch name from the parallel task attempt and extract just the ID
        let branch_name = state.selected_workspace()
            .and_then(|ws| {
                ws.parallel_tasks.iter()
                    .flat_map(|t| t.attempts.iter())
                    .find(|a| a.session_id == session.id)
                    .map(|a| a.branch_name.clone())
            })
            .unwrap_or_default();

        if !branch_name.is_empty() {
            // Extract just the ID part (last segment after final '-')
            let short_id = branch_name.rsplit('-').next().unwrap_or(&branch_name);
            let style = if is_worktree_active {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Cyan)
            };
            let suffix = if is_worktree_active { " ◀" } else { "" };
            Span::styled(
                format!(" ⎇ {}{}", short_id, suffix),
                style,
            )
        } else {
            Span::raw("")
        }
    } else if let Some(branch) = &session.worktree_branch {
        // Regular session with worktree - extract just the ID part
        let short_id = branch.rsplit('-').next().unwrap_or(branch);
        let style = if is_worktree_active {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Cyan)
        };
        let suffix = if is_worktree_active { " ◀" } else { "" };
        Span::styled(
            format!(" ⎇ {}{}", short_id, suffix),
            style,
        )
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
        Span::styled(status_icon, Style::default().fg(status_color)),
        Span::raw(" "),
        Span::styled(session.agent_type.display_name().to_string(), name_style),
        branch_indicator,
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
