use crate::app::AppState;
use crate::git;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, state: &AppState) {
    let area = centered_rect(60, 50, frame.area());

    // Clear the background
    frame.render_widget(Clear, area);

    let workspace = state.selected_workspace();
    let workspace_name = workspace.map(|w| w.name.as_str()).unwrap_or("Unknown");
    let workspace_path = workspace.map(|w| w.path.as_path());

    // Get git info
    let (branch_name, commit_short) = workspace_path
        .and_then(|p| {
            let branch = git::get_current_branch(p).ok()?;
            let commit = git::get_head_commit(p).ok()?;
            Some((branch, commit.chars().take(7).collect::<String>()))
        })
        .unwrap_or_else(|| ("unknown".to_string(), "???????".to_string()));

    let is_clean = workspace_path
        .map(|p| git::is_clean(p).unwrap_or(false))
        .unwrap_or(false);

    let mut content = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  Workspace: {}", workspace_name),
            Style::default().fg(Color::Gray),
        )),
        Line::from(vec![
            Span::styled("  Branch: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{} ({})", branch_name, commit_short),
                Style::default().fg(Color::Cyan),
            ),
        ]),
    ];

    // Warn if working directory is not clean
    if !is_clean {
        content.push(Line::from(Span::styled(
            "  ⚠ Working directory has uncommitted changes!",
            Style::default().fg(Color::Yellow),
        )));
    }

    content.push(Line::from(""));
    content.push(Line::from(Span::styled(
        "  Task prompt:",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));

    // Show the input buffer with cursor
    let prompt_display = if state.ui.parallel_task_prompt.is_empty() {
        "  │ _".to_string()
    } else {
        format!("  │ {}_", state.ui.parallel_task_prompt)
    };
    content.push(Line::from(Span::styled(
        prompt_display,
        Style::default().fg(Color::White),
    )));

    content.push(Line::from(""));
    content.push(Line::from(Span::styled(
        "  Agents to use:",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));

    // Render agent selection
    let agent_count = state.ui.parallel_task_agents.len();
    for (idx, (agent_type, selected)) in state.ui.parallel_task_agents.iter().enumerate() {
        let is_focused = idx == state.ui.parallel_task_agent_idx;
        let checkbox = if *selected { "[x]" } else { "[ ]" };
        let agent_name = agent_type.display_name();
        let agent_badge = agent_type.badge();

        let line = if is_focused {
            Line::from(vec![
                Span::styled("  > ", Style::default().fg(Color::Yellow)),
                Span::styled(
                    checkbox,
                    Style::default()
                        .fg(if *selected { Color::Green } else { Color::Gray }),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("[{}] ", agent_badge),
                    Style::default().fg(Color::Magenta),
                ),
                Span::styled(
                    agent_name,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ])
        } else {
            Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    checkbox,
                    Style::default()
                        .fg(if *selected { Color::Green } else { Color::DarkGray }),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("[{}] ", agent_badge),
                    Style::default().fg(Color::Magenta),
                ),
                Span::raw(agent_name),
            ])
        };
        content.push(line);
    }

    content.push(Line::from(""));

    // Report checkbox - uses special index (agent_count) when focused
    let report_focused = state.ui.parallel_task_agent_idx == agent_count;
    let report_checkbox = if state.ui.parallel_task_request_report { "[x]" } else { "[ ]" };
    let report_line = if report_focused {
        Line::from(vec![
            Span::styled("  > ", Style::default().fg(Color::Yellow)),
            Span::styled(
                report_checkbox,
                Style::default()
                    .fg(if state.ui.parallel_task_request_report { Color::Green } else { Color::Gray }),
            ),
            Span::styled(
                " Request report (PARALLEL_REPORT.md)",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ])
    } else {
        Line::from(vec![
            Span::raw("    "),
            Span::styled(
                report_checkbox,
                Style::default()
                    .fg(if state.ui.parallel_task_request_report { Color::Green } else { Color::DarkGray }),
            ),
            Span::raw(" Request report (PARALLEL_REPORT.md)"),
        ])
    };
    content.push(report_line);

    content.push(Line::from(""));
    content.push(Line::from("  ─────────────────────────────────────────"));
    content.push(Line::from(vec![
        Span::styled("  Enter", Style::default().fg(Color::Cyan)),
        Span::raw(": Start   "),
        Span::styled("Tab", Style::default().fg(Color::Cyan)),
        Span::raw(": Next   "),
        Span::styled("x", Style::default().fg(Color::Cyan)),
        Span::raw(": Toggle   "),
        Span::styled("Esc", Style::default().fg(Color::Cyan)),
        Span::raw(": Cancel"),
    ]));

    let block = Block::default()
        .title(" Start Parallel Task ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta))
        .style(Style::default().bg(Color::Black));

    let paragraph = Paragraph::new(content).block(block);

    frame.render_widget(paragraph, area);
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
