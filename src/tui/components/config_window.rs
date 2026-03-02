use crate::app::{AppState, ConfigTab};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, state: &AppState) {
    let area = centered_rect(70, 80, frame.area());
    frame.render_widget(Clear, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);

    render_tab_bar(frame, chunks[0], state);

    match state.ui.config_tab {
        ConfigTab::Agents => render_agents_tab(frame, chunks[1], state),
        ConfigTab::Hotkeys => render_hotkeys_tab(frame, chunks[1], state),
        ConfigTab::Scrollback => render_scrollback_tab(frame, chunks[1], state),
    }
}

fn render_tab_bar(frame: &mut Frame, area: Rect, state: &AppState) {
    let active = state.ui.config_tab;

    let tabs = vec![
        ("1", "Agents", ConfigTab::Agents),
        ("2", "Hotkeys", ConfigTab::Hotkeys),
        ("3", "Memory", ConfigTab::Scrollback),
    ];

    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::raw("  "));

    for (i, (num, label, tab)) in tabs.iter().enumerate() {
        if *tab == active {
            spans.push(Span::styled(
                format!(" {} {} ", num, label),
                Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                format!(" {} ", num),
                Style::default().fg(Color::Cyan),
            ));
            spans.push(Span::styled(
                format!("{} ", label),
                Style::default().fg(Color::Gray),
            ));
        }
        if i < tabs.len() - 1 {
            spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
        }
    }

    let line = Line::from(spans);

    let block = Block::default()
        .title(" Settings (F12) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));

    let paragraph = Paragraph::new(vec![Line::from(""), line])
        .block(block);

    frame.render_widget(paragraph, area);
}

