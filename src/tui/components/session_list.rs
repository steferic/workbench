use crate::agent_status::Activity;
use crate::app::{AppState, FocusPanel};
use crate::git;
use crate::models::{Session, SessionStatus};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use uuid::Uuid;

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = crate::theme::current();
    let is_focused = state.ui.focus == FocusPanel::SessionList;
    let border_style = if is_focused {
        Style::default().fg(t.border_focused)
    } else {
        Style::default().fg(t.border)
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
    let parallel_session_ids: Vec<Uuid> = state
        .selected_workspace()
        .map(|ws| {
            ws.parallel_tasks
                .iter()
                .flat_map(|t| t.attempts.iter().map(|a| a.session_id))
                .collect()
        })
        .unwrap_or_default();

    // Categorize sessions in a single pass
    let mut agent_indices: Vec<usize> = Vec::new();
    let mut parallel_indices: Vec<usize> = Vec::new();
    let mut terminal_indices: Vec<usize> = Vec::new();

    for (i, s) in sessions.iter().enumerate() {
        if s.agent_type.is_terminal() {
            terminal_indices.push(i);
        } else if parallel_session_ids.contains(&s.id) {
            parallel_indices.push(i);
        } else {
            agent_indices.push(i);
        }
    }

    let mut items: Vec<ListItem> = Vec::new();
    let mut selected_visual_idx: Option<usize> = None;
    let mut current_visual_idx: usize = 0;

    // Branch indicator at the top - show active worktree branch if one is selected
    if let Some(workspace) = state.selected_workspace() {
        // Check if there's an active worktree session
        let (branch_name, is_worktree) =
            if let Some(worktree_session_id) = workspace.active_worktree_session_id {
                // Get the branch from the session's worktree
                state
                    .data
                    .sessions
                    .values()
                    .flatten()
                    .find(|s| s.id == worktree_session_id)
                    .and_then(|s| s.worktree_branch.clone())
                    .map(|b| (b, true))
                    .unwrap_or_else(|| {
                        // Fallback to main branch if session not found
                        // Use fast file-based read instead of spawning git subprocess
                        let main = git::get_current_branch_fast(&workspace.path)
                            .unwrap_or_else(|| "unknown".to_string());
                        (main, false)
                    })
            } else {
                // No worktree active - show main branch
                // Use fast file-based read instead of spawning git subprocess
                let main = git::get_current_branch_fast(&workspace.path)
                    .unwrap_or_else(|| "unknown".to_string());
                (main, false)
            };

        let branch_style = if is_worktree {
            Style::default()
                .fg(t.active)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(t.accent)
                .add_modifier(Modifier::BOLD)
        };
        let icon_style = if is_worktree {
            Style::default().fg(t.active)
        } else {
            Style::default().fg(t.success)
        };

        let hint = if is_worktree {
            Span::styled(" (w:back)", Style::default().fg(t.fg_faint))
        } else {
            Span::raw("")
        };

        // Look up diff stats for the workspace
        let mut branch_spans = vec![
            Span::styled("⎇ ", icon_style),
            Span::styled(branch_name, branch_style),
        ];

        if let Some(stat) = state.system.diff_stats.get(&workspace.path) {
            if stat.insertions > 0 || stat.deletions > 0 {
                branch_spans.push(Span::raw(" "));
                if stat.insertions > 0 {
                    branch_spans.push(Span::styled(
                        format!("+{}", stat.insertions),
                        Style::default().fg(t.success),
                    ));
                }
                if stat.deletions > 0 {
                    if stat.insertions > 0 {
                        branch_spans.push(Span::raw(" "));
                    }
                    branch_spans.push(Span::styled(
                        format!("-{}", stat.deletions),
                        Style::default().fg(t.error),
                    ));
                }
            }
        }

        branch_spans.push(hint);

        items.push(ListItem::new(Line::from(branch_spans)));
        current_visual_idx += 1;
    }

    // Agents section header
    if !agent_indices.is_empty() {
        let header_style = Style::default()
            .fg(t.special)
            .add_modifier(Modifier::BOLD);
        items.push(ListItem::new(Line::from(vec![Span::styled(
            "── Agents ──",
            header_style,
        )])));
        current_visual_idx += 1;
    }

    // Agent sessions
    for &session_idx in &agent_indices {
        let session = &sessions[session_idx];
        let item = create_session_item(state, session_idx, session, is_focused, pinned_ids, false);
        items.push(item);
        if session_idx == state.selected_session_idx() {
            selected_visual_idx = Some(current_visual_idx);
        }
        current_visual_idx += 1;
    }

    // Parallel task section
    if !parallel_indices.is_empty() {
        // Get task prompt preview
        let task_preview = state
            .selected_workspace()
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

        let header_style = Style::default()
            .fg(t.active)
            .add_modifier(Modifier::BOLD);
        items.push(ListItem::new(Line::from(vec![Span::styled(
            format!("── {} ──", task_preview),
            header_style,
        )])));
        current_visual_idx += 1;

        // Parallel sessions
        for &session_idx in &parallel_indices {
            let session = &sessions[session_idx];
            let item =
                create_session_item(state, session_idx, session, is_focused, pinned_ids, true);
            items.push(item);
            if session_idx == state.selected_session_idx() {
                selected_visual_idx = Some(current_visual_idx);
            }
            current_visual_idx += 1;
        }
    }

    // Terminals section header
    if !terminal_indices.is_empty() {
        let header_style = Style::default()
            .fg(t.success)
            .add_modifier(Modifier::BOLD);
        items.push(ListItem::new(Line::from(vec![Span::styled(
            "── Terminals ──",
            header_style,
        )])));
        current_visual_idx += 1;
    }

    // Terminal sessions
    for &session_idx in &terminal_indices {
        let session = &sessions[session_idx];
        let item = create_session_item(state, session_idx, session, is_focused, pinned_ids, false);
        items.push(item);
        if session_idx == state.selected_session_idx() {
            selected_visual_idx = Some(current_visual_idx);
        }
        current_visual_idx += 1;
    }

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
    list_state.select(selected_visual_idx);

    frame.render_stateful_widget(list, list_area, &mut list_state);

    // Render action bars (2 rows)
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

fn create_session_item<'a>(
    state: &AppState,
    session_idx: usize,
    session: &Session,
    is_focused: bool,
    pinned_ids: &[Uuid],
    is_parallel: bool,
) -> ListItem<'a> {
    let t = crate::theme::current();
    let is_selected = session_idx == state.selected_session_idx() && is_focused;
    let is_active = state.active_session_id() == Some(session.id);
    let activity = state.activity(session.id);
    let is_pinned = pinned_ids.contains(&session.id);
    let is_worktree_active = state
        .selected_workspace()
        .and_then(|ws| ws.active_worktree_session_id)
        .map(|id| id == session.id)
        .unwrap_or(false);

    // Status icon: an agent that stopped *for you* outranks every other
    // state — it is the only one that costs you time by going unnoticed.
    let (status_icon, status_color) = match session.status {
        SessionStatus::Running => {
            if session.agent_type.is_terminal() {
                ("◆", t.success)
            } else {
                match activity {
                    Activity::NeedsAttention(_) => ("!", t.warning),
                    Activity::Working => (state.spinner_char(), t.active),
                    Activity::Idle | Activity::Exited => ("◆", t.fg_faint),
                }
            }
        }
        SessionStatus::Stopped => ("○", t.fg_dim),
        SessionStatus::Errored => ("✗", t.error),
    };

    // Pinned terminals are flagged by the diamond color rather than a label.
    let status_color = if is_pinned {
        t.special
    } else {
        status_color
    };

    // Dangerous mode indicator (skip permissions)
    let dangerous_indicator =
        if session.dangerously_skip_permissions && session.agent_type.is_agent() {
            Span::styled(" ⚡", Style::default().fg(t.danger))
        } else {
            Span::raw("")
        };

    // Show worktree indicator with short ID for parallel or regular worktree sessions
    let branch_indicator = if is_parallel {
        // Get the branch name from the parallel task attempt and extract just the ID
        let branch_name = state
            .selected_workspace()
            .and_then(|ws| {
                ws.parallel_tasks
                    .iter()
                    .flat_map(|t| t.attempts.iter())
                    .find(|a| a.session_id == session.id)
                    .map(|a| a.branch_name.clone())
            })
            .unwrap_or_default();

        if !branch_name.is_empty() {
            // Extract just the ID part (last segment after final '-')
            let short_id = branch_name.rsplit('-').next().unwrap_or(&branch_name);
            let style = if is_worktree_active {
                Style::default()
                    .fg(t.active)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.accent)
            };
            let suffix = if is_worktree_active { " ◀" } else { "" };
            Span::styled(format!(" ⎇ {}{}", short_id, suffix), style)
        } else {
            Span::raw("")
        }
    } else if let Some(branch) = &session.worktree_branch {
        // Regular session with worktree - extract just the ID part
        let short_id = branch.rsplit('-').next().unwrap_or(branch);
        let style = if is_worktree_active {
            Style::default()
                .fg(t.active)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.accent)
        };
        let suffix = if is_worktree_active { " ◀" } else { "" };
        Span::styled(format!(" ⎇ {}{}", short_id, suffix), style)
    } else {
        Span::raw("")
    };

    let name_style = if is_selected {
        Style::default()
            .fg(t.accent)
            .add_modifier(Modifier::BOLD)
    } else if is_active {
        Style::default()
            .fg(t.active)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.fg)
    };

    let prefix = if is_active {
        "* "
    } else if session_idx == state.selected_session_idx() {
        "> "
    } else {
        "  "
    };

    let alias_indicator = match session.alias.as_deref() {
        Some(alias) => Span::styled(
            format!(" “{alias}”"),
            Style::default().fg(t.accent),
        ),
        None => Span::raw(""),
    };

    // What it is stopped for, in the agent's own words ("needs approval").
    // Only for the blocked state: a label on every row would be noise.
    let attention_indicator = match activity.needs_attention() {
        Some(kind) => Span::styled(
            format!(" {}", kind.label()),
            Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
        ),
        None => Span::raw(""),
    };

    let main_spans = vec![
        Span::styled(prefix.to_string(), name_style),
        Span::styled(status_icon, Style::default().fg(status_color)),
        Span::raw(" "),
        // The model rather than the agent it runs in — "Opus 5", not
        // "Claude". Falls back to the agent's name until a turn has been
        // journalled, which is the only moment anything knows the model.
        Span::styled(state.session_label(session.id), name_style),
        alias_indicator,
        attention_indicator,
        dangerous_indicator,
        branch_indicator,
    ];

    let main_line = Line::from(main_spans);

    if let Some(ref cmd) = session.start_command {
        let max_len = 40;
        let display_cmd = if cmd.len() > max_len {
            format!("{}...", &cmd[..max_len])
        } else {
            cmd.clone()
        };
        let cmd_line = Line::from(vec![
            Span::raw("      "),
            Span::styled("$ ", Style::default().fg(t.fg_faint)),
            Span::styled(display_cmd, Style::default().fg(t.command)),
        ]);
        ListItem::new(Text::from(vec![main_line, cmd_line]))
    } else {
        ListItem::new(main_line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_status::{AgentStatus, Attention};
    use crate::models::{AgentType, Workspace};
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

    fn state_with_blocked_agent(kind: Attention, reason: &str) -> AppState {
        let mut state = AppState::default();
        let workspace = Workspace::new("w".into(), std::path::PathBuf::from("/tmp/w"));
        let workspace_id = workspace.id;
        let session = Session::new(workspace_id, AgentType::Claude, false);
        let session_id = session.id;
        state.data.workspaces.push(workspace);
        state.data.sessions.insert(workspace_id, vec![session]);
        state.system.agent_status.insert(
            session_id,
            AgentStatus {
                activity: crate::agent_status::Activity::NeedsAttention(kind),
                reason: reason.into(),
                at: chrono::Utc::now(),
                event: "Notification".into(),
            },
        );
        state
    }

    #[test]
    fn a_blocked_agent_says_so_on_its_row() {
        let state = state_with_blocked_agent(
            Attention::Permission,
            "Claude needs your permission to use Bash",
        );
        let out = screen(&state, 44, 10);

        assert!(out.contains("needs approval"), "{out}");
        // The marker replaces the idle diamond, so a glance at the column is
        // enough — this is the row you have to act on.
        assert!(out.contains("! Claude"), "{out}");
        assert!(!out.contains("◆ Claude"), "{out}");
    }

    #[test]
    fn an_unblocked_agent_keeps_the_ordinary_row() {
        let mut state = state_with_blocked_agent(Attention::Input, "waiting");
        state.system.agent_status.clear();
        let out = screen(&state, 44, 10);

        assert!(!out.contains('!'), "{out}");
        assert!(out.contains("◆ Claude"), "{out}");
    }
}
