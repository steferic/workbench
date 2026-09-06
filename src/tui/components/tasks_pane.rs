//! Objectives pane — the project's standing priorities, and the proposals
//! made against them.
//!
//! Rows come from `app::objectives_view` so navigation and rendering can never
//! disagree about what is on screen. The line above the list says what the
//! desk holds; the desk itself lives in the right-hand panel (F3, or D from
//! any left pane), because it is paragraphs and this pane is forty columns.

use crate::app::{AppState, FocusPanel};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let t = crate::theme::current();
    let is_focused = state.ui.focus == FocusPanel::TasksPane;
    let border_style = if is_focused {
        Style::default().fg(t.border_focused)
    } else {
        Style::default().fg(t.border)
    };

    let block = Block::default()
        .title(Line::from(vec![Span::raw(" OBJECTIVES ")]))
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner_area);
    let (desk_area, list_area, action_area) = (chunks[0], chunks[1], chunks[2]);

    render_desk_line(frame, desk_area, state, is_focused);
    render_action_bar(frame, action_area, state, is_focused);
    render_objectives_tab(frame, list_area, state, is_focused);
}

/// What the desk holds, in one line, and the key that opens it. A count is
/// the most this pane can afford to say about decisions; the decisions
/// themselves need the room the right-hand panel has.
fn render_desk_line(frame: &mut Frame, area: Rect, state: &AppState, is_focused: bool) {
    let t = crate::theme::current();
    let waiting = crate::app::desk_view::rows(state).len();
    let key = state
        .system
        .user_config
        .global_hotkeys
        .get("ToggleDesk")
        .filter(|k| !k.is_empty())
        .cloned()
        .unwrap_or_else(|| "D".to_string());
    let label_style = Style::default().fg(if is_focused { t.fg_dim } else { t.fg_faint });
    let key_style = Style::default().fg(if is_focused { t.accent } else { t.fg_faint });
    let spans = if waiting == 0 {
        vec![
            Span::styled(" Desk: ", label_style),
            Span::styled("nothing needs you", Style::default().fg(t.fg_faint)),
        ]
    } else {
        vec![
            Span::styled(" Desk: ", label_style),
            Span::styled(
                format!("{waiting} waiting"),
                Style::default().fg(t.active).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ", label_style),
            Span::styled(key, key_style),
            Span::styled(" opens it", label_style),
        ]
    };
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The project's standing priorities, in priority order.
///
/// Rank is position, so the top line is what matters most — which is the only
/// thing a manager will need to read off this list. State is shown as a word
/// rather than a colour alone, because "held" and "met" are decisions worth
/// spelling out.
fn render_objectives_tab(frame: &mut Frame, area: Rect, state: &mut AppState, is_focused: bool) {
    use crate::models::ObjectiveState;

    let t = crate::theme::current();
    let Some(workspace) = state.selected_workspace() else {
        let msg = Paragraph::new(Line::from(Span::styled(
            "  Open a project first.",
            Style::default().fg(t.fg_faint),
        )));
        frame.render_widget(msg, area);
        return;
    };

    if workspace.objectives.is_empty() {
        let msg = Paragraph::new(vec![
            Line::from(Span::styled(
                "  No objectives yet. Press n to write one.",
                Style::default().fg(t.fg_faint),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Standing priorities for this project — what",
                Style::default().fg(t.fg_faint),
            )),
            Line::from(Span::styled(
                "  work should keep moving toward.",
                Style::default().fg(t.fg_faint),
            )),
        ]);
        frame.render_widget(msg, area);
        return;
    }

    // Drawn from the same row list the keys walk, so the highlighted line and
    // the line a keypress acts on can never drift apart. Text is wrapped by
    // hand rather than by ratatui: knowing exactly how many screen lines each
    // row takes is what lets the view scroll to the cursor, and a hanging
    // indent keeps a wrapped objective reading as one item rather than three.
    let width = area.width as usize;
    let rows = crate::app::objectives_view::rows(state);
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_span = (0usize, 0usize);

    for (row_index, row) in rows.iter().enumerate() {
        let selected = is_focused && row_index == state.ui.selected_objective;
        let marker = if selected { "> " } else { "  " };
        let row_start = lines.len();

        match row {
            crate::app::objectives_view::ObjectiveRow::Objective { id, index } => {
                let Some(objective) = workspace.objectives.iter().find(|o| o.id == *id) else {
                    continue;
                };
                let (state_label, state_color) = match objective.state {
                    ObjectiveState::Active => ("", t.fg),
                    ObjectiveState::Held => ("held ", t.fg_faint),
                    ObjectiveState::Met => ("met ", t.success),
                };
                let text_style = match objective.state {
                    ObjectiveState::Active if selected => {
                        Style::default().fg(t.fg).add_modifier(Modifier::BOLD)
                    }
                    ObjectiveState::Active => Style::default().fg(t.fg),
                    _ => Style::default().fg(t.fg_faint),
                };

                let number = format!("{}. ", index + 1);
                let indent = marker.len() + number.len() + state_label.len();
                for (i, piece) in wrapped(&objective.text, width.saturating_sub(indent))
                    .into_iter()
                    .enumerate()
                {
                    let mut spans = Vec::new();
                    if i == 0 {
                        spans.push(Span::styled(marker, Style::default().fg(t.accent)));
                        spans.push(Span::styled(
                            number.clone(),
                            Style::default().fg(t.fg_faint),
                        ));
                        if !state_label.is_empty() {
                            spans.push(Span::styled(
                                state_label,
                                Style::default().fg(state_color),
                            ));
                        }
                    } else {
                        spans.push(Span::raw(" ".repeat(indent)));
                    }
                    spans.push(Span::styled(piece, text_style));
                    lines.push(Line::from(spans));
                }
                // The morning question, answered under the priority itself:
                // is this moving, and at what burn.
                let ledger =
                    crate::models::objective_ledger(&workspace.proposals, objective.id);
                if ledger != crate::models::ObjectiveLedger::default() {
                    let mut bits: Vec<String> = Vec::new();
                    if ledger.resolved_this_week > 0 {
                        bits.push(format!("{} resolved this wk", ledger.resolved_this_week));
                    }
                    if ledger.in_flight > 0 {
                        bits.push(format!("{} in flight", ledger.in_flight));
                    }
                    if ledger.needs_user > 0 {
                        bits.push(format!("{} on you", ledger.needs_user));
                    }
                    if ledger.agent_turns > 0 {
                        bits.push(format!(
                            "≈{} agent turns, {} reviews",
                            ledger.agent_turns, ledger.reviews
                        ));
                    }
                    if let Some(at) = ledger.last_activity {
                        let mins = (chrono::Utc::now() - at).num_minutes();
                        bits.push(if mins < 60 {
                            format!("active {mins}m ago")
                        } else if mins < 60 * 48 {
                            format!("active {}h ago", mins / 60)
                        } else {
                            format!("active {}d ago", mins / (60 * 24))
                        });
                    }
                    lines.push(Line::from(Span::styled(
                        format!("{}{}", " ".repeat(indent), bits.join(" · ")),
                        Style::default().fg(t.fg_faint),
                    )));
                }
                if let Some(check) = objective.done_when.as_ref() {
                    let (mark, color) = if check.proposed {
                        ("?", t.warning)
                    } else {
                        ("✓", t.success)
                    };
                    for (i, piece) in wrapped(&check.command, width.saturating_sub(indent + 1))
                        .into_iter()
                        .enumerate()
                    {
                        let lead = if i == 0 {
                            format!("{}{mark}", " ".repeat(indent))
                        } else {
                            " ".repeat(indent + 1)
                        };
                        lines.push(Line::from(vec![
                            Span::styled(lead, Style::default().fg(color)),
                            Span::styled(piece, Style::default().fg(color)),
                        ]));
                    }
                }
            }
            crate::app::objectives_view::ObjectiveRow::Proposal { id } => {
                let Some(proposal) = workspace.proposals.iter().find(|p| p.id == *id) else {
                    continue;
                };
                // Approved ones stay on the list, marked, until the queue
                // is done with them: seeing that a suggestion became work
                // is most of what this view is for.
                // Once a check has spoken, its verdict is the headline:
                // that is the fact worth reading, and the only one a
                // manager could not have written itself.
                // The lifecycle outranks the check's verdict once review has
                // spoken: "resolved" and "needs you" are decisions, a verdict
                // is evidence. Mid-flight, the verdict is the best headline.
                let verb = proposal_verb(proposal);
                let verb_color = match verb {
                    "resolved " | "verified " => t.success,
                    "needs you " | "unclear " => t.warning,
                    "rejected " => t.error,
                    "closed " => t.fg_dim,
                    _ => t.info,
                };
                let body_style = if selected {
                    Style::default().fg(t.fg)
                } else {
                    Style::default().fg(t.fg_dim)
                };
                let agent = proposal.agent.clone().unwrap_or_else(|| {
                    if proposal.open_on_board() { "anyone" } else { "nobody" }.into()
                });
                let head = format!("   {marker}");
                let indent = head.len() + verb.len();
                // The instruction in full — a job whose tail is an ellipsis
                // is a job you approve on faith.
                for (i, piece) in wrapped(&proposal.instruction, width.saturating_sub(indent))
                    .into_iter()
                    .enumerate()
                {
                    let mut spans = Vec::new();
                    if i == 0 {
                        spans.push(Span::styled(head.clone(), Style::default().fg(t.accent)));
                        spans.push(Span::styled(verb, Style::default().fg(verb_color)));
                        spans.push(Span::styled(
                            format!("{agent}: "),
                            Style::default().fg(t.accent),
                        ));
                    } else {
                        spans.push(Span::raw(" ".repeat(indent)));
                    }
                    spans.push(Span::styled(piece, body_style));
                    lines.push(Line::from(spans));
                }
            }
        }

        if selected {
            selected_span = (row_start, lines.len());
        }
    }

    // Scroll so the cursor's whole row is on screen. A row taller than the
    // pane opens at its head, and `j` reads deeper into it line by line —
    // `objective_scroll` is how far in; `objective_overflow`, written here,
    // is how the keys know when the tail has finally been reached.
    let height = area.height as usize;
    let (row_top, row_end) = selected_span;
    let base = row_top.min(row_end.saturating_sub(height));
    let deepest = row_end.saturating_sub(height).max(base);
    let offset = (base + state.ui.objective_scroll as usize).min(deepest);
    state.ui.objective_scroll = (offset - base) as u16;
    state.ui.objective_overflow = row_end.saturating_sub(offset + height) as u16;
    frame.render_widget(
        Paragraph::new(lines).scroll((offset as u16, 0)),
        area,
    );
}

/// Greedy word wrap to `width`, splitting words longer than a line. Never
/// drops or elides anything: elision is how this pane lost the user's own
/// words twice.
pub(crate) fn wrapped(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let mut word = word;
        // A word longer than the line is cut, not dropped.
        while word.chars().count() > width {
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
            }
            let head: String = word.chars().take(width).collect();
            let taken = head.len();
            out.push(head);
            word = &word[taken..];
        }
        let need = word.chars().count() + if line.is_empty() { 0 } else { 1 };
        if line.chars().count() + need > width && !line.is_empty() {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn render_action_bar(frame: &mut Frame, area: Rect, state: &AppState, is_focused: bool) {
    let t = crate::theme::current();

    // A recent action's outcome outranks the key hints.
    if let Some(status) = state.ui.task_status() {
        let msg = Paragraph::new(Line::from(Span::styled(
            format!(" {status}"),
            Style::default().fg(t.info),
        )));
        frame.render_widget(msg, area);
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
        ("n", ":add "),
        ("e", ":edit "),
        ("d", ":del "),
        ("Space", ":state "),
        ("J/K", ":rank "),
        ("a", ":approve "),
        ("x", ":no "),
        ("h", ":help"),
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

/// The one word that heads a proposal row in the objectives list.
///
/// Pulled out of the render so it can be asserted on. It has to be read in
/// order: the lifecycle outranks the check's verdict once review has spoken,
/// because "resolved", "needs you" and "closed" are decisions while a verdict
/// is only evidence. Mid-flight, the verdict is the best headline there is.
///
/// The last two arms are the trap. A closed job stays `Approved` so the
/// ledger keeps the turns it burned, so without an explicit arm above them it
/// falls through to its check's verdict or to "queued" — and a job the user
/// stopped goes on advertising itself as running.
pub(crate) fn proposal_verb(proposal: &crate::models::Proposal) -> &'static str {
    use crate::models::{ProposalState, ReviewPhase, Verdict};
    match (proposal.review, &proposal.verdict, proposal.state) {
        (Some(ReviewPhase::Resolved), _, _) => "resolved ",
        (Some(ReviewPhase::NeedsUser), _, _) => "needs you ",
        (Some(ReviewPhase::Closed), _, _) => "closed ",
        (Some(ReviewPhase::AwaitingReview), _, _) => "in review ",
        (Some(ReviewPhase::Working), _, _) if proposal.review_rounds > 0 => "rework ",
        (_, Some(Verdict::Verified), _) => "verified ",
        (_, Some(Verdict::Rejected { .. }), _) => "rejected ",
        (_, Some(Verdict::Inconclusive { .. }), _) => "unclear ",
        (_, None, ProposalState::Approved) => "queued ",
        _ => "proposes ",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    /// Render the pane over a fixture workspace and return the screen as text.
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

    /// A project with one agent in it, focus on this pane.
    fn pane_state() -> (AppState, uuid::Uuid, tempfile::TempDir) {
        let (mut state, session_id, dir) = crate::app::tasks_view::tests::fixture();
        state.ui.focus = FocusPanel::TasksPane;
        (state, session_id, dir)
    }

    /// Nothing the user wrote may be dropped or elided — elision is how this
    /// pane lost their words twice.
    #[test]
    fn wrapping_keeps_every_word() {
        let text = "make absolutely sure that every objective wraps onto following lines";
        let lines = wrapped(text, 20);
        assert!(lines.len() > 1, "{lines:?}");
        assert!(lines.iter().all(|l| l.chars().count() <= 20), "{lines:?}");
        assert_eq!(lines.join(" "), text, "every word survives");

        let monster = "a".repeat(45);
        let lines = wrapped(&monster, 20);
        assert_eq!(lines.join(""), monster, "a monster token is cut, not lost");

        assert_eq!(wrapped("", 20), vec![String::new()], "empty stays one row");
    }

    #[test]
    fn the_pane_is_the_objectives_list_and_says_what_the_desk_holds() {
        let (mut state, _session_id, _dir) = pane_state();
        let out = screen(&mut state, 72, 12);
        assert!(out.contains("OBJECTIVES"), "{out}");
        assert!(out.contains("Desk: nothing needs you"), "{out}");
        assert!(out.contains("No objectives yet"), "{out}");
        assert!(!out.contains("Managers"), "the roster is gone:\n{out}");
    }

    /// The count is the one thing this pane says about decisions, and it has
    /// to name the key that opens them.
    #[test]
    fn the_desk_line_counts_what_is_waiting_and_names_the_key() {
        let (mut state, _session_id, _dir) = pane_state();
        state.data.workspaces[0]
            .proposals
            .push(crate::models::Proposal::new("m1", "split the auth module"));
        let out = screen(&mut state, 72, 12);
        assert!(out.contains("Desk: 1 waiting"), "{out}");
        assert!(out.contains("F3 opens it"), "{out}");
    }
}

/// The detail overlay: everything a decision deserves, in one modal.
///
/// Drawn last so it floats above the panes. `a`/`x` decide what it shows;
/// any other key closes it.
pub fn render_detail(frame: &mut Frame, state: &AppState) {
    use crate::models::ReviewPhase;

    let Some(target) = state.ui.detail else {
        return;
    };
    let t = crate::theme::current();
    let screen = frame.area();
    let w = (screen.width as f32 * 0.72) as u16;
    let h = (screen.height as f32 * 0.72) as u16;
    let area = Rect {
        x: (screen.width - w) / 2,
        y: (screen.height - h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(ratatui::widgets::Clear, area);

    let inner_width = w.saturating_sub(4) as usize;
    let mut lines: Vec<Line> = vec![Line::from("")];
    let mut field = |label: &str, value: &str, color| {
        if value.trim().is_empty() {
            return;
        }
        lines.push(Line::from(Span::styled(
            format!("  {label}"),
            Style::default().fg(t.fg_faint),
        )));
        for piece in wrapped(value.trim(), inner_width.saturating_sub(2)) {
            lines.push(Line::from(Span::styled(
                format!("  {piece}"),
                Style::default().fg(color),
            )));
        }
        lines.push(Line::from(""));
    };

    let title;
    match target {
        crate::app::DetailTarget::Proposal {
            workspace_id,
            proposal_id,
        } => {
            let Some(proposal) = state
                .data
                .workspaces
                .iter()
                .find(|ws| ws.id == workspace_id)
                .and_then(|ws| ws.proposals.iter().find(|p| p.id == proposal_id))
            else {
                return;
            };
            title = " Proposal ";
            let phase = match (proposal.review, &proposal.verdict) {
                (Some(ReviewPhase::Resolved), _) => "resolved".to_string(),
                (Some(ReviewPhase::NeedsUser), _) => "needs you".to_string(),
                (Some(ReviewPhase::Closed), _) => "closed — you declined it".to_string(),
                (Some(ReviewPhase::AwaitingReview), _) => "in review".to_string(),
                (Some(ReviewPhase::Working), _) if proposal.review_rounds > 0 => {
                    format!("rework, round {}", proposal.review_rounds)
                }
                (Some(ReviewPhase::Working), _) => "working".to_string(),
                (None, _) => format!("{:?}", proposal.state).to_lowercase(),
            };
            field("STATE", &phase, t.fg);
            field("INSTRUCTION", &proposal.instruction, t.fg);
            field("WHY", &proposal.rationale, t.fg_dim);
            field(
                "WHO",
                &format!(
                    "manager {} → agent {}",
                    proposal.manager,
                    proposal.agent.as_deref().unwrap_or("(none)")
                ),
                t.fg_dim,
            );
            if let Some(findings) = &proposal.findings {
                field("FINDINGS", findings, t.warning);
            }
            if let Some(verdict) = &proposal.verdict {
                field(
                    "VERDICT",
                    &format!("{} — {}", verdict.label(), verdict.why()),
                    t.fg,
                );
            }
            if let Some(run) = &proposal.result {
                field("CHECK OUTPUT", &run.tail, t.fg_faint);
            }
        }
        crate::app::DetailTarget::Objective {
            workspace_id,
            objective_id,
        } => {
            let Some(workspace) = state
                .data
                .workspaces
                .iter()
                .find(|ws| ws.id == workspace_id)
            else {
                return;
            };
            let Some(objective) = workspace.objectives.iter().find(|o| o.id == objective_id)
            else {
                return;
            };
            title = " Objective ";
            field("OBJECTIVE", &objective.text, t.fg);
            field("STATE", &format!("{:?}", objective.state).to_lowercase(), t.fg_dim);
            if let Some(check) = &objective.done_when {
                field(
                    "DONE WHEN",
                    &format!(
                        "{}{}",
                        check.command,
                        if check.proposed { "  (proposed, unapproved)" } else { "" }
                    ),
                    if check.proposed { t.warning } else { t.success },
                );
            }
            let ledger = crate::models::objective_ledger(&workspace.proposals, objective.id);
            field(
                "THIS WEEK",
                &format!(
                    "{} resolved · {} in flight · {} on you · ≈{} agent turns, {} reviews",
                    ledger.resolved_this_week,
                    ledger.in_flight,
                    ledger.needs_user,
                    ledger.agent_turns,
                    ledger.reviews
                ),
                t.fg_dim,
            );
        }
    }

    lines.push(Line::from(Span::styled(
        "  a: yes   x: no   any other key: close",
        Style::default().fg(t.fg_faint),
    )));

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.special))
        .style(Style::default().bg(t.bg));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}
