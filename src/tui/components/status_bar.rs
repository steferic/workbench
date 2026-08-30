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

        let paragraph =
            Paragraph::new(Line::from(spans)).style(Style::default().bg(t.fg_faint).fg(t.fg));

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
                Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
            ),
        ];

        let right_text = vec![
            Span::styled("Press ", Style::default().fg(t.fg_dim)),
            Span::styled(
                "[y]",
                Style::default().fg(t.error).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " or Enter to confirm, Esc to cancel",
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

        let paragraph =
            Paragraph::new(Line::from(spans)).style(Style::default().bg(t.fg_faint).fg(t.fg));

        frame.render_widget(paragraph, area);
        return;
    }

    let (left_text, right_text) =
        match state.ui.input_mode {
            // The modal itself says what the keys are; nothing to add here.
            InputMode::CreateManager | InputMode::AssignAgent => (Vec::new(), Vec::new()),
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
            InputMode::ComposeTaskMessage => {
                use crate::app::TaskEdit;
                let (label, hint) = match state.ui.task_edit.as_ref().map(|(_, edit, _)| *edit) {
                    Some(TaskEdit::Rewrite) => (
                        " EDIT TODO ",
                        "Change the text, Enter to save, Esc to cancel",
                    ),
                    _ => (
                        " ADD TODO ",
                        "Describe the work, Enter to queue it for this agent, Esc to cancel",
                    ),
                };
                (
                    vec![
                        Span::styled(
                            label,
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
                    vec![Span::styled(hint, Style::default().fg(t.fg_dim))],
                )
            }
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
                // An agent stopped for you outranks key hints: it is the one thing
                // on screen that is costing time while it goes unread. The agent's
                // own words say what it wants, which "needs approval" cannot.
                let waiting = state.sessions_needing_attention().first().copied().map(
                    |(session_id, kind)| {
                        let name = state
                            .get_session(session_id)
                            .map(|s| s.display_name())
                            .unwrap_or_else(|| "An agent".to_string());
                        let detail = state
                            .activity_reason(session_id)
                            .filter(|reason| !reason.is_empty())
                            .map(str::to_string)
                            .unwrap_or_else(|| kind.label().to_string());
                        (
                            vec![Span::styled(
                                format!(" {name} "),
                                Style::default()
                                    .fg(t.on_accent)
                                    .bg(t.warning)
                                    .add_modifier(Modifier::BOLD),
                            )],
                            vec![Span::styled(detail, Style::default().fg(t.warning))],
                        )
                    },
                );

                let context_hints = match state.ui.focus {
                    FocusPanel::WorkspaceList => vec![
                        Span::styled("[n]", Style::default().fg(t.accent)),
                        Span::raw(" New  "),
                        Span::styled("[Enter/l]", Style::default().fg(t.accent)),
                        Span::raw(" Sessions  "),
                        Span::styled("[?]", Style::default().fg(t.accent)),
                        Span::raw(" Help  "),
                        Span::styled("[^Q]", Style::default().fg(t.accent)),
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
                    FocusPanel::TasksPane => vec![
                        Span::styled("[n]", Style::default().fg(t.accent)),
                        Span::raw(" Add  "),
                        Span::styled("[e]", Style::default().fg(t.accent)),
                        Span::raw(" Edit  "),
                        Span::styled("[d]", Style::default().fg(t.accent)),
                        Span::raw(" Drop  "),
                        Span::styled("[Enter]", Style::default().fg(t.accent)),
                        Span::raw(" Open"),
                    ],
                    FocusPanel::OutputPane => {
                        if state.active_session_id().is_some() {
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
                        Style::default().fg(if running > 0 { t.success } else { t.fg_dim }),
                    ),
                ];

                // Show idle queue if there are waiting sessions
                if idle_count > 0 {
                    status.push(Span::raw(" | "));
                    status.push(Span::styled(
                        format!("{} idle", idle_count),
                        Style::default().fg(t.active).add_modifier(Modifier::BOLD),
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

                // System memory, because it is what predicts death: every
                // abrupt exit happened while the kernel was shooting a
                // process a second. The number shown is swap; the *colour*
                // is the kernel's own pressure verdict — the signal its
                // killer acts on. Swap percent alone cried wolf: idle pages
                // stay swapped on a healthy machine, so it read 91% while
                // free memory sat at 53%. Read per frame — two sysctls,
                // microseconds — so the alarm is never a minute stale.
                if let Some((used, total)) = state.system.perf.system_swap() {
                    if total > 0 {
                        let pct = used as f64 / total as f64 * 100.0;
                        let (style, word) = match state.system.perf.memory_pressure_level() {
                            // The kernel is hunting. Anything can be next.
                            Some(4) => (
                                Style::default().fg(t.error).add_modifier(Modifier::BOLD),
                                " MEM CRITICAL",
                            ),
                            Some(2) => (Style::default().fg(t.active), " mem warn"),
                            _ => (Style::default().fg(t.fg_faint), ""),
                        };
                        status.push(Span::raw(" | "));
                        status.push(Span::styled(format!("swap {pct:.0}%{word}"), style));
                    }
                }

                waiting.unwrap_or((status, context_hints))
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

    let paragraph =
        Paragraph::new(Line::from(spans)).style(Style::default().bg(t.fg_faint).fg(t.fg));

    frame.render_widget(paragraph, area);
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    /// Swap is the number that predicts death on this machine, so the bar
    /// must carry it whenever the kernel will answer the question.
    #[test]
    fn the_bar_reports_system_swap() {
        let state = AppState::default();
        assert!(
            state.system.perf.system_swap().is_some(),
            "macos answers the sysctl"
        );

        let mut terminal = Terminal::new(TestBackend::new(120, 1)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let line: String = (0..120).map(|x| buffer[(x, 0)].symbol().to_string()).collect();
        assert!(line.contains("swap "), "{line}");
        assert!(line.contains('%'), "{line}");
    }
}
