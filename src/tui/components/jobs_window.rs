//! The Jobs window (F4): a project's repeatable agent jobs, with their
//! history and stats, over the whole screen (see `app::jobs`).
//!
//! Left, the jobs; right, the selected one in four tabs. Overview is what
//! you want before pressing Enter: what the job is, how its runs have gone,
//! and whether its instructions changed since anyone last ran it. Runs is
//! the ledger, one line per run with the cursor's run in full underneath.
//! Lessons and Prompt are the two files the next run will read.
use crate::app::jobs::{self, DetailTab, Row, WindowFocus};
use crate::app::{Action, AppState};
use crate::jobs::{manifest::describe_every, RunRecord, RunStatus};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// "3m ago", "2h ago", "yesterday", "Sep 12".
pub fn ago(at: chrono::DateTime<chrono::Utc>) -> String {
    let elapsed = chrono::Utc::now() - at;
    let minutes = elapsed.num_minutes();
    if minutes < 1 {
        "just now".into()
    } else if minutes < 60 {
        format!("{minutes}m ago")
    } else if elapsed.num_hours() < 24 {
        format!("{}h ago", elapsed.num_hours())
    } else if elapsed.num_days() < 2 {
        "yesterday".into()
    } else if elapsed.num_days() < 7 {
        format!("{}d ago", elapsed.num_days())
    } else {
        at.format("%b %-d").to_string()
    }
}

fn duration(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {:02}m", seconds / 3600, (seconds % 3600) / 60)
    }
}

fn status_color(status: RunStatus, t: &crate::theme::Theme) -> ratatui::style::Color {
    match status {
        RunStatus::Running => t.success,
        RunStatus::Completed => t.fg,
        RunStatus::Partial | RunStatus::Unreported => t.warning,
        RunStatus::Blocked | RunStatus::Failed | RunStatus::Aborted => t.error,
    }
}

fn first_sentence(text: &str) -> String {
    let text = clean(text);
    text.split(['.', '\n'])
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

/// A run in one line: when, how it ended, how long, who.
fn run_line(run: &RunRecord) -> String {
    let took = run
        .ended_utc
        .and_then(|e| (e - run.started_utc).to_std().ok())
        .map(|d| duration(d.as_secs()))
        .unwrap_or_else(|| "…".into());
    let mut line = format!(
        "{}  {:<10} {:>7}  {}",
        run.started_utc.format("%Y-%m-%d %H:%M"),
        run.status.label(),
        took,
        clean(&run.by.user)
    );
    if let Some(summary) = &run.summary {
        let first = first_sentence(summary);
        if !first.is_empty() {
            line.push_str("  · ");
            line.push_str(&first);
        }
    }
    line
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let vertical = Layout::default()
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
        .split(vertical[1])[1]
}

pub fn render(frame: &mut Frame, state: &mut AppState) {
    let t = crate::theme::current();
    let full = frame.area();
    let area = if full.width < 100 || full.height < 30 {
        full
    } else {
        centered_rect(90, 90, full)
    };
    frame.render_widget(Clear, area);
    let rows = jobs::rows(state);
    let due = rows.iter().filter(|r| r.due).count();
    let running = rows.iter().filter(|r| r.open_session.is_some()).count();
    let scope = if state.ui.jobs.all_projects {
        "all projects".to_string()
    } else {
        state
            .selected_workspace()
            .map(|w| clean(&w.name))
            .unwrap_or_else(|| "no project".into())
    };
    let mut title = format!(" Jobs · {scope} · {} ", rows.len());
    if running > 0 {
        title.push_str(&format!("· {running} running "));
    }
    if due > 0 {
        title.push_str(&format!("· {due} due "));
    }
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border_focused))
        .style(Style::default().bg(t.bg).fg(t.fg));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 3 || inner.width < 20 {
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    let body = chunks[0];
    let footer = chunks[1];
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(36), Constraint::Percentage(64)])
        .split(body);
    render_list(frame, columns[0], state, &rows);
    let selected = jobs::selected(state);
    render_detail(frame, columns[1], state, selected.as_ref());
    render_footer(frame, footer, state, selected.as_ref());
}

