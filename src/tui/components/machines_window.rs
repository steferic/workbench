//! The Machines window: other Macs' agents on the left, the selected one's
//! conversation on the right (see `app::machines`).

use crate::app::machines::{self, Row};
use crate::app::AppState;
use crate::remote::peers;
use crate::remote::Role;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

pub fn render(frame: &mut Frame, state: &AppState) {
    let t = crate::theme::current();
    let full = frame.area();
    let area = if full.width < 100 || full.height < 30 {
        full
    } else {
        Rect {
            x: full.x + full.width / 20,
            y: full.y + full.height / 20,
            width: full.width - full.width / 10,
            height: full.height - full.height / 10,
        }
    };
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(" Machines ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border_focused))
        .style(Style::default().bg(t.bg).fg(t.fg));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 4 || inner.width < 30 {
        return;
    }

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(36.min(inner.width / 3)),
            Constraint::Min(20),
        ])
        .split(inner);

    let selected = machines::selected(state);
    render_list(frame, columns[0], selected.as_ref());
    render_conversation(frame, columns[1], state, selected.as_ref());
}

fn render_list(frame: &mut Frame, area: Rect, selected: Option<&Row>) {
    let t = crate::theme::current();
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_line = 0;
    let rows = machines::rows();
    let Some(peers) = peers::get() else {
        frame.render_widget(
            Paragraph::new("No other Macs configured.").style(Style::default().fg(t.fg_dim)),
            area,
        );
        return;
    };
    for machine in peers.machines() {
        let name = &machine.config.name;
        let health = match (&machine.error, machine.seen) {
            (Some(err), _) => Span::styled(
                format!("  {}", short(err, 40)),
                Style::default().fg(t.warning),
            ),
            (None, None) => Span::styled("  connecting…", Style::default().fg(t.fg_dim)),
            (None, Some(_)) => Span::raw(""),
        };
        lines.push(Line::from(vec![
            Span::styled(
                name.clone(),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            health,
        ]));
        let mut project = String::new();
        for row in rows.iter().filter(|r| &r.machine == name) {
            if row.agent.project != project {
                project = row.agent.project.clone();
                lines.push(Line::styled(
                    format!(" {project}"),
                    Style::default().fg(t.fg_dim),
                ));
            }
            let is_selected = selected.is_some_and(|s| s.key() == row.key());
            if is_selected {
                selected_line = lines.len();
            }
            let label = row
                .agent
                .alias
                .clone()
                .unwrap_or_else(|| row.agent.provider.clone());
            let status = status_span(&row.agent.status, row.agent.prompt.is_some());
            let style = if is_selected {
                Style::default()
                    .bg(t.selection_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            lines.push(Line::from(vec![
                Span::styled(format!("   {label} "), style),
                Span::styled(
                    format!("{} ", short(&row.agent.id, 8)),
                    Style::default().fg(t.fg_faint),
                ),
                status,
            ]));
        }
        lines.push(Line::raw(""));
    }
    let height = area.height as usize;
    let skip = selected_line.saturating_sub(height.saturating_sub(2));
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(t.border));
    frame.render_widget(
        Paragraph::new(lines.into_iter().skip(skip).collect::<Vec<_>>()).block(block),
        area,
    );
}

fn status_span(status: &str, asking: bool) -> Span<'static> {
    let t = crate::theme::current();
    let (text, color) = match (status, asking) {
        (_, true) | ("blocked", _) => ("needs you", t.warning),
        ("working", _) => ("working", t.active),
        ("idle", _) => ("idle", t.fg_dim),
        (other, _) => (if other == "stopped" { "stopped" } else { "?" }, t.fg_faint),
    };
    Span::styled(text.to_string(), Style::default().fg(color))
}

fn render_conversation(frame: &mut Frame, area: Rect, state: &AppState, row: Option<&Row>) {
    let t = crate::theme::current();
    let area = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(1),
        ..area
    };
    let Some(row) = row else {
        let text = state
            .ui
            .machines
            .message
            .clone()
            .unwrap_or_else(|| "No agents on your other Macs yet.".into());
        frame.render_widget(
            Paragraph::new(text).style(Style::default().fg(t.fg_dim)),
            area,
        );
        return;
    };
    let agent = &row.agent;
    let ui = &state.ui.machines;

    // Foot: the question the agent is stopped on, then the reply line.
    let mut foot: Vec<Line> = Vec::new();
    if let Some(prompt) = &agent.prompt {
        for line in prompt.lines.iter().rev().take(4).rev() {
            foot.push(Line::styled(line.clone(), Style::default().fg(t.fg)));
        }
        for option in &prompt.options {
            foot.push(Line::from(vec![
                Span::styled(
                    format!(" {} ", option.key),
                    Style::default().fg(t.on_accent).bg(t.accent),
                ),
                Span::raw(format!(" {}", option.label)),
            ]));
        }
    }
    let note = if ui.confirm_kill {
        Line::styled(
            format!(
                "Stop {} on {}? y to stop, any other key to keep it.",
                label(row),
                row.machine
            ),
            Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
        )
    } else if ui.typing {
        Line::from(vec![
            Span::styled(
                if ui.file { "file › " } else { "› " },
                Style::default().fg(t.accent),
            ),
            Span::raw(ui.draft.clone()),
            Span::styled("▏", Style::default().fg(t.accent)),
        ])
    } else {
        Line::styled(
            ui.message
                .clone()
                .unwrap_or_else(|| "Enter to reply · f to send files.".into()),
            Style::default().fg(t.fg_dim),
        )
    };
    foot.push(note);

    let header = Line::from(vec![
        Span::styled(label(row), Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            format!(
                "  {} · {} · {}",
                agent.project,
                agent
                    .model
                    .clone()
                    .unwrap_or_else(|| agent.provider.clone()),
                row.machine
            ),
            Style::default().fg(t.fg_dim),
        ),
    ]);

    let foot_height = (foot.len() as u16).min(area.height / 2);
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(foot_height),
        ])
        .split(area);
    frame.render_widget(Paragraph::new(vec![header, Line::raw("")]), parts[0]);

    // The conversation, wrapped here so the end can be pinned to the bottom.
    let width = parts[1].width.max(10) as usize;
    let mut lines: Vec<Line> = Vec::new();
    if agent.messages.is_empty() && agent.tail.is_empty() {
        let text = if row.loaded {
            "No conversation yet."
        } else {
            "Loading the conversation…"
        };
        lines.push(Line::styled(text, Style::default().fg(t.fg_dim)));
    }
    for message in &agent.messages {
        let (prefix, style) = match message.role {
            Role::You => ("you  ", Style::default().fg(t.accent)),
            Role::Agent => ("     ", Style::default().fg(t.fg)),
            Role::Tool => ("  ▸  ", Style::default().fg(t.fg_faint)),
        };
        let text = if message.role == Role::Tool {
            short(&message.text, width.saturating_sub(6))
        } else {
            message.text.clone()
        };
        for (i, line) in wrap(&text, width.saturating_sub(5)).into_iter().enumerate() {
            let lead = if i == 0 { prefix } else { "     " };
            lines.push(Line::from(vec![
                Span::styled(lead.to_string(), style),
                Span::styled(line, style),
            ]));
        }
        if message.role != Role::Tool {
            lines.push(Line::raw(""));
        }
    }
    if agent.messages.is_empty() {
        for line in &agent.tail {
            lines.push(Line::styled(line.clone(), Style::default().fg(t.fg_dim)));
        }
    }
    let height = parts[1].height as usize;
    let end = lines
        .len()
        .saturating_sub(ui.scroll as usize)
        .max(height.min(lines.len()));
    let start = end.saturating_sub(height);
    frame.render_widget(Paragraph::new(lines[start..end].to_vec()), parts[1]);
    frame.render_widget(Paragraph::new(foot), parts[2]);
}

fn label(row: &Row) -> String {
    row.agent
        .alias
        .clone()
        .unwrap_or_else(|| format!("{} {}", row.agent.provider, short(&row.agent.id, 8)))
}

fn short(text: &str, max: usize) -> String {
    let flat = text.replace('\n', " ");
    if flat.chars().count() <= max {
        return flat;
    }
    flat.chars()
        .take(max.saturating_sub(1))
        .chain(['…'])
        .collect()
}

/// Greedy word wrap by display width; a word longer than the line is cut.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut out = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let candidate = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if candidate.width() <= width {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
            }
            let mut rest = word.to_string();
            while rest.width() > width {
                let cut: String = rest.chars().take(width).collect();
                rest = rest.chars().skip(width).collect();
                out.push(cut);
            }
            line = rest;
        }
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping_keeps_words_whole_and_cuts_only_long_ones() {
        assert_eq!(wrap("one two three", 8), vec!["one two", "three"]);
        assert_eq!(wrap("abcdefghij", 8), vec!["abcdefgh", "ij"]);
        assert_eq!(wrap("a\n\nb", 8), vec!["a", "", "b"]);
    }
}