fn render_agents_tab(frame: &mut Frame, area: Rect, state: &AppState) {
    let agents = &state.system.user_config.agents;
    let selected_row = state.ui.config_selected_row;
    let selected_col = state.ui.config_selected_col;
    let editing = state.ui.config_editing;
    let edit_buffer = &state.ui.config_edit_buffer;

    let mut lines: Vec<Line> = Vec::new();

    // Header
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  #  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Key     ", Style::default().fg(Color::DarkGray)),
        Span::styled("Name          ", Style::default().fg(Color::DarkGray)),
        Span::styled("Command            ", Style::default().fg(Color::DarkGray)),
        Span::styled("Badge", Style::default().fg(Color::DarkGray)),
    ]));
    lines.push(Line::from(Span::styled(
        "  ─────────────────────────────────────────────────────────",
        Style::default().fg(Color::DarkGray),
    )));

    for (idx, agent) in agents.iter().enumerate() {
        let is_selected = idx == selected_row;
        let row_bg = if is_selected {
            Color::DarkGray
        } else {
            Color::Black
        };
        let row_fg = if is_selected {
            Color::White
        } else {
            Color::Gray
        };

        let num_str = format!("  {}  ", idx + 1);

        // Build each field, highlighting the selected cell
        let hotkey_val = if editing && is_selected && selected_col == 0 {
            format!("{}_", edit_buffer)
        } else {
            agent.hotkey.clone()
        };
        let name_val = if editing && is_selected && selected_col == 1 {
            format!("{}_", edit_buffer)
        } else {
            agent.display_name.clone()
        };
        let cmd_val = if editing && is_selected && selected_col == 2 {
            format!("{}_", edit_buffer)
        } else {
            agent.command.clone()
        };
        let badge_val = if editing && is_selected && selected_col == 3 {
            format!("{}_", edit_buffer)
        } else {
            agent.badge.clone()
        };

        // Pad fields to fixed widths
        let hotkey_display = format!("{:<8}", hotkey_val);
        let name_display = format!("{:<14}", name_val);
        let cmd_display = format!("{:<19}", cmd_val);
        let badge_display = format!("{:<5}", badge_val);

        let cell_style = |col: usize| -> Style {
            if is_selected && selected_col == col {
                Style::default()
                    .fg(Color::Cyan)
                    .bg(row_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(row_fg).bg(row_bg)
            }
        };

        lines.push(Line::from(vec![
            Span::styled(num_str, Style::default().fg(Color::DarkGray).bg(row_bg)),
            Span::styled(hotkey_display, cell_style(0)),
            Span::styled(name_display, cell_style(1)),
            Span::styled(cmd_display, cell_style(2)),
            Span::styled(badge_display, cell_style(3)),
        ]));
    }

    // Padding
    lines.push(Line::from(""));

    // Footer
    lines.push(Line::from(Span::styled(
        "  ─────────────────────────────────────────────────────────",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(vec![
        Span::styled("  [j/k]", Style::default().fg(Color::Cyan)),
        Span::raw(" Navigate  "),
        Span::styled("[h/l]", Style::default().fg(Color::Cyan)),
        Span::raw(" Field  "),
        Span::styled("[Enter]", Style::default().fg(Color::Cyan)),
        Span::raw(" Edit  "),
        Span::styled("[a]", Style::default().fg(Color::Cyan)),
        Span::raw(" Add  "),
        Span::styled("[d]", Style::default().fg(Color::Cyan)),
        Span::raw(" Delete  "),
        Span::styled("[J/K]", Style::default().fg(Color::Cyan)),
        Span::raw(" Reorder"),
    ]));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .style(Style::default().bg(Color::Black));

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn format_action_name(action: &str) -> &str {
    match action {
        "CycleNextWorkspace" => "Cycle Workspace",
        "CycleNextSession" => "Cycle Session",
        "InitiateQuit" => "Quit",
        "EnterHelpMode" => "Help",
        "ToggleDebugOverlay" => "Debug Overlay",
        "EnterConfigWindow" => "Config Window",
        _ => action,
    }
}

fn render_hotkeys_tab(frame: &mut Frame, area: Rect, state: &AppState) {
    let hotkeys = &state.system.user_config.global_hotkeys;
    let selected_row = state.ui.config_selected_row;
    let rebinding = state.ui.config_rebinding;

    // Sort by action name for consistent ordering
    let mut sorted_keys: Vec<&String> = hotkeys.keys().collect();
    sorted_keys.sort();

    let mut lines: Vec<Line> = Vec::new();

    // Header
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  Action                    ", Style::default().fg(Color::DarkGray)),
        Span::styled("Key", Style::default().fg(Color::DarkGray)),
    ]));
    lines.push(Line::from(Span::styled(
        "  ─────────────────────────────────────────────────────────",
        Style::default().fg(Color::DarkGray),
    )));

    for (idx, action) in sorted_keys.iter().enumerate() {
        let is_selected = idx == selected_row;
        let row_bg = if is_selected {
            Color::DarkGray
        } else {
            Color::Black
        };
        let row_fg = if is_selected {
            Color::White
        } else {
            Color::Gray
        };

        let display_name = format_action_name(action);
        let key_val = hotkeys.get(*action).map(|s| s.as_str()).unwrap_or("???");

        let key_display = if rebinding && is_selected {
            Span::styled("Press a key...", Style::default().fg(Color::Yellow).bg(row_bg))
        } else {
            Span::styled(key_val.to_string(), Style::default().fg(row_fg).bg(row_bg))
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<26}", display_name),
                Style::default().fg(row_fg).bg(row_bg),
            ),
            key_display,
        ]));
    }

    // Padding
    lines.push(Line::from(""));

    // Footer
    lines.push(Line::from(Span::styled(
        "  ─────────────────────────────────────────────────────────",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(vec![
        Span::styled("  [j/k]", Style::default().fg(Color::Cyan)),
        Span::raw(" Navigate  "),
        Span::styled("[Enter]", Style::default().fg(Color::Cyan)),
        Span::raw(" Rebind  "),
        Span::styled("[r]", Style::default().fg(Color::Cyan)),
        Span::raw(" Reset all"),
    ]));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .style(Style::default().bg(Color::Black));

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn render_scrollback_tab(frame: &mut Frame, area: Rect, state: &AppState) {
    let config = &state.system.user_config;
    let selected_row = state.ui.config_selected_row;
    let editing = state.ui.config_editing;
    let edit_buffer = &state.ui.config_edit_buffer;

    let settings: Vec<(&str, String, &str)> = vec![
        (
            "Raw buffer (KB)",
            config.scrollback_buffer_kb.to_string(),
            "Total bytes of raw terminal output kept per session. Increase to preserve more history when replaying output. Uses more memory.",
        ),
        (
            "Replay rows",
            config.replay_parser_rows.to_string(),
            "Height of the virtual terminal used to replay raw output for scrollback. More rows = more visible scrollback lines with colors/formatting intact.",
        ),
        (
            "Live scrollback rows",
            config.live_scrollback_rows.to_string(),
            "Lines the live terminal parser keeps above the visible area. Scroll up in a session to see this many lines of recent output.",
        ),
    ];

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    for (idx, (label, value, desc)) in settings.iter().enumerate() {
        let is_selected = idx == selected_row;
        let row_bg = if is_selected {
            Color::DarkGray
        } else {
            Color::Black
        };
        let row_fg = if is_selected {
            Color::White
        } else {
            Color::Gray
        };

        let val_display = if editing && is_selected {
            format!("{}_", edit_buffer)
        } else {
            value.clone()
        };

        let val_style = if editing && is_selected {
            Style::default()
                .fg(Color::Cyan)
                .bg(row_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Cyan).bg(row_bg)
        };

        let marker = if is_selected { ">" } else { " " };

        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", marker), Style::default().fg(Color::Cyan).bg(row_bg)),
            Span::styled(format!("{:<22}", label), Style::default().fg(row_fg).bg(row_bg).add_modifier(if is_selected { Modifier::BOLD } else { Modifier::empty() })),
            Span::styled(val_display, val_style),
        ]));

        // Description below the setting
        lines.push(Line::from(vec![
            Span::styled("    ", Style::default().bg(row_bg)),
            Span::styled(desc.to_string(), Style::default().fg(Color::DarkGray).bg(row_bg)),
        ]));
        lines.push(Line::from(""));
    }

    // Footer
    lines.push(Line::from(Span::styled(
        "  ─────────────────────────────────────────────────────────",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(vec![
        Span::styled("  [j/k]", Style::default().fg(Color::Cyan)),
        Span::raw(" Navigate  "),
        Span::styled("[Enter]", Style::default().fg(Color::Cyan)),
        Span::raw(" Edit  "),
        Span::styled("[r]", Style::default().fg(Color::Cyan)),
        Span::raw(" Reset defaults"),
    ]));
    lines.push(Line::from(Span::styled(
        "  Changes apply to new sessions only",
        Style::default().fg(Color::DarkGray),
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .style(Style::default().bg(Color::Black));

    let paragraph = Paragraph::new(lines).block(block);
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