fn render_list(frame: &mut Frame, area: Rect, state: &mut AppState, rows: &[Row]) {
    let t = crate::theme::current();
    let focused = state.ui.jobs.focus == WindowFocus::List;
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(t.border));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if rows.is_empty() {
        let has_manifest = state
            .selected_workspace()
            .is_some_and(|w| w.path.join(crate::jobs::MANIFEST).is_file());
        let text = if state.system.jobs_scan_inflight && state.system.project_jobs.is_empty() {
            "Reading…".to_string()
        } else if has_manifest {
            format!(
                "No jobs in {}.\n\nn adds one with an agent's help.",
                crate::jobs::MANIFEST
            )
        } else if state.ui.jobs.all_projects {
            "No project has a manifest yet.\n\nSelect a project and press n.".to_string()
        } else {
            format!(
                "This project has no {}.\n\nn creates it and starts an agent that adds the first job with you; a shows every project's jobs.",
                crate::jobs::MANIFEST
            )
        };
        frame.render_widget(
            Paragraph::new(text)
                .style(Style::default().fg(t.fg_dim))
                .wrap(Wrap { trim: false }),
            Rect::new(
                inner.x + 1,
                inner.y,
                inner.width.saturating_sub(2),
                inner.height,
            ),
        );
        return;
    }
    let row_height = 2u16;
    let capacity = (inner.height as usize / row_height as usize).max(1);
    let selected = rows
        .iter()
        .position(|r| Some(&r.key) == state.ui.jobs.selected.as_ref())
        .unwrap_or(0);
    let offset = &mut state.ui.jobs.offset;
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
        let y = inner.y + visible as u16 * row_height;
        let rect = Rect::new(
            inner.x,
            y,
            inner.width,
            inner.bottom().saturating_sub(y).min(row_height),
        );
        if rect.height == 0 {
            continue;
        }
        let style = if index == selected && focused {
            Style::default()
                .bg(t.selection_bg)
                .add_modifier(Modifier::BOLD)
        } else if index == selected {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let lines = match (&row.job, &row.error) {
            (Some(job), _) => {
                let (marker, color) = if row.open_session.is_some() {
                    ("▶", t.success)
                } else if row.due {
                    ("●", t.warning)
                } else {
                    ("○", t.fg_faint)
                };
                let mut first = vec![
                    Span::styled(format!(" {marker} "), Style::default().fg(color)),
                    Span::raw(clean(&job.title)),
                ];
                if state.ui.jobs.all_projects {
                    first.push(Span::styled(
                        format!("  {}", clean(&row.project)),
                        Style::default().fg(t.fg_faint),
                    ));
                }
                let mut second = String::from("   ");
                match &row.last {
                    None => second.push_str("never run"),
                    Some(run) if run.status.is_open() => {
                        second.push_str(&format!("running · {}", ago(run.started_utc)))
                    }
                    Some(run) => second.push_str(&format!(
                        "{} · {}",
                        run.status.label(),
                        ago(run.started_utc)
                    )),
                }
                if let Some(every) = job.every {
                    second.push_str(&format!(" · every {}", describe_every(every)));
                }
                if row.due {
                    second.push_str(" · due");
                }
                let second_color = row
                    .last
                    .as_ref()
                    .map(|r| status_color(r.status, &t))
                    .unwrap_or(t.fg_dim);
                vec![
                    Line::from(first),
                    Line::from(Span::styled(second, Style::default().fg(second_color))),
                ]
            }
            (None, error) => vec![
                Line::from(vec![
                    Span::styled(" ! ", Style::default().fg(t.error)),
                    Span::raw(format!(
                        "{} · {}",
                        clean(&row.project),
                        crate::jobs::MANIFEST
                    )),
                ]),
                Line::from(Span::styled(
                    format!("   {}", clean(error.as_deref().unwrap_or("unreadable"))),
                    Style::default().fg(t.error),
                )),
            ],
        };
        frame.render_widget(Paragraph::new(lines).style(style), rect);
        state
            .ui
            .click_hits
            .push((rect, Action::SelectJob(row.key.clone())));
    }
}

