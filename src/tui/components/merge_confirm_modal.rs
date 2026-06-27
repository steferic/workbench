use crate::app::AppState;
use crate::git;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, state: &AppState) {
    let t = crate::theme::current();
    let area = centered_rect(50, 25, frame.area());

    // Clear the background
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Merge Worktree ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.warning))
        .style(Style::default().bg(t.bg));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Get merge info
    let (branch_name, main_branch) = if let Some(session_id) = state.ui.merging_session_id {
        let session_info = state
            .data
            .sessions
            .values()
            .flatten()
            .find(|s| s.id == session_id);

        if let Some(session) = session_info {
            let branch = session
                .worktree_branch
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let workspace_path = state
                .data
                .workspaces
                .iter()
                .find(|w| w.id == session.workspace_id)
                .map(|w| w.path.clone());

            let main = workspace_path
                .and_then(|p| git::get_current_branch(&p).ok())
                .unwrap_or_else(|| "main".to_string());

            (branch, main)
        } else {
            ("unknown".to_string(), "main".to_string())
        }
    } else {
        ("unknown".to_string(), "main".to_string())
    };

    // Split into: message, branch info, help
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Warning message
            Constraint::Length(3), // Branch info
            Constraint::Length(2), // Help
        ])
        .split(inner);

    let message_area = chunks[0];
    let branch_area = chunks[1];
    let help_area = chunks[2];

    // Warning message
    let message = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("⚠ ", Style::default().fg(t.warning)),
            Span::styled(
                "Worktree has uncommitted changes",
                Style::default().fg(t.fg),
            ),
        ]),
        Line::from(vec![Span::styled(
            "  Commit all changes and merge?",
            Style::default().fg(t.fg_dim),
        )]),
    ]);
    frame.render_widget(message, message_area);

    // Branch info
    let branch_info = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("  From: ", Style::default().fg(t.fg_dim)),
            Span::styled(
                &branch_name,
                Style::default()
                    .fg(t.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Into: ", Style::default().fg(t.fg_dim)),
            Span::styled(
                &main_branch,
                Style::default()
                    .fg(t.success)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ]);
    frame.render_widget(branch_info, branch_area);

    // Help
    let help = Paragraph::new(Line::from(vec![
        Span::styled(
            "[Y/Enter]",
            Style::default()
                .fg(t.success)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Commit & Merge  "),
        Span::styled("[N/Esc]", Style::default().fg(t.error)),
        Span::raw(" Cancel"),
    ]));
    frame.render_widget(help, help_area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
