//! The desk as a full panel.
//!
//! Everything waiting on you, from every project, with the context each
//! decision needs — in the space a decision deserves. Rows come from
//! `app::desk_view::rows`, the same list the phone shows, so the two cannot
//! disagree about what needs you or in what order.
//!
//! It takes the whole right-hand panel, where the terminal usually is. A
//! proposal is paragraphs, not a line, and reading paragraphs through a
//! forty-column slot beside the sessions list was how the desk went unread.

use crate::app::desk_view::{self, DeskRow};
use crate::app::{AppState, FocusPanel};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::tasks_pane::wrapped;

/// The leading column: a cursor marker, then the kind of decision.
const TAG_WIDTH: usize = 11;

pub fn render(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let t = crate::theme::current();
    let is_focused = state.ui.focus == FocusPanel::OutputPane;
    let border_style = if is_focused {
        Style::default().fg(t.border_focused)
    } else {
        Style::default().fg(t.border)
    };

    let rows = desk_view::rows(state);
    let title = if rows.is_empty() {
        " DESK ".to_string()
    } else {
        format!(" DESK · {} waiting ", rows.len())
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    let (body, footer) = (chunks[0], chunks[1]);
    render_footer(frame, footer, state, is_focused);

    if rows.is_empty() {
        state.ui.desk_scroll = 0;
        let faint = Style::default().fg(t.fg_faint);
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  Nothing needs you.",
                    Style::default().fg(t.success),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "  Agents stopped on a question, reviews a manager handed back,",
                    faint,
                )),
                Line::from(Span::styled(
                    "  proposals awaiting a yes, and checks nobody has approved all",
                    faint,
                )),
                Line::from(Span::styled(
                    "  land here, most urgent first, from every project.",
                    faint,
                )),
            ]),
            body,
        );
        return;
    }

    let selected = state.ui.selected_desk_row.min(rows.len() - 1);
    let width = body.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut selected_span = (0usize, 0usize);
    for (i, row) in rows.iter().enumerate() {
        let on_cursor = is_focused && i == selected;
        let start = lines.len();
        card(state, row, width, on_cursor, &mut lines);
        lines.push(Line::from(""));
        if i == selected {
            selected_span = (start, lines.len());
        }
    }

    // Scroll only as far as it takes to keep the whole selected card on
    // screen, so reading down the list does not jump. A card taller than the
    // panel shows its head.
    let height = body.height as usize;
    let (top, end) = selected_span;
    let mut offset = state.ui.desk_scroll as usize;
    if end > offset + height {
        offset = end.saturating_sub(height);
    }
    if top < offset {
        offset = top;
    }
    offset = offset.min(lines.len().saturating_sub(height));
    state.ui.desk_scroll = offset as u16;
    frame.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), body);
}

