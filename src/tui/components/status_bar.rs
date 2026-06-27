use crate::app::{AppState, FocusPanel, InputMode, PendingDelete};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = crate::theme::current();
    // Check for pending delete confirmation first
    if let Some(pending) = &state.ui.pending_delete {
        let (item_type, name) = match pending {
            PendingDelete::Session(_, name) => ("session", name.as_str()),
            PendingDelete::Workspace(_, name) => ("workspace", name.as_str()),
            PendingDelete::Todo(_, name) => ("todo", name.as_str()),
        };

        let left_text = vec![
            Span::styled(
                " DELETE? ",
                Style::default()
                    .fg(t.fg)
                    .bg(t.error)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                format!("Delete {} \"{}\"?", item_type, name),
                Style::default().fg(t.error).add_modifier(Modifier::BOLD),
            ),
        ];

        let right_text = vec![
            Span::styled("Press ", Style::default().fg(t.fg_dim)),
            Span::styled(
                "[d]",
                Style::default().fg(t.error).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " to confirm, any other key to cancel",
                Style::default().fg(t.fg_dim),
            ),
        ];

        let left_len: usize = left_text.iter().map(|s| s.content.len()).sum();
        let right_len: usize = right_text.iter().map(|s| s.content.len()).sum();
        let padding = area
            .width
            .saturating_sub(left_len as u16 + right_len as u16 + 2);

        let mut spans = left_text;
        spans.push(Span::raw(" ".repeat(padding as usize)));
        spans.extend(right_text);
        spans.push(Span::raw(" "));

        let paragraph = Paragraph::new(Line::from(spans))
            .style(Style::default().bg(t.fg_faint).fg(t.fg));

        frame.render_widget(paragraph, area);
        return;
    }

    // Check for pending quit confirmation
    if state.ui.pending_quit {
        let left_text = vec![
            Span::styled(
                " QUIT? ",
                Style::default()
                    .fg(t.fg)
                    .bg(t.error)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                "Are you sure you want to exit?",
                Style::default()
                    .fg(t.warning)
                    .add_modifier(Modifier::BOLD),
            ),
        ];

        let right_text = vec![
            Span::styled("Press ", Style::default().fg(t.fg_dim)),
            Span::styled(
                "[Esc/q/y]",
                Style::default().fg(t.error).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " to confirm, any other key to cancel",
                Style::default().fg(t.fg_dim),
            ),
        ];

        let left_len: usize = left_text.iter().map(|s| s.content.len()).sum();
        let right_len: usize = right_text.iter().map(|s| s.content.len()).sum();
        let padding = area
            .width
            .saturating_sub(left_len as u16 + right_len as u16 + 2);

        let mut spans = left_text;
        spans.push(Span::raw(" ".repeat(padding as usize)));
        spans.extend(right_text);
        spans.push(Span::raw(" "));

        let paragraph = Paragraph::new(Line::from(spans))
            .style(Style::default().bg(t.fg_faint).fg(t.fg));

        frame.render_widget(paragraph, area);
        return;
    }

    let (left_text, right_text) = match state.ui.input_mode {
        InputMode::CreateWorkspace => (
            vec![Span::styled(
                " NEW WORKSPACE ",
                Style::default()
                    .fg(t.on_accent)
                    .bg(t.success)
                    .add_modifier(Modifier::BOLD),
            )],
            vec![Span::styled(
                if state.ui.workspace_create_mode {
                    "Browse to parent, Space to name, Esc to cancel"
                } else {
                    "Type path or filter, Enter to open, Space to select, Esc to cancel"
                },
                Style::default().fg(t.fg_dim),
            )],
        ),
        InputMode::CreateSession => (
            vec![Span::styled(
                " NEW SESSION ",
                Style::default()
                    .fg(t.on_accent)
                    .bg(t.special)
                    .add_modifier(Modifier::BOLD),
            )],
            vec![Span::styled(
                "1=Claude 2=Gemini 3=Codex 4=Grok  t=Terminal  Esc=Cancel",
                Style::default().fg(t.fg_dim),
            )],
        ),
        InputMode::SetStartCommand => (
            vec![
                Span::styled(
                    " START COMMAND ",
                    Style::default()
                        .fg(t.on_accent)
                        .bg(t.special)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" $ "),
                Span::styled(
                    format!("{}_", state.ui.input_buffer),
                    Style::default().fg(t.fg),
                ),
            ],
            vec![Span::styled(
                "Enter command to run on terminal start, Enter to save, Esc to cancel",
                Style::default().fg(t.fg_dim),
            )],
        ),
        InputMode::CreateTodo => (
            vec![
                Span::styled(
                    " NEW TODO ",
                    Style::default()
                        .fg(t.on_accent)
                        .bg(t.success)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("{}_", state.ui.input_buffer),
                    Style::default().fg(t.fg),
                ),
            ],
            vec![Span::styled(
                "Enter todo description, press Enter to create, Esc to cancel",
                Style::default().fg(t.fg_dim),
            )],
        ),
        InputMode::SelectWorkspaceAction => (
            vec![Span::styled(
                " ADD WORKSPACE ",
                Style::default()
                    .fg(t.on_accent)
                    .bg(t.accent)
                    .add_modifier(Modifier::BOLD),
            )],
            vec![Span::styled(
                "↑/↓ Navigate  Enter to select  Esc to cancel",
                Style::default().fg(t.fg_dim),
            )],
        ),
        InputMode::EnterWorkspaceName => (
            vec![
                Span::styled(
                    " CREATE PROJECT ",
                    Style::default()
                        .fg(t.on_accent)
                        .bg(t.active)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("{}_", state.ui.input_buffer),
                    Style::default().fg(t.fg),
                ),
            ],
            vec![Span::styled(
                "Enter project name, press Enter to create, Esc to cancel",
                Style::default().fg(t.fg_dim),
            )],
        ),
        InputMode::CreateParallelTask => (
            vec![Span::styled(
                " PARALLEL TASK ",
                Style::default()
                    .fg(t.on_accent)
                    .bg(t.special)
                    .add_modifier(Modifier::BOLD),
            )],
            vec![Span::styled(
                "Tab: toggle agent  Enter: start  Esc: cancel",
                Style::default().fg(t.fg_dim),
            )],
        ),
        InputMode::ConfirmParallelMerge => (
            vec![Span::styled(
                " MERGE PARALLEL ",
                Style::default()
                    .fg(t.on_accent)
                    .bg(t.active)
                    .add_modifier(Modifier::BOLD),
            )],
            vec![Span::styled(
                "Y/Enter: commit & merge  N/Esc: cancel",
                Style::default().fg(t.fg_dim),
            )],
        ),
        InputMode::ConfirmMergeWorktree => (
            vec![Span::styled(
                " MERGE WORKTREE ",
                Style::default()
                    .fg(t.on_accent)
                    .bg(t.active)
                    .add_modifier(Modifier::BOLD),
            )],
            vec![Span::styled(
                "Y/Enter: commit & merge  N/Esc: cancel",
                Style::default().fg(t.fg_dim),
            )],
        ),
        InputMode::CommandPalette => (
            vec![Span::styled(
                " COMMAND PALETTE ",
                Style::default()
                    .fg(t.on_accent)
                    .bg(t.accent)
                    .add_modifier(Modifier::BOLD),
            )],
            vec![Span::styled(
                "Type to filter  Enter: execute  Esc: close",
                Style::default().fg(t.fg_dim),
            )],
        ),
        InputMode::ConfigWindow => (
            vec![Span::styled(
                " CONFIG ",
                Style::default()
                    .fg(t.on_accent)
                    .bg(t.active)
                    .add_modifier(Modifier::BOLD),
            )],
            vec![Span::styled(
                "Tab: switch tab  Enter: edit  Esc: close",
                Style::default().fg(t.fg_dim),
            )],
        ),
        InputMode::Normal => {
            let context_hints = match state.ui.focus {
                FocusPanel::WorkspaceList => vec![
                    Span::styled("[n]", Style::default().fg(t.accent)),
                    Span::raw(" New  "),
                    Span::styled("[Enter/l]", Style::default().fg(t.accent)),
                    Span::raw(" Sessions  "),
                    Span::styled("[?]", Style::default().fg(t.accent)),
                    Span::raw(" Help  "),
                    Span::styled("[q]", Style::default().fg(t.accent)),
                    Span::raw(" Quit"),
                ],
                FocusPanel::SessionList => vec![
                    Span::styled("[1]", Style::default().fg(t.accent)),
                    Span::raw("Claude "),
                    Span::styled("[2]", Style::default().fg(t.accent)),
                    Span::raw("Gemini "),
                    Span::styled("[3]", Style::default().fg(t.accent)),
                    Span::raw("Codex "),
                    Span::styled("[4]", Style::default().fg(t.accent)),
                    Span::raw("Grok "),
                    Span::styled("[t]", Style::default().fg(t.accent)),
                    Span::raw("Term "),
                    Span::styled("[s]", Style::default().fg(t.accent)),
                    Span::raw("Stop"),
                ],
                FocusPanel::TodosPane => vec![
                    Span::styled("[n]", Style::default().fg(t.accent)),
                    Span::raw(" New  "),
                    Span::styled("[Enter]", Style::default().fg(t.accent)),
                    Span::raw(" Run  "),
                    Span::styled("[x]", Style::default().fg(t.accent)),
                    Span::raw(" Done  "),
                    Span::styled("[a]", Style::default().fg(t.accent)),
                    Span::raw(" Autorun  "),
                    Span::styled("[d]", Style::default().fg(t.accent)),
                    Span::raw(" Del"),
                ],
                FocusPanel::OutputPane => {
                    if state.ui.active_session_id.is_some() {
                        vec![
                            Span::styled("[Esc]", Style::default().fg(t.accent)),
                            Span::raw(" Back  "),
                            Span::styled("[Ctrl+C]", Style::default().fg(t.accent)),
                            Span::raw(" Interrupt  "),
                            Span::styled("Type", Style::default().fg(t.active)),
                            Span::raw(" to send input"),
                        ]
                    } else {
                        vec![
                            Span::styled("[h/Esc]", Style::default().fg(t.accent)),
                            Span::raw(" Back to sessions"),
                        ]
                    }
                }
                FocusPanel::PinnedTerminalPane(_) => {
                    vec![
                        Span::styled("[Esc]", Style::default().fg(t.accent)),
                        Span::raw(" Back  "),
                        Span::styled("[Tab]", Style::default().fg(t.accent)),
                        Span::raw(" Next pane  "),
                        Span::styled("[Ctrl+C]", Style::default().fg(t.accent)),
                        Span::raw(" Interrupt  "),
                        Span::styled("Type", Style::default().fg(t.active)),
                        Span::raw(" to send input"),
                    ]
                }
                FocusPanel::UtilitiesPane => {
                    vec![
                        Span::styled("[j/k]", Style::default().fg(t.accent)),
                        Span::raw(" Navigate  "),
                        Span::styled("[Enter]", Style::default().fg(t.accent)),
                        Span::raw(" Select  "),
                        Span::styled("[Tab]", Style::default().fg(t.accent)),
                        Span::raw(" Switch Section  "),
                        Span::styled("[h]", Style::default().fg(t.accent)),
                        Span::raw(" Back"),
                    ]
                }
            };

            let running = state.running_session_count();
            let total_sessions: usize = state.data.sessions.values().map(|s| s.len()).sum();
            let idle_count = state.idle_queue_count();

            let mut status = vec![
                Span::styled(
                    " WORKBENCH ",
                    Style::default()
                        .fg(t.on_accent)
                        .bg(t.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("{} workspaces", state.data.workspaces.len()),
                    Style::default().fg(t.fg_dim),
                ),
                Span::raw(" | "),
                Span::styled(
                    format!("{}/{} sessions", running, total_sessions),
                    Style::default().fg(if running > 0 {
                        t.success
                    } else {
                        t.fg_dim
                    }),
                ),
            ];

            // Show idle queue if there are waiting sessions
            if idle_count > 0 {
                status.push(Span::raw(" | "));
                status.push(Span::styled(
                    format!("{} idle", idle_count),
                    Style::default()
                        .fg(t.active)
                        .add_modifier(Modifier::BOLD),
                ));
                status.push(Span::styled(" [`]", Style::default().fg(t.active)));
            }

            // Performance metrics - always visible
            let fps = state.system.perf.fps();
            let frame_ms = state.system.perf.frame_time_ms();
            let mem_mb = state.system.perf.memory_mb();
            let pty_batch = state.system.perf.avg_pty_batch();

            status.push(Span::raw(" | "));
            status.push(Span::styled(
                format!("{:.0}fps", fps),
                Style::default().fg(if fps >= 30.0 {
                    t.success
                } else if fps >= 15.0 {
                    t.active
                } else {
                    t.error
                }),
            ));
            status.push(Span::styled(
                format!(" {:.0}ms", frame_ms),
                Style::default().fg(t.fg_faint),
            ));
            status.push(Span::raw(" "));
            status.push(Span::styled(
                format!("{:.0}MB", mem_mb),
                Style::default().fg(if mem_mb < 100.0 {
                    t.success
                } else if mem_mb < 300.0 {
                    t.active
                } else {
                    t.error
                }),
            ));
            if pty_batch > 0.1 {
                status.push(Span::raw(" "));
                status.push(Span::styled(
                    format!("pty:{:.0}", pty_batch),
                    Style::default().fg(t.accent),
                ));
            }

            (status, context_hints)
        }
    };

    // Calculate spacing
    let left_len: usize = left_text.iter().map(|s| s.content.len()).sum();
    let right_len: usize = right_text.iter().map(|s| s.content.len()).sum();
    let padding = area
        .width
        .saturating_sub(left_len as u16 + right_len as u16 + 2);

    let mut spans = left_text;
    spans.push(Span::raw(" ".repeat(padding as usize)));
    spans.extend(right_text);
    spans.push(Span::raw(" "));

    let paragraph = Paragraph::new(Line::from(spans))
        .style(Style::default().bg(t.fg_faint).fg(t.fg));

    frame.render_widget(paragraph, area);
}
