use crate::app::AppState;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, state: &AppState) {
    let t = crate::theme::current();
    let agents = &state.system.user_config.agents;
    // Same dialog, two errands. Picking the provider is identical either way —
    // a manager is one of these with a brief — so only the framing changes.
    let for_manager = state.ui.input_mode == crate::app::InputMode::CreateManager;
    let for_assign = state.ui.input_mode == crate::app::InputMode::AssignAgent;
    let enabled_count = agents.iter().filter(|a| a.enabled).count();
    // Height has to follow the content, because the content grew: with seven
    // agents configured the box was already cutting off the Terminal line
    // before Manager was added to it. Counted rather than guessed —
    // header, one row per enabled agent, the two extra rows with their
    // separating blanks, the Esc line, and the border.
    let needed_lines = 5 + enabled_count + if for_manager || for_assign { 2 } else { 6 } + 2;
    let height_pct =
        ((needed_lines * 100) / frame.area().height.max(1) as usize).clamp(25, 85) as u16;
    let area = centered_rect(40, height_pct, frame.area());
    frame.render_widget(Clear, area);

    let workspace_name = state
        .selected_workspace()
        .map(|w| w.name.as_str())
        .unwrap_or("Unknown");

    let mut content = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  Workspace: {}", workspace_name),
            Style::default().fg(t.fg_dim),
        )),
        Line::from(""),
        Line::from(Span::styled(
            if for_manager {
                "  Which agent runs it?"
            } else if for_assign {
                "  Who should do this work?"
            } else {
                "  Select an agent:"
            },
            Style::default().fg(t.fg).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    for agent in agents {
        if !agent.enabled {
            continue;
        }
        content.push(Line::from(vec![
            Span::styled(
                format!("  [{}] ", agent.hotkey),
                Style::default().fg(t.accent),
            ),
            Span::styled(
                format!("[{}] ", agent.badge),
                Style::default().fg(t.special),
            ),
            Span::raw(agent.display_name.clone()),
        ]));
    }

    if !for_manager && !for_assign {
        content.push(Line::from(""));
        content.push(Line::from(vec![
            Span::styled("  [t] ", Style::default().fg(t.accent)),
            Span::styled("[T] ", Style::default().fg(t.special)),
            Span::raw("Terminal"),
        ]));
        content.push(Line::from(""));
        content.push(Line::from(vec![
            Span::styled("  [m] ", Style::default().fg(t.accent)),
            Span::styled("[M] ", Style::default().fg(t.special)),
            Span::raw("Manager, then pick its agent"),
        ]));
    }
    content.push(Line::from(""));
    content.push(Line::from(Span::styled(
        "  Press Esc to cancel",
        Style::default().fg(t.fg_faint),
    )));

    let block = Block::default()
        .title(if for_manager {
            " New Manager "
        } else if for_assign {
            " Assign Agent "
        } else {
            " New Session "
        })
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.special))
        .style(Style::default().bg(t.bg));

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