/// One row, as a card: a head line naming the kind of decision and the
/// project, then the text the decision needs, wrapped in full. Nothing is
/// elided — a job whose tail is an ellipsis is a job you approve on faith.
fn card(
    state: &AppState,
    row: &DeskRow,
    width: usize,
    on_cursor: bool,
    lines: &mut Vec<Line<'static>>,
) {
    let t = crate::theme::current();
    let head_style = if on_cursor {
        Style::default().fg(t.fg).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.fg)
    };
    let text_width = width.saturating_sub(TAG_WIDTH);
    let marker = if on_cursor { "> " } else { "  " };

    let mut head = |tag: &str, color: Color, project: &str, headline: String| {
        let lead = format!("{marker}{tag:<w$}", w = TAG_WIDTH - 2);
        for (i, piece) in wrapped(&format!("{project}  {headline}"), text_width)
            .into_iter()
            .enumerate()
        {
            let mut spans = Vec::new();
            if i == 0 {
                spans.push(Span::styled(lead.clone(), Style::default().fg(color)));
            } else {
                spans.push(Span::raw(" ".repeat(TAG_WIDTH)));
            }
            spans.push(Span::styled(piece, head_style));
            lines.push(Line::from(spans));
        }
    };

    match row {
        DeskRow::BlockedAgent {
            session_id,
            project,
        } => {
            let name = state
                .get_session(*session_id)
                .map(|s| format!("{} {}", s.display_name(), s.short_id()))
                .unwrap_or_else(|| "an agent".into());
            let why = state
                .activity_reason(*session_id)
                .filter(|reason| !reason.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| "stopped on a question".into());
            head("blocked", t.warning, project, name);
            text(lines, None, &why, t.fg, text_width);
        }
        DeskRow::PendingProposal {
            workspace_id,
            proposal_id,
            project,
        } => {
            let Some((proposal, objective)) =
                proposal_with_objective(state, *workspace_id, *proposal_id)
            else {
                return;
            };
            let agent = proposal
                .agent
                .clone()
                .map(|a| format!("agent {a}"))
                .unwrap_or_else(|| "whoever is free first".into());
            head(
                "approve?",
                t.info,
                project,
                format!(
                    "proposed {} by manager {} · for {agent}",
                    age(proposal.created_at),
                    proposal.manager
                ),
            );
            text(lines, None, &proposal.instruction, t.fg, text_width);
            text(
                lines,
                Some("why: "),
                &proposal.rationale,
                t.fg_dim,
                text_width,
            );
            text(lines, Some("toward: "), &objective, t.fg_faint, text_width);
        }
        DeskRow::NeedsUser {
            workspace_id,
            proposal_id,
            project,
        } => {
            let Some((proposal, objective)) =
                proposal_with_objective(state, *workspace_id, *proposal_id)
            else {
                return;
            };
            let agent = proposal.agent.clone().unwrap_or_else(|| "(none)".into());
            head(
                "on you",
                t.warning,
                project,
                format!(
                    "handed back by manager {} after round {} · agent {agent}",
                    proposal.manager, proposal.review_rounds
                ),
            );
            text(lines, None, &proposal.instruction, t.fg, text_width);
            if let Some(findings) = &proposal.findings {
                text(lines, Some("findings: "), findings, t.warning, text_width);
            }
            if let Some(verdict) = &proposal.verdict {
                text(
                    lines,
                    Some("check: "),
                    &format!("{} — {}", verdict.label(), verdict.why()),
                    t.fg_dim,
                    text_width,
                );
            }
            text(lines, Some("toward: "), &objective, t.fg_faint, text_width);
        }
        DeskRow::ProposedCheck {
            workspace_id,
            objective_id,
            project,
        } => {
            let Some(objective) = state
                .data
                .workspaces
                .iter()
                .find(|ws| ws.id == *workspace_id)
                .and_then(|ws| ws.objectives.iter().find(|o| o.id == *objective_id))
            else {
                return;
            };
            let command = objective
                .done_when
                .as_ref()
                .map(|check| check.command.clone())
                .unwrap_or_default();
            head(
                "check?",
                t.info,
                project,
                "a manager proposed how this objective is checked".into(),
            );
            text(
                lines,
                Some("objective: "),
                &objective.text,
                t.fg,
                text_width,
            );
            text(lines, Some("run: "), &command, t.warning, text_width);
            text(
                lines,
                None,
                "Approve to let work be held to it. Exit 0 is the only pass.",
                t.fg_faint,
                text_width,
            );
        }
    }
}

/// A block of text under the head line, wrapped and indented, with an
/// optional label leading its first line. Empty text draws nothing.
fn text(
    lines: &mut Vec<Line<'static>>,
    label: Option<&str>,
    body: &str,
    color: Color,
    width: usize,
) {
    let body = body.trim();
    if body.is_empty() {
        return;
    }
    let full = match label {
        Some(label) => format!("{label}{body}"),
        None => body.to_string(),
    };
    for piece in wrapped(&full, width) {
        lines.push(Line::from(vec![
            Span::raw(" ".repeat(TAG_WIDTH)),
            Span::styled(piece, Style::default().fg(color)),
        ]));
    }
}

/// The proposal a row points at, and the text of the objective it serves.
fn proposal_with_objective(
    state: &AppState,
    workspace_id: uuid::Uuid,
    proposal_id: uuid::Uuid,
) -> Option<(crate::models::Proposal, String)> {
    let workspace = state
        .data
        .workspaces
        .iter()
        .find(|ws| ws.id == workspace_id)?;
    let proposal = workspace.proposals.iter().find(|p| p.id == proposal_id)?;
    let objective = proposal
        .objective_id
        .and_then(|id| workspace.objectives.iter().find(|o| o.id == id))
        .map(|o| o.text.clone())
        .unwrap_or_default();
    Some((proposal.clone(), objective))
}

fn age(at: chrono::DateTime<chrono::Utc>) -> String {
    let mins = (chrono::Utc::now() - at).num_minutes();
    if mins < 1 {
        "just now".to_string()
    } else if mins < 60 {
        format!("{mins}m ago")
    } else if mins < 60 * 48 {
        format!("{}h ago", mins / 60)
    } else {
        format!("{}d ago", mins / (60 * 24))
    }
}

