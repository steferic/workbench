use crate::app::{servers, Action, AppState};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

pub fn render(frame: &mut Frame, area: Rect, actions: Rect, state: &mut AppState) {
    let t = crate::theme::current();
    let rows = servers::rows(state);
    let scope = if state.ui.servers.current_project {
        "This project"
    } else {
        "All projects"
    };
    let title = format!("{scope} · {}", rows.len());
    let header_rows = u16::from(area.height >= 4);
    let message_rows = u16::from(area.height >= 5);
    let scope_area = Rect::new(area.x, area.y, area.width, header_rows);
    frame.render_widget(
        Paragraph::new(title).style(Style::default().fg(t.accent)),
        scope_area,
    );
    state.ui.click_hits.push((scope_area, Action::ServerScope));
    let body = Rect::new(
        area.x,
        area.y + header_rows,
        area.width,
        area.height.saturating_sub(header_rows + message_rows),
    );
    if rows.is_empty() {
        let text = if state.system.port_scan_inflight {
            "Scanning…"
        } else if state.ui.servers.scan_error.is_some() {
            "Scan failed · r retry"
        } else {
            "No servers running"
        };
        frame.render_widget(
            Paragraph::new(text).style(Style::default().fg(t.fg_dim)),
            body,
        );
    } else {
        let row_height = body.height.min(2).max(1);
        let capacity = (body.height as usize / row_height as usize).max(1);
        let selected = rows
            .iter()
            .position(|r| Some(r.server.key()) == state.ui.servers.selected)
            .unwrap_or(0);
        let offset = &mut state.ui.servers.offset;
        *offset = (*offset).min(selected);
        if selected >= *offset + capacity {
            *offset = selected + 1 - capacity;
        }
        *offset = (*offset).min(rows.len().saturating_sub(capacity));
        for (visible, (index, row)) in rows
            .iter()
            .enumerate()
            .skip(*offset)
            .take(capacity)
            .enumerate()
        {
            let y = body.y + visible as u16 * row_height;
            let rect = Rect::new(
                body.x,
                y,
                body.width,
                body.bottom().saturating_sub(y).min(row_height),
            );
            if rect.height == 0 {
                continue;
            }
            let stopping = state.ui.servers.is_stopping(&row.server);
            let selected_style = if index == selected {
                Style::default()
                    .bg(t.selection_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let marker = if stopping { "…" } else { "●" };
            let text = vec![
                Line::from(vec![
                    Span::styled(
                        format!("{marker} :{} ", row.server.port),
                        Style::default().fg(if stopping { t.warning } else { t.success }),
                    ),
                    Span::raw(clean(&row.server.command)),
                ]),
                Line::from(Span::styled(
                    format!("  {} · PID {}", clean(&row.name), row.server.pid),
                    Style::default().fg(t.fg_dim),
                )),
            ];
            frame.render_widget(Paragraph::new(text).style(selected_style), rect);
            state.ui.click_hits.push((rect, Action::SelectServer(row.server.key())));
        }
    }
    let message = state
        .ui
        .servers
        .scan_error
        .as_deref()
        .or(state.ui.servers.message.as_deref())
        .unwrap_or("↑↓ select · Enter details");
    if message_rows > 0 {
        frame.render_widget(
            Paragraph::new(clean(message)).style(Style::default().fg(t.fg_dim)),
            Rect::new(area.x, area.bottom() - 1, area.width, 1),
        );
    }
    let first = Rect::new(actions.x, actions.y, actions.width, actions.height.min(1));
    buttons(
        frame,
        first,
        state,
        &[
            ("o Open", Action::ServerOpen),
            ("x Stop", Action::ServerAskStop),
        ],
    );
    if actions.height > 1 {
        buttons(
            frame,
            Rect::new(actions.x, actions.y + 1, actions.width, 1),
            state,
            &[
                ("a Scope", Action::ServerScope),
                ("r Refresh", Action::ServerRefresh),
            ],
        );
    }
}

fn buttons(frame: &mut Frame, area: Rect, state: &mut AppState, buttons: &[(&str, Action)]) {
    if buttons.is_empty() || area.height == 0 {
        return;
    }
    let t = crate::theme::current();
    let width = area.width / buttons.len() as u16;
    for (i, (label, action)) in buttons.iter().enumerate() {
        let rect = Rect::new(area.x + i as u16 * width, area.y, width, area.height);
        frame.render_widget(
            Paragraph::new(*label).style(Style::default().fg(t.accent)),
            rect,
        );
        state.ui.click_hits.push((rect, action.clone()));
    }
}

pub fn dialog(frame: &mut Frame, state: &mut AppState) {
    let Some(dialog) = state.ui.servers.dialog.clone() else {
        return;
    };
    let t = crate::theme::current();
    let full = frame.area();
    let width = full.width.saturating_sub(4).min(76);
    let height = full.height.saturating_sub(2).min(17);
    let area = Rect::new(
        full.x + (full.width - width) / 2,
        full.y + (full.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(if dialog.confirming {
            " Stop server? "
        } else {
            " Server details "
        })
        .style(Style::default().bg(t.bg).fg(t.fg));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let server = &dialog.row.server;
    let ports: Vec<_> = state
        .system
        .dev_servers
        .iter()
        .filter(|s| s.pid == server.pid && s.start == server.start)
        .map(|s| s.port.to_string())
        .collect();
    let status = if state.ui.servers.is_stopping(server) {
        "Stopping"
    } else if state
        .system
        .dev_servers
        .iter()
        .any(|s| s.key() == server.key())
    {
        "Listening"
    } else {
        "Exited or changed"
    };
    let mut lines = vec![
        Line::from(format!("Project: {}", clean(&dialog.row.name))),
        Line::from(format!(
            "Process: {} · PID {} · {status}",
            clean(&server.command),
            server.pid
        )),
        Line::from(format!("URL: {}", server.url())),
        Line::from(format!("Ports for this process: {}", ports.join(", "))),
        Line::from(format!(
            "Directory: {}",
            clean(&server.cwd.display().to_string())
        )),
        Line::from(""),
    ];
    if dialog.confirming {
        lines.push(Line::from(
            "Stop this process and its children? All its ports will close.",
        ));
        lines.push(Line::from(
            "Workbench asks it to exit, then force-stops it if necessary.",
        ));
    } else {
        lines.push(Line::from(
            "A supervisor may restart a stopped server; it will reappear here.",
        ));
        if let Some(message) = &state.ui.servers.message {
            lines.push(Line::from(clean(message)));
        }
    }
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        Rect::new(
            inner.x,
            inner.y,
            inner.width,
            inner.height.saturating_sub(2),
        ),
    );
    if inner.height > 0 {
        let footer = Rect::new(inner.x, inner.bottom() - 1, inner.width, 1);
        if dialog.confirming {
            buttons(
                frame,
                footer,
                state,
                &[
                    ("Enter Stop", Action::ServerConfirmStop),
                    ("Esc Cancel", Action::ServerClose),
                ],
            );
        } else {
            buttons(
                frame,
                footer,
                state,
                &[
                    ("o Open", Action::ServerOpen),
                    ("x Stop", Action::ServerAskStop),
                    ("Esc Close", Action::ServerClose),
                ],
            );
        }
    }
}