fn render_detail(frame: &mut Frame, area: Rect, state: &mut AppState, row: Option<&Row>) {
    let t = crate::theme::current();
    let focused = state.ui.jobs.focus == WindowFocus::Detail;
    let tabs_area = Rect::new(area.x, area.y, area.width, 1);
    let content = Rect::new(
        area.x + 1,
        area.y + 2,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    // Tabs
    let mut spans = vec![Span::raw(" ")];
    let mut x = tabs_area.x + 1;
    for (i, tab) in DetailTab::ALL.iter().enumerate() {
        let label = format!(" {} {} ", i + 1, tab.label());
        let width = label.chars().count() as u16;
        if x + width > tabs_area.right() {
            break;
        }
        let style = if *tab == state.ui.jobs.tab && focused {
            Style::default()
                .fg(t.on_accent)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD)
        } else if *tab == state.ui.jobs.tab {
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.fg_dim)
        };
        spans.push(Span::styled(label, style));
        spans.push(Span::raw(" "));
        state
            .ui
            .click_hits
            .push((Rect::new(x, tabs_area.y, width, 1), Action::JobsTab(*tab)));
        x += width + 1;
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), tabs_area);
    let Some(row) = row else {
        return;
    };
    let Some(job) = &row.job else {
        frame.render_widget(
            Paragraph::new(clean(row.error.as_deref().unwrap_or("")))
                .style(Style::default().fg(t.error))
                .wrap(Wrap { trim: false }),
            content,
        );
        return;
    };
    match state.ui.jobs.tab {
        DetailTab::Overview => render_overview(frame, content, state, row, job),
        DetailTab::Runs => render_runs(frame, content, state, focused),
        DetailTab::Lessons => {
            let text = state
                .system
                .project_jobs
                .get(&row.key.workspace)
                .and_then(|p| p.lessons.get(&job.id))
                .map(|l| clean_multiline(l))
                .unwrap_or_else(|| {
                    format!(
                        "No lessons yet.\n\nAn agent adds one with `workbench jobs report --lesson \"…\"`; every later run reads {}/{}.md first.",
                        crate::jobs::LESSONS_DIR,
                        job.id
                    )
                });
            frame.render_widget(
                Paragraph::new(text)
                    .wrap(Wrap { trim: false })
                    .scroll((state.ui.jobs.scroll, 0)),
                content,
            );
        }
        DetailTab::Prompt => {
            let body = state
                .system
                .project_jobs
                .get(&row.key.workspace)
                .and_then(|p| p.prompts.get(&job.id))
                .cloned()
                .unwrap_or_else(|| Ok(String::new()));
            let mut lines: Vec<Line> = Vec::new();
            match body {
                Ok(body) => {
                    let source = match &job.prompt {
                        crate::jobs::PromptSource::Inline(_) => {
                            "inline in the manifest".to_string()
                        }
                        crate::jobs::PromptSource::File(path) => format!("from {}", path.display()),
                    };
                    lines.push(Line::from(Span::styled(
                        format!("What the agent is sent ({source}), then the harness footer:"),
                        Style::default().fg(t.fg_dim),
                    )));
                    lines.push(Line::from(""));
                    for l in body.lines() {
                        lines.push(Line::from(clean(l)));
                    }
                    lines.push(Line::from(""));
                    for l in crate::jobs::prompt::footer(&job.id, "<run-id>").lines() {
                        lines.push(Line::from(Span::styled(
                            clean(l),
                            Style::default().fg(t.fg_dim),
                        )));
                    }
                }
                Err(error) => lines.push(Line::from(Span::styled(
                    clean(&error),
                    Style::default().fg(t.error),
                ))),
            }
            frame.render_widget(
                Paragraph::new(lines)
                    .wrap(Wrap { trim: false })
                    .scroll((state.ui.jobs.scroll, 0)),
                content,
            );
        }
    }
}

fn clean_multiline(text: &str) -> String {
    text.lines().map(clean).collect::<Vec<_>>().join("\n")
}

