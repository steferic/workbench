//! Details for decisions opened from the Desk.
use crate::app::AppState;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

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
            let Some(objective) = workspace.objectives.iter().find(|o| o.id == objective_id) else {
                return;
            };
            title = " Objective ";
            field("OBJECTIVE", &objective.text, t.fg);
            field(
                "STATE",
                &format!("{:?}", objective.state).to_lowercase(),
                t.fg_dim,
            );
            if let Some(check) = &objective.done_when {
                field(
                    "DONE WHEN",
                    &format!(
                        "{}{}",
                        check.command,
                        if check.proposed {
                            "  (proposed, unapproved)"
                        } else {
                            ""
                        }
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
