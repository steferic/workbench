use crate::app::{AppState, FocusPanel, InputMode, PendingDelete};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
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
                    .fg(Color::White)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                format!("Delete {} \"{}\"?", item_type, name),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
        ];

        let right_text = vec![
            Span::styled(
                "Press ",
                Style::default().fg(Color::Gray),
            ),
            Span::styled(
                "[d]",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " to confirm, any other key to cancel",
                Style::default().fg(Color::Gray),
            ),
        ];

        let left_len: usize = left_text.iter().map(|s| s.content.len()).sum();
        let right_len: usize = right_text.iter().map(|s| s.content.len()).sum();
        let padding = area.width.saturating_sub(left_len as u16 + right_len as u16 + 2);

        let mut spans = left_text;
        spans.push(Span::raw(" ".repeat(padding as usize)));
        spans.extend(right_text);
        spans.push(Span::raw(" "));

        let paragraph = Paragraph::new(Line::from(spans))
            .style(Style::default().bg(Color::DarkGray).fg(Color::White));

        frame.render_widget(paragraph, area);
        return;
    }

    let (left_text, right_text) = match state.ui.input_mode {
        InputMode::Help => (
            vec![Span::styled(
                " HELP ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )],
            vec![Span::styled(
                "Press Esc or ? to close",
                Style::default().fg(Color::Gray),
            )],
        ),
        InputMode::CreateWorkspace => (
            vec![Span::styled(
                " NEW WORKSPACE ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )],
            vec![Span::styled(
                "Enter path, press Enter to confirm, Esc to cancel",
                Style::default().fg(Color::Gray),
            )],
        ),
        InputMode::CreateSession => (
            vec![Span::styled(
                " NEW SESSION ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )],
            vec![Span::styled(
                "1=Claude 2=Gemini 3=Codex 4=Grok  t=Terminal  Esc=Cancel",
                Style::default().fg(Color::Gray),
            )],
        ),
        InputMode::SetStartCommand => {
            (
                vec![
                    Span::styled(
                        " START COMMAND ",
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" $ "),
                    Span::styled(
                        format!("{}_", state.ui.input_buffer),
                        Style::default().fg(Color::White),
                    ),
                ],
                vec![Span::styled(
                    "Enter command to run on terminal start, Enter to save, Esc to cancel",
                    Style::default().fg(Color::Gray),
                )],
            )
        }
        InputMode::CreateTodo => {
            (
                vec![
                    Span::styled(
                        " NEW TODO ",
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        format!("{}_", state.ui.input_buffer),
                        Style::default().fg(Color::White),
                    ),
                ],
                vec![Span::styled(
                    "Enter todo description, press Enter to create, Esc to cancel",
                    Style::default().fg(Color::Gray),
                )],
            )
        }
        InputMode::SelectWorkspaceAction => (
            vec![Span::styled(
                " ADD WORKSPACE ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )],
            vec![Span::styled(
                "↑/↓ Navigate  Enter to select  Esc to cancel",
                Style::default().fg(Color::Gray),
            )],
        ),
        InputMode::EnterWorkspaceName => (
            vec![
                Span::styled(
                    " CREATE PROJECT ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("{}_", state.ui.input_buffer),
                    Style::default().fg(Color::White),
                ),
            ],
            vec![Span::styled(
                "Enter project name, press Enter to create, Esc to cancel",
                Style::default().fg(Color::Gray),
            )],
        ),
        InputMode::Normal => {
            let context_hints = match state.ui.focus {
                FocusPanel::WorkspaceList => vec![
                    Span::styled("[n]", Style::default().fg(Color::Cyan)),
                    Span::raw(" New  "),
                    Span::styled("[Enter/l]", Style::default().fg(Color::Cyan)),
                    Span::raw(" Sessions  "),
                    Span::styled("[?]", Style::default().fg(Color::Cyan)),
                    Span::raw(" Help  "),
                    Span::styled("[q]", Style::default().fg(Color::Cyan)),
                    Span::raw(" Quit"),
                ],
                FocusPanel::SessionList => vec![
                    Span::styled("[1]", Style::default().fg(Color::Cyan)),
                    Span::raw("Claude "),
                    Span::styled("[2]", Style::default().fg(Color::Cyan)),
                    Span::raw("Gemini "),
                    Span::styled("[3]", Style::default().fg(Color::Cyan)),
                    Span::raw("Codex "),
                    Span::styled("[4]", Style::default().fg(Color::Cyan)),
                    Span::raw("Grok "),
                    Span::styled("[t]", Style::default().fg(Color::Cyan)),
                    Span::raw("Term "),
                    Span::styled("[s]", Style::default().fg(Color::Cyan)),
                    Span::raw("Stop"),
                ],
                FocusPanel::TodosPane => vec![
                    Span::styled("[n]", Style::default().fg(Color::Cyan)),
                    Span::raw(" New  "),
                    Span::styled("[Enter]", Style::default().fg(Color::Cyan)),
                    Span::raw(" Run  "),
                    Span::styled("[x]", Style::default().fg(Color::Cyan)),
                    Span::raw(" Done  "),
                    Span::styled("[a]", Style::default().fg(Color::Cyan)),
                    Span::raw(" Autorun  "),
                    Span::styled("[d]", Style::default().fg(Color::Cyan)),
                    Span::raw(" Del"),
                ],
                FocusPanel::OutputPane => {
                    if state.ui.active_session_id.is_some() {
                        vec![
                            Span::styled("[Esc]", Style::default().fg(Color::Cyan)),
                            Span::raw(" Back  "),
                            Span::styled("[Ctrl+C]", Style::default().fg(Color::Cyan)),
                            Span::raw(" Interrupt  "),
                            Span::styled("Type", Style::default().fg(Color::Yellow)),
                            Span::raw(" to send input"),
                        ]
                    } else {
                        vec![
                            Span::styled("[h/Esc]", Style::default().fg(Color::Cyan)),
                            Span::raw(" Back to sessions"),
                        ]
                    }
                }
                FocusPanel::PinnedTerminalPane(_) => {
                    vec![
                        Span::styled("[Esc]", Style::default().fg(Color::Cyan)),
                        Span::raw(" Back  "),
                        Span::styled("[Tab]", Style::default().fg(Color::Cyan)),
                        Span::raw(" Next pane  "),
                        Span::styled("[Ctrl+C]", Style::default().fg(Color::Cyan)),
                        Span::raw(" Interrupt  "),
                        Span::styled("Type", Style::default().fg(Color::Yellow)),
                        Span::raw(" to send input"),
                    ]
                }
                FocusPanel::UtilitiesPane => {
                    vec![
                        Span::styled("[j/k]", Style::default().fg(Color::Cyan)),
                        Span::raw(" Navigate  "),
                        Span::styled("[Enter]", Style::default().fg(Color::Cyan)),
                        Span::raw(" Select  "),
                        Span::styled("[Tab]", Style::default().fg(Color::Cyan)),
                        Span::raw(" Switch Section  "),
                        Span::styled("[h]", Style::default().fg(Color::Cyan)),
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
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("{} workspaces", state.data.workspaces.len()),
                    Style::default().fg(Color::Gray),
                ),
                Span::raw(" | "),
                Span::styled(
                    format!("{}/{} sessions", running, total_sessions),
                    Style::default().fg(if running > 0 {
                        Color::Green
                    } else {
                        Color::Gray
                    }),
                ),
            ];

            // Show idle queue if there are waiting sessions
            if idle_count > 0 {
                status.push(Span::raw(" | "));
                status.push(Span::styled(
                    format!("{} idle", idle_count),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));
                status.push(Span::styled(
                    " [`]",
                    Style::default().fg(Color::Yellow),
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
        .style(Style::default().bg(Color::DarkGray).fg(Color::White));

    frame.render_widget(paragraph, area);
}