fn render_overview(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    row: &Row,
    job: &crate::jobs::JobDef,
) {
    let t = crate::theme::current();
    let dim = Style::default().fg(t.fg_dim);
    let head = Style::default().fg(t.accent).add_modifier(Modifier::BOLD);
    let ov = jobs::overview(state, row);
    let mut lines: Vec<Line> = Vec::new();

    let mut title = vec![Span::styled(
        clean(&job.title),
        Style::default().add_modifier(Modifier::BOLD),
    )];
    if let Some(every) = job.every {
        title.push(Span::styled(
            format!("   every {}", describe_every(every)),
            dim,
        ));
    }
    if row.due {
        title.push(Span::styled("   due", Style::default().fg(t.warning)));
    }
    if row.open_session.is_some() {
        title.push(Span::styled(
            "   running now",
            Style::default().fg(t.success),
        ));
    }
    lines.push(Line::from(title));
    let mut meta = format!("id {} · project {}", job.id, clean(&row.project));
    meta.push_str(&format!(
        " · agent {}",
        job.agent.as_deref().unwrap_or("default")
    ));
    if job.skip_permissions {
        meta.push_str(" · ⚡ permissions off");
    }
    if !job.tags.is_empty() {
        meta.push_str(&format!(" · {}", job.tags.join(", ")));
    }
    lines.push(Line::from(Span::styled(meta, dim)));
    if !job.description.is_empty() {
        lines.push(Line::from(clean(&job.description)));
    }
    lines.push(Line::from(""));

    // Stats
    let s = &ov.stats;
    lines.push(Line::from(Span::styled("Stats", head)));
    if s.total == 0 {
        lines.push(Line::from(Span::styled("  never run", dim)));
    } else {
        let mut counts = vec![format!("{} runs", s.total)];
        for (n, label) in [
            (s.completed, "completed"),
            (s.partial, "partial"),
            (s.blocked, "blocked"),
            (s.failed, "failed"),
            (s.unreported, "unreported"),
            (s.aborted, "aborted"),
            (s.running, "running"),
        ] {
            if n > 0 {
                counts.push(format!("{n} {label}"));
            }
        }
        lines.push(Line::from(format!("  {}", counts.join(" · "))));
        let mut rates = Vec::new();
        if let Some(pct) = s.success_pct {
            rates.push(format!("success {pct}%"));
        }
        if let Some(secs) = s.mean_seconds {
            rates.push(format!("mean {}", duration(secs)));
        }
        rates.push(format!("last 7 days {}", s.last_7_days));
        rates.push(format!(
            "by {}",
            s.people
                .iter()
                .map(|p| clean(p))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        lines.push(Line::from(format!("  {}", rates.join(" · "))));
        if let Some(last) = &row.last {
            let mut text = format!(
                "  last {} {} by {} on {}",
                ago(last.started_utc),
                last.status.label(),
                clean(&last.by.user),
                clean(&last.by.host)
            );
            if last.status.is_open() && row.open_session.is_none() {
                text.push_str(" (not in this workbench)");
            }
            lines.push(Line::from(Span::styled(
                text,
                Style::default().fg(status_color(last.status, &t)),
            )));
        }
    }
    lines.push(Line::from(""));

    // Instructions
    let mut ins_title = format!("Instructions ({})", ov.instructions.len());
    if ov.changed_since_last_run > 0 {
        ins_title.push_str(&format!(
            " · {} changed since the last run",
            ov.changed_since_last_run
        ));
    }
    lines.push(Line::from(Span::styled(ins_title, head)));
    if ov.instructions.is_empty() {
        lines.push(Line::from(Span::styled(
            "  none listed; add `instructions = [...]` so runs record their versions",
            dim,
        )));
    }
    for (path, hash, before) in &ov.instructions {
        let mut spans = vec![
            Span::raw(format!("  {path}  ")),
            Span::styled(hash.clone(), dim),
        ];
        match before {
            Some(b) if b != hash => spans.push(Span::styled(
                format!("  changed (was {b})"),
                Style::default().fg(t.warning),
            )),
            Some(_) => spans.push(Span::styled("  same as last run", dim)),
            None => {}
        }
        if hash == "missing" {
            spans.push(Span::styled("  missing", Style::default().fg(t.error)));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(""));

    // Last report
    if let Some(last) = row.last.as_ref().filter(|r| r.summary.is_some()) {
        lines.push(Line::from(Span::styled("Last report", head)));
        for l in last.summary.as_deref().unwrap_or("").lines() {
            lines.push(Line::from(format!("  {}", clean(l))));
        }
        if let Some(artifacts) = &last.artifacts {
            lines.push(Line::from(Span::styled(
                format!("  artifacts: {}", clean(artifacts)),
                dim,
            )));
        }
        for lesson in &last.lessons {
            lines.push(Line::from(Span::styled(
                format!("  lesson: {}", clean(lesson)),
                dim,
            )));
        }
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((state.ui.jobs.scroll, 0)),
        area,
    );
}

fn render_runs(frame: &mut Frame, area: Rect, state: &mut AppState, focused: bool) {
    let t = crate::theme::current();
    let dim = Style::default().fg(t.fg_dim);
    let runs = jobs::selected_runs(state);
    if runs.is_empty() {
        frame.render_widget(
            Paragraph::new("No runs yet. Enter starts one.").style(dim),
            area,
        );
        return;
    }
    let cursor = state.ui.jobs.run_cursor.min(runs.len() - 1);
    state.ui.jobs.run_cursor = cursor;
    // The list takes the upper part; the cursor's run is spelled out below.
    let list_height = (area.height / 2).max(3).min(runs.len() as u16 + 1);
    let list = Rect::new(area.x, area.y, area.width, list_height.min(area.height));
    let detail = Rect::new(
        area.x,
        area.y + list.height,
        area.width,
        area.height.saturating_sub(list.height),
    );
    lines_header(frame, list, dim);
    let capacity = list.height.saturating_sub(1) as usize;
    let offset = cursor.saturating_sub(capacity.saturating_sub(1));
    for (i, run) in runs.iter().enumerate().skip(offset).take(capacity) {
        let y = list.y + 1 + (i - offset) as u16;
        let rect = Rect::new(list.x, y, list.width, 1);
        let style = if i == cursor && focused {
            Style::default()
                .bg(t.selection_bg)
                .add_modifier(Modifier::BOLD)
        } else if i == cursor {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                run_line(run),
                Style::default().fg(status_color(run.status, &t)),
            ))
            .style(style),
            rect,
        );
    }
    if detail.height == 0 {
        return;
    }
    let run = &runs[cursor];
    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                format!("Run {}", run.run_id),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "  {} · agent {}{}",
                    run.status.label(),
                    run.agent,
                    run.session
                        .as_ref()
                        .map(|s| format!(" · session {s}"))
                        .unwrap_or_default()
                ),
                dim,
            ),
        ]),
    ];
    lines.push(Line::from(Span::styled(
        format!(
            "{} → {} · {} on {}",
            run.started_utc.format("%Y-%m-%d %H:%M:%S"),
            run.ended_utc
                .map(|e| e.format("%H:%M:%S").to_string())
                .unwrap_or_else(|| "…".into()),
            clean(&run.by.user),
            clean(&run.by.host)
        ),
        dim,
    )));
    match &run.summary {
        Some(summary) => {
            for l in summary.lines() {
                lines.push(Line::from(clean(l)));
            }
        }
        None => lines.push(Line::from(Span::styled(
            match run.status {
                RunStatus::Running => "no report yet",
                RunStatus::Unreported => "the agent's turn ended without a report",
                RunStatus::Aborted => "the session ended before a report",
                _ => "no summary",
            },
            dim,
        ))),
    }
    if let Some(artifacts) = &run.artifacts {
        lines.push(Line::from(Span::styled(
            format!("artifacts: {}", clean(artifacts)),
            dim,
        )));
    }
    for lesson in &run.lessons {
        lines.push(Line::from(Span::styled(
            format!("lesson: {}", clean(lesson)),
            dim,
        )));
    }
    if !run.instructions_sha256_16.is_empty() {
        lines.push(Line::from(Span::styled(
            format!(
                "instructions: {}",
                run.instructions_sha256_16
                    .iter()
                    .map(|(p, h)| format!("{p} {h}"))
                    .collect::<Vec<_>>()
                    .join(" · ")
            ),
            dim,
        )));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), detail);
}