/// The keys, or the outcome of the last one pressed — that outranks the
/// hints, and this is where the user is already looking.
fn render_footer(frame: &mut Frame, area: Rect, state: &AppState, is_focused: bool) {
    let t = crate::theme::current();
    if let Some(status) = state.ui.task_status() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(" {status}"),
                Style::default().fg(t.info),
            ))),
            area,
        );
        return;
    }
    let action_style = if is_focused {
        Style::default().fg(t.fg_faint)
    } else {
        Style::default().fg(t.inactive)
    };
    let key_style = if is_focused {
        Style::default().fg(t.accent)
    } else {
        Style::default().fg(t.fg_faint)
    };
    let hints: &[(&str, &str)] = &[
        (" a", ":yes  "),
        ("x", ":no  "),
        ("Enter", ":go to  "),
        ("j/k", ":move  "),
        ("Esc", ":close"),
    ];
    let spans: Vec<Span> = hints
        .iter()
        .flat_map(|(key, label)| {
            [
                Span::styled(*key, key_style),
                Span::styled(*label, action_style),
            ]
        })
        .collect();
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AgentType, Proposal, Session, Workspace};
    use ratatui::{backend::TestBackend, Terminal};

    fn screen(state: &mut AppState, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), state))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// One project with one agent, the desk open and focused, and the given
    /// proposals pending from a manager "m1".
    fn desk_with(proposals: &[(&str, &str)]) -> AppState {
        let mut state = AppState::default();
        let mut ws = Workspace::new("alpha".into(), std::path::PathBuf::from("/tmp/alpha"));
        let ws_id = ws.id;
        for (instruction, rationale) in proposals {
            let mut proposal = Proposal::new("m1", *instruction);
            proposal.rationale = rationale.to_string();
            ws.proposals.push(proposal);
        }
        state.data.workspaces.push(ws);
        state
            .data
            .sessions
            .insert(ws_id, vec![Session::new(ws_id, AgentType::Claude, false)]);
        state.ui.desk_open = true;
        state.ui.focus = FocusPanel::OutputPane;
        state
    }

    #[test]
    fn an_empty_desk_says_so() {
        let mut state = desk_with(&[]);
        let out = screen(&mut state, 80, 12);
        assert!(out.contains(" DESK "), "{out}");
        assert!(out.contains("Nothing needs you."), "{out}");
    }

    /// The whole point of the panel: the instruction and the reasoning are
    /// readable in place, with the project named, and nothing cut short.
    #[test]
    fn a_proposal_is_shown_in_full_with_its_project_and_reasoning() {
        let mut state = desk_with(&[(
            "split the auth module into a session store and a token verifier so the two can be tested apart",
            "the module is the one place every failing test in the last week has pointed at",
        )]);
        let out = screen(&mut state, 90, 16);
        assert!(out.contains("DESK · 1 waiting"), "{out}");
        assert!(
            out.contains("> approve?"),
            "the cursor sits on the first card:\n{out}"
        );
        assert!(out.contains("alpha"), "{out}");
        assert!(out.contains("by manager m1"), "{out}");
        // Read the panel as prose: borders out, lines joined.
        let flat = out.replace(['\n', '│'], " ");
        let squeezed = flat.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            squeezed.contains("so the two can be tested apart"),
            "the instruction survives to its last word:\n{out}"
        );
        assert!(
            squeezed.contains("why: the module is the one place"),
            "the rationale is right under it:\n{out}"
        );
    }

    /// Moving down a long desk scrolls the panel rather than losing the
    /// cursor below the fold.
    #[test]
    fn the_selected_card_is_kept_on_screen() {
        let proposals: Vec<(String, String)> = (1..=6)
            .map(|n| {
                (
                    format!("job number {n} which is long enough to wrap onto more than one line at this width"),
                    format!("reason {n}"),
                )
            })
            .collect();
        let borrowed: Vec<(&str, &str)> = proposals
            .iter()
            .map(|(i, r)| (i.as_str(), r.as_str()))
            .collect();
        let mut state = desk_with(&borrowed);

        state.ui.selected_desk_row = 5;
        let out = screen(&mut state, 60, 12);
        assert!(out.contains("job number 6"), "{out}");
        assert!(
            !out.contains("job number 1 "),
            "the top scrolled away:\n{out}"
        );

        // Coming back up scrolls back, and does not jump past the card.
        state.ui.selected_desk_row = 0;
        let out = screen(&mut state, 60, 12);
        assert!(out.contains("> approve?"), "{out}");
        assert!(out.contains("job number 1"), "{out}");
    }
}
