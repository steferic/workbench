//! Agents that journal to append-only JSONL: Claude Code and Codex.
//!
//! Both write one line per event, so a tracker keeps a byte offset and parses
//! only what was appended. Finding *which* file belongs to a workbench session
//! is the subtle part — see `locate_claude` / `locate_codex`.

use chrono::{DateTime, Duration as ChronoDuration, Local, Utc};
use serde_json::Value;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use super::{json_id, timestamp, AgentTask, BatchBuilder, TaskSource, TaskState};

pub(super) fn claude_projects_root(home: &Path) -> PathBuf {
    home.join(".claude").join("projects")
}

pub(super) fn codex_sessions_root(home: &Path) -> PathBuf {
    home.join(".codex").join("sessions")
}

// ---------------------------------------------------------------------------
// Claude Code
// ---------------------------------------------------------------------------

pub(super) fn ingest_claude(batches: &mut BatchBuilder, v: &Value) {
    // Subagent (Task tool) transcripts share the file; their task lists are
    // not the session's own.
    if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
        return;
    }
    let ts = timestamp(v.get("timestamp"));
    match v.get("type").and_then(Value::as_str) {
        Some("user") => {
            if v.get("isMeta").and_then(Value::as_bool) == Some(true) {
                return;
            }
            let Some(content) = v.pointer("/message/content") else {
                return;
            };
            if let Some(text) = content.as_str() {
                batches.note_prompt(text, ts);
            } else if let Some(blocks) = content.as_array() {
                for block in blocks {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(text) = block.get("text").and_then(Value::as_str) {
                                batches.note_prompt(text, ts);
                            }
                        }
                        Some("tool_result") => apply_task_id(batches, block),
                        _ => {}
                    }
                }
            }
        }
        Some("assistant") => {
            let Some(blocks) = v.pointer("/message/content").and_then(Value::as_array) else {
                return;
            };
            for block in blocks {
                if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                    continue;
                }
                let name = block.get("name").and_then(Value::as_str).unwrap_or("");
                let input = block.get("input");
                match name {
                    "TaskCreate" => {
                        let Some(input) = input else { continue };
                        let subject = input
                            .get("subject")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        if subject.is_empty() {
                            continue;
                        }
                        let at = batches.push_task(
                            AgentTask {
                                // Provisional until the tool_result names it.
                                id: String::new(),
                                subject,
                                state: TaskState::Pending,
                            },
                            ts,
                        );
                        if let Some(tool_id) = block.get("id").and_then(Value::as_str) {
                            batches.await_id(tool_id.to_string(), at);
                        }
                    }
                    "TaskUpdate" => {
                        let Some(input) = input else { continue };
                        let Some(id) = input.get("taskId").and_then(json_id) else {
                            continue;
                        };
                        let state = input
                            .get("status")
                            .and_then(Value::as_str)
                            .map(TaskState::parse);
                        if let Some(task) = batches.find_task_mut(&id) {
                            if let Some(state) = state {
                                task.state = state;
                            }
                            if let Some(subject) = input.get("subject").and_then(Value::as_str) {
                                task.subject = subject.to_string();
                            }
                        }
                    }
                    // Older harnesses snapshot the whole list instead.
                    "TodoWrite" => {
                        let Some(todos) =
                            input.and_then(|i| i.get("todos")).and_then(Value::as_array)
                        else {
                            continue;
                        };
                        let tasks = todos
                            .iter()
                            .enumerate()
                            .filter_map(|(i, todo)| {
                                let subject = todo
                                    .get("content")
                                    .and_then(Value::as_str)
                                    .or_else(|| todo.get("subject").and_then(Value::as_str))?;
                                Some(AgentTask {
                                    id: (i + 1).to_string(),
                                    subject: subject.to_string(),
                                    state: todo
                                        .get("status")
                                        .and_then(Value::as_str)
                                        .map(TaskState::parse)
                                        .unwrap_or(TaskState::Pending),
                                })
                            })
                            .collect();
                        batches.replace_tasks(tasks, ts);
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// "Task #3 created successfully: ..." → the id for a provisional task.
fn apply_task_id(batches: &mut BatchBuilder, block: &Value) {
    let Some(tool_id) = block.get("tool_use_id").and_then(Value::as_str) else {
        return;
    };
    let text = match block.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => return,
    };
    let Some(id) = text
        .split_once('#')
        .map(|(_, rest)| {
            rest.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
        })
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    batches.assign_id(tool_id, id);
}

/// The log Claude would write for `session_uuid`, if one exists.
///
/// Doubles as the "is this id taken?" check at spawn time: Claude aborts with
/// "Session ID is already in use" if `--session-id` names an existing log, and
/// workbench reuses a session's uuid when you restart a stopped session.
pub fn claude_log_for_session(session_uuid: &str) -> Option<PathBuf> {
    let projects = claude_projects_root(&dirs::home_dir()?);
    let name = format!("{session_uuid}.jsonl");
    fs::read_dir(&projects)
        .ok()?
        .flatten()
        .map(|entry| entry.path().join(&name))
        .find(|path| path.is_file())
}

/// Sessions we spawn carry `--session-id <workbench uuid>`, so the log is named
/// after the session we already know; a resumed session's log is named after
/// the conversation it resumed (Claude appends to it rather than forking).
/// Only when neither id is available — a `--continue` fallback — does this
/// guess from the newest unclaimed log for this cwd.
pub(super) fn locate_claude(
    projects: &Path,
    ctx: &TaskSource,
    claimed: &HashSet<String>,
) -> Option<PathBuf> {
    // A log named after this session, or after the conversation we asked it to
    // resume, is unambiguous — no cwd/mtime guessing needed.
    for id in [Some(ctx.session_uuid.as_str()), ctx.conversation.as_deref()]
        .into_iter()
        .flatten()
    {
        let name = format!("{id}.jsonl");
        if let Some(found) = fs::read_dir(projects)
            .ok()?
            .flatten()
            .map(|entry| entry.path().join(&name))
            .find(|path| path.is_file())
        {
            return Some(found);
        }
    }

    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    let cutoff = ctx.started_at - ChronoDuration::minutes(1);

    for entry in fs::read_dir(projects).ok()?.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        for file in fs::read_dir(&dir).ok().into_iter().flatten().flatten() {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(meta) = file.metadata() else { continue };
            let Ok(modified) = meta.modified() else {
                continue;
            };
            if DateTime::<Utc>::from(modified) < cutoff {
                continue;
            }
            if claimed.contains(&path.to_string_lossy().into_owned()) {
                continue;
            }
            let is_newer = newest.as_ref().map_or(true, |(m, _)| modified > *m);
            if is_newer && claude_log_cwd(&path).as_deref() == Some(ctx.cwd.as_path()) {
                newest = Some((modified, path));
            }
        }
    }
    newest.map(|(_, p)| p)
}

/// The working directory a Claude log belongs to, from its first entry that
/// carries one (the leading mode/permission lines do not).
fn claude_log_cwd(path: &Path) -> Option<PathBuf> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    for line in reader.lines().take(40).map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(cwd) = value.get("cwd").and_then(Value::as_str) {
            return Some(PathBuf::from(cwd));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Codex
// ---------------------------------------------------------------------------

pub(super) fn ingest_codex(batches: &mut BatchBuilder, v: &Value) {
    let ts = timestamp(v.get("timestamp"));
    let Some(payload) = v.get("payload") else {
        return;
    };
    match (
        v.get("type").and_then(Value::as_str),
        payload.get("type").and_then(Value::as_str),
    ) {
        (Some("event_msg"), Some("user_message")) => {
            if let Some(text) = payload.get("message").and_then(Value::as_str) {
                batches.note_prompt(text, ts);
            }
        }
        (Some("response_item"), Some("function_call"))
            if payload.get("name").and_then(Value::as_str) == Some("update_plan") =>
        {
            // `arguments` is a JSON *string* holding the entire plan.
            let Some(args) = payload.get("arguments").and_then(Value::as_str) else {
                return;
            };
            let Ok(args) = serde_json::from_str::<Value>(args) else {
                return;
            };
            let Some(plan) = args.get("plan").and_then(Value::as_array) else {
                return;
            };
            let tasks: Vec<AgentTask> = plan
                .iter()
                .enumerate()
                .filter_map(|(i, step)| {
                    let subject = step.get("step").and_then(Value::as_str)?;
                    Some(AgentTask {
                        id: (i + 1).to_string(),
                        subject: subject.to_string(),
                        state: step
                            .get("status")
                            .and_then(Value::as_str)
                            .map(TaskState::parse)
                            .unwrap_or(TaskState::Pending),
                    })
                })
                .collect();
            // update_plan always carries the full plan — replace, never merge.
            batches.replace_tasks(tasks, ts);
        }
        _ => {}
    }
}

/// Codex has no "use this id" flag, so its rollout has to be recognised.
///
/// Every codex start — fresh *or* resumed — creates a new rollout, so the one
/// belonging to a running session is the first rollout for this cwd created
/// after that process was spawned. Claiming then hands the next one to the
/// next session, which is why sessions are resolved in spawn order (see
/// `handler::refresh_agent_tasks`).
///
/// Without a spawn time (a session that is not running, or was already going
/// when this rule arrived) fall back to the newest unclaimed log for the cwd.
pub(super) fn locate_codex(
    root: &Path,
    ctx: &TaskSource,
    claimed: &HashSet<String>,
) -> Option<PathBuf> {
    // Clock skew between our spawn and codex writing its metadata.
    const SLACK: i64 = 5;
    let spawn_cutoff = ctx.spawned_at.map(|at| at - ChronoDuration::seconds(SLACK));
    let cutoff = spawn_cutoff.unwrap_or(ctx.started_at - ChronoDuration::minutes(1));

    // Rollout directories are YYYY/MM/DD in local time; only days from the
    // cutoff onwards can hold this session's log.
    let start_day = cutoff.with_timezone(&Local).date_naive();
    let today = Local::now().date_naive();

    // Spawn-anchored: the *earliest* qualifying rollout. Newest-first: the
    // last one touched. Both keep a single candidate.
    let mut best: Option<(DateTime<Utc>, PathBuf)> = None;

    let mut day = start_day;
    while day <= today {
        let dir = root
            .join(day.format("%Y").to_string())
            .join(day.format("%m").to_string())
            .join(day.format("%d").to_string());
        day += ChronoDuration::days(1);
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for file in entries.flatten() {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(modified) = file.metadata().and_then(|m| m.modified()) else {
                continue;
            };
            // A rollout is only appended to, so its last write can never
            // precede its creation — cheap way to skip old days' files.
            if DateTime::<Utc>::from(modified) < cutoff {
                continue;
            }
            if claimed.contains(&path.to_string_lossy().into_owned()) {
                continue;
            }
            let Some(head) = codex_log_head(&path) else {
                continue;
            };
            if head.cwd.as_deref() != Some(ctx.cwd.as_path()) {
                continue;
            }

            let (rank, better) = match spawn_cutoff {
                // Created since this process started, earliest first.
                Some(_) => {
                    // No opening timestamp means we cannot tell whether this
                    // rollout belongs to this process — skip it, don't abandon
                    // the search.
                    let Some(created) = head.created else { continue };
                    if created < cutoff {
                        continue;
                    }
                    (created, best.as_ref().map_or(true, |(b, _)| created < *b))
                }
                // No spawn anchor: the most recently written one.
                None => {
                    let touched = DateTime::<Utc>::from(modified);
                    (touched, best.as_ref().map_or(true, |(b, _)| touched > *b))
                }
            };
            if better {
                best = Some((rank, path));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// The id `codex resume <id>` reopens: the uuid the rollout is *named* after.
///
/// Not `session_meta.session_id` — on a rollout forked from another (which is
/// what resuming produces) that field holds the lineage root, so resuming it
/// would rewind past everything done since the fork.
pub(super) fn codex_rollout_id(path: &Path) -> Option<String> {
    const UUID_LEN: usize = 36;
    let stem = path.file_stem()?.to_string_lossy();
    let id = stem.get(stem.len().checked_sub(UUID_LEN)?..)?;
    let dashes: Vec<usize> = id.match_indices('-').map(|(i, _)| i).collect();
    (dashes == [8, 13, 18, 23]).then(|| id.to_string())
}

/// What the rollout's leading `session_meta` line says about itself.
struct CodexHead {
    cwd: Option<PathBuf>,
    created: Option<DateTime<Utc>>,
}

fn codex_log_head(path: &Path) -> Option<CodexHead> {
    let file = File::open(path).ok()?;
    let mut first = String::new();
    BufReader::new(file).read_line(&mut first).ok()?;
    let value: Value = serde_json::from_str(first.trim_end()).ok()?;
    Some(CodexHead {
        cwd: value
            .pointer("/payload/cwd")
            .and_then(Value::as_str)
            .map(PathBuf::from),
        // The envelope timestamp is when the rollout was opened.
        created: timestamp(value.get("timestamp"))
            .or_else(|| timestamp(value.pointer("/payload/timestamp"))),
    })
}
