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
        ConfigTab::QuickRef => render_quickref_tab(frame, chunks[1], state),
        ConfigTab::Agents => render_agents_tab(frame, chunks[1], state),
        ConfigTab::Hotkeys => render_hotkeys_tab(frame, chunks[1], state),
        ConfigTab::Scrollback => render_scrollback_tab(frame, chunks[1], state),
    }
}

fn render_tab_bar(frame: &mut Frame, area: Rect, state: &AppState) {
    let active = state.ui.config_tab;

    let tabs = [
        ("1", "Quick Ref", ConfigTab::QuickRef),
        ("2", "Agents", ConfigTab::Agents),
        ("3", "Hotkeys", ConfigTab::Hotkeys),
        ("4", "Memory", ConfigTab::Scrollback),
    ];

    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::raw("  "));

    for (i, (num, label, tab)) in tabs.iter().enumerate() {
        if *tab == active {
            spans.push(Span::styled(
                format!(" {} {} ", num, label),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
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
        .title(" Help & Settings (F1) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));

    let paragraph = Paragraph::new(vec![Line::from(""), line]).block(block);

    frame.render_widget(paragraph, area);
}

fn render_quickref_tab(frame: &mut Frame, area: Rect, state: &AppState) {
    let scroll_offset = state.ui.config_scroll_offset;

    let mut lines: Vec<Line> = Vec::new();

    let section_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let key_style = Style::default().fg(Color::Cyan);
    let sep_style = Style::default().fg(Color::DarkGray);

    let sep = || {
        Line::from(Span::styled(
            "  ──────────────────────────────────────────────────────",
            sep_style,
        ))
    };

    // -- Navigation --
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  Navigation", section_style)));
    lines.push(sep());
    lines.push(Line::from(vec![
        Span::styled("  j/k, Up/Down       ", key_style),
        Span::raw("Move up/down in lists"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  h/l, Left/Right    ", key_style),
        Span::raw("Switch between panels"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Tab                ", key_style),
        Span::raw("Cycle focus between panels"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Shift+Left/Right   ", key_style),
        Span::raw("Focus left/right panel"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  `                  ", key_style),
        Span::raw("Jump to next idle session"),
    ]));

    // -- Workspaces --
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  Workspaces", section_style)));
    lines.push(sep());
    lines.push(Line::from(vec![
        Span::styled("  n                  ", key_style),
        Span::raw("Create/open workspace"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Enter              ", key_style),
        Span::raw("Select workspace"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  w                  ", key_style),
        Span::raw("Toggle working/paused"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  d                  ", key_style),
        Span::raw("Delete workspace"),
    ]));

    // -- Sessions --
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  Sessions", section_style)));
    lines.push(sep());
    lines.push(Line::from(vec![
        Span::styled("  1/2/3/4            ", key_style),
        Span::raw("New Claude/Gemini/Codex/Grok"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  !/@ /#/$           ", key_style),
        Span::raw("Same but skip permissions"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Alt+1-4            ", key_style),
        Span::raw("Create in isolated worktree"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Alt+!/@ /#         ", key_style),
        Span::raw("Worktree + skip permissions"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  t                  ", key_style),
        Span::raw("New terminal"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  P                  ", key_style),
        Span::raw("Start parallel task"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Enter              ", key_style),
        Span::raw("Activate selected session"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  s                  ", key_style),
        Span::raw("Stop session (graceful)"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  x                  ", key_style),
        Span::raw("Kill session (force)"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  d                  ", key_style),
        Span::raw("Delete session"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  p                  ", key_style),
        Span::raw("Pin/unpin to side panel"),
    ]));

    // -- Worktrees --
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  Worktrees", section_style)));
    lines.push(sep());
    lines.push(Line::from(vec![
        Span::styled("  w                  ", key_style),
        Span::raw("Open terminal in worktree"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  m                  ", key_style),
        Span::raw("Merge worktree into main"),
    ]));

    // -- Todos --
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  Todos", section_style)));
    lines.push(sep());
    lines.push(Line::from(vec![
        Span::styled("  n                  ", key_style),
        Span::raw("Create new todo"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Enter              ", key_style),
        Span::raw("Run todo with agent"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  y / Y              ", key_style),
        Span::raw("Accept suggested / Accept all"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  x                  ", key_style),
        Span::raw("Mark as done"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  X                  ", key_style),
        Span::raw("Archive todo"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  d                  ", key_style),
        Span::raw("Delete todo"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Tab                ", key_style),
        Span::raw("Switch tabs (Active/Archived/Reports)"),
    ]));

    // -- Todo Reports --
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  Todo Reports", section_style)));
    lines.push(sep());
    lines.push(Line::from(vec![
        Span::styled("  v                  ", key_style),
        Span::raw("View report details"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  m                  ", key_style),
        Span::raw("Merge selected attempt"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  d                  ", key_style),
        Span::raw("Discard attempt"),
    ]));

    // -- Utilities --
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  Utilities", section_style)));
    lines.push(sep());
    lines.push(Line::from(vec![
        Span::styled("  Tab                ", key_style),
        Span::raw("Switch tabs (Util/Sounds/Cfg/Notes)"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Enter              ", key_style),
        Span::raw("Toggle/activate item"),
    ]));

    // -- Output Pane --
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  Output Pane", section_style)));
    lines.push(sep());
    lines.push(Line::from(vec![
        Span::styled("  (type)             ", key_style),
        Span::raw("Send input to active session"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Ctrl+H             ", key_style),
        Span::raw("Return to session list"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Esc                ", key_style),
        Span::raw("Send escape to agent (interrupt)"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Ctrl+C             ", key_style),
        Span::raw("Send interrupt signal"),
    ]));

    // -- General --
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  General", section_style)));
    lines.push(sep());
    lines.push(Line::from(vec![
        Span::styled("  F1                 ", key_style),
        Span::raw("Open this window"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  q                  ", key_style),
        Span::raw("Quit workbench"),
    ]));
    lines.push(Line::from(""));

    // Footer
    lines.push(Line::from(Span::styled(
        "  ──────────────────────────────────────────────────────",
        sep_style,
    )));
    lines.push(Line::from(vec![
        Span::styled("  [j/k]", Style::default().fg(Color::Cyan)),
        Span::raw(" Scroll"),
    ]));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .style(Style::default().bg(Color::Black));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((scroll_offset as u16, 0));
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
        "CycleNextWorkspace" => "Cycle Workspace →",
        "CyclePrevWorkspace" => "Cycle Workspace ←",
        "CycleNextSession" => "Cycle Session →",
        "CyclePrevSession" => "Cycle Session ←",
        "InitiateQuit" => "Quit",
        "ToggleDebugOverlay" => "Debug Overlay",
        "EnterConfigWindow" => "Help & Settings",
        _ => action,
    }
}

fn render_hotkeys_tab(frame: &mut Frame, area: Rect, state: &AppState) {
    let hotkeys = &state.system.user_config.global_hotkeys;
    let selected_row = state.ui.config_selected_row;
    let rebinding = state.ui.config_rebinding;

    let ordered_actions = crate::config::user_config::ordered_global_hotkey_actions(hotkeys);

    let mut lines: Vec<Line> = Vec::new();

    // Header
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            "  Action                    ",
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled("Key", Style::default().fg(Color::DarkGray)),
    ]));
    lines.push(Line::from(Span::styled(
        "  ─────────────────────────────────────────────────────────",
        Style::default().fg(Color::DarkGray),
    )));

    for (idx, action) in ordered_actions.iter().enumerate() {
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
        let key_val = hotkeys.get(action).map(|s| s.as_str()).unwrap_or("???");

        let key_display = if rebinding && is_selected {
            Span::styled(
                "Press a key...",
                Style::default().fg(Color::Yellow).bg(row_bg),
            )
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
    let editing = state.ui.config_editing;
    let edit_buffer = &state.ui.config_edit_buffer;

    let row_bg = Color::DarkGray;

    let val_display = if editing {
        format!("{}_", edit_buffer)
    } else {
        format!("{}", config.scrollback_mb)
    };

    let val_style = if editing {
        Style::default()
            .fg(Color::Cyan)
            .bg(row_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Cyan).bg(row_bg)
    };

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    lines.push(Line::from(vec![
        Span::styled("  > ", Style::default().fg(Color::Cyan).bg(row_bg)),
        Span::styled(
            format!("{:<22}", "Scrollback (MB)"),
            Style::default()
                .fg(Color::White)
                .bg(row_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(val_display, val_style),
        Span::styled(
            "  (range: 1-16)".to_string(),
            Style::default().fg(Color::DarkGray).bg(row_bg),
        ),
    ]));

    lines.push(Line::from(vec![
        Span::styled("    ", Style::default().bg(row_bg)),
        Span::styled(
            "Memory per session for terminal scrollback history.",
            Style::default().fg(Color::DarkGray).bg(row_bg),
        ),
    ]));
    lines.push(Line::from(""));

    // Show derived values as read-only info
    lines.push(Line::from(vec![Span::styled(
        "    Current allocation per session:",
        Style::default().fg(Color::Gray),
    )]));
    lines.push(Line::from(vec![
        Span::styled(
            "      Raw buffer:       ",
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("{} KB", config.scrollback_buffer_kb),
            Style::default().fg(Color::Gray),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            "      Replay rows:      ",
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("{}", config.replay_parser_rows),
            Style::default().fg(Color::Gray),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            "      Live scrollback:  ",
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("{} rows", config.live_scrollback_rows),
            Style::default().fg(Color::Gray),
        ),
    ]));

    lines.push(Line::from(""));

    // Footer
    lines.push(Line::from(Span::styled(
        "  ─────────────────────────────────────────────────────────",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(vec![
        Span::styled("  [Enter]", Style::default().fg(Color::Cyan)),
        Span::raw(" Edit  "),
        Span::styled("[r]", Style::default().fg(Color::Cyan)),
        Span::raw(" Reset default"),
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