fn lines_header(frame: &mut Frame, list: Rect, dim: Style) {
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(
                "{:<16}  {:<10} {:>7}  who · summary",
                "started", "status", "took"
            ),
            dim,
        )),
        Rect::new(list.x, list.y, list.width, 1),
    );
}

fn render_footer(frame: &mut Frame, area: Rect, state: &mut AppState, row: Option<&Row>) {
    let t = crate::theme::current();
    if let Some(message) = state.ui.jobs.message.clone() {
        frame.render_widget(
            Paragraph::new(format!(" {}", clean(&message))).style(Style::default().fg(t.warning)),
            area,
        );
        return;
    }
    let run_label = match row.and_then(|r| r.open_session) {
        Some(_) => "go to run",
        None => "run",
    };
    let hints: [(&str, &str, Action); 8] = [
        ("Enter", run_label, Action::JobRun),
        ("R", "again", Action::JobRunForce),
        ("i", "improve", Action::JobImprove),
        ("n", "new", Action::JobNew),
        ("a", "scope", Action::JobsScope),
        ("r", "refresh", Action::JobsRefresh),
        ("Tab", "detail", Action::JobsSwitchFocus),
        ("Esc", "close", Action::CloseJobs),
    ];
    let mut x = area.x + 1;
    let mut spans = vec![Span::raw(" ")];
    for (key, label, action) in hints {
        let width = (key.chars().count() + 1 + label.chars().count() + 2) as u16;
        if x + width > area.right() {
            break;
        }
        spans.push(Span::styled(key, Style::default().fg(t.accent)));
        spans.push(Span::styled(
            format!(":{label}  "),
            Style::default().fg(t.fg_dim),
        ));
        state
            .ui
            .click_hits
            .push((Rect::new(x, area.y, width, 1), action));
        x += width;
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
