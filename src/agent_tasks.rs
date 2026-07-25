//! Live mirror of each agent's *own* task list.
//!
//! Coding agents break a prompt into a task list and keep it in their own
//! context — but they also journal every mutation to a session log on disk.
//! This module tails those logs and reconstructs, per workbench session:
//! the prompt that started a batch of work, the tasks the agent derived from
//! it, and each task's current state.
//!
//! ```text
//! Claude Code  ~/.claude/projects/<slug>/<session-uuid>.jsonl
//!              assistant tool_use `TaskCreate` / `TaskUpdate` (+ legacy
//!              `TodoWrite` snapshots); the id lands in the tool_result
//!              ("Task #3 created successfully: ...").
//! Codex        ~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl
//!              response_item function_call `update_plan`, whose arguments
//!              carry the WHOLE plan each time.
//! ```
//!
//! Both logs are append-only, so a tracker keeps a byte offset and only parses
//! what is new. Nothing here writes to the logs — the agent owns its list; the
//! pane influences it by talking to the agent (see `handlers::tasks`).

use chrono::{DateTime, Duration as ChronoDuration, Local, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::models::AgentType;

/// Where a task sits in the agent's list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Pending,
    InProgress,
    Completed,
}

impl TaskState {
    fn parse(s: &str) -> Self {
        match s {
            "in_progress" => TaskState::InProgress,
            "completed" | "complete" | "done" => TaskState::Completed,
            _ => TaskState::Pending,
        }
    }
}

/// One entry in an agent's task list.
#[derive(Debug, Clone)]
pub struct AgentTask {
    /// Agent-assigned id ("3" for Claude, the plan index for Codex). Used to
    /// apply later updates and to address the task when messaging the agent.
    pub id: String,
    pub subject: String,
    pub detail: Option<String>,
    pub state: TaskState,
}

/// A task list and the prompt that produced it.
#[derive(Debug, Clone)]
pub struct TaskBatch {
    pub prompt: String,
    pub at: Option<DateTime<Utc>>,
    pub tasks: Vec<AgentTask>,
}

impl TaskBatch {
    pub fn completed(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| t.state == TaskState::Completed)
            .count()
    }

    /// "4m", "2h" since the prompt landed — how stale this list is.
    pub fn age(&self) -> Option<String> {
        let at = self.at?;
        let mins = (Utc::now() - at).num_minutes().max(0);
        Some(match mins {
            0 => "now".to_string(),
            m if m < 60 => format!("{m}m"),
            m if m < 60 * 24 => format!("{}h", m / 60),
            m => format!("{}d", m / (60 * 24)),
        })
    }
}

/// Which session-log format an agent writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Claude,
    Codex,
}

impl Provider {
    /// `None` for agents whose task lists we cannot read (Gemini, Grok,
    /// plain terminals) — those panes simply show nothing.
    pub fn for_agent(agent: &AgentType) -> Option<Provider> {
        match agent {
            AgentType::Claude => Some(Provider::Claude),
            AgentType::Codex => Some(Provider::Codex),
            AgentType::Custom { command, .. } => match command.as_str() {
                "claude" => Some(Provider::Claude),
                "codex" => Some(Provider::Codex),
                _ => None,
            },
            _ => None,
        }
    }
}

/// Everything needed to find a session's log on disk.
#[derive(Debug, Clone)]
pub struct TaskSource {
    pub provider: Provider,
    /// Workbench session uuid. Claude sessions are spawned with
    /// `--session-id <this>`, which makes the log path deterministic.
    pub session_uuid: String,
    /// Directory the agent runs in (worktree path or workspace path).
    pub cwd: PathBuf,
    pub started_at: DateTime<Utc>,
}

/// Incremental reader over one session log.
#[derive(Debug, Clone)]
pub struct TaskTracker {
    provider: Provider,
    source: Option<PathBuf>,
    offset: u64,
    batches: Vec<TaskBatch>,
    /// Prompt seen since the last task mutation — starts the next batch.
    pending_prompt: Option<(String, Option<DateTime<Utc>>)>,
    /// Claude only: tool_use id → (batch, task), awaiting the tool_result
    /// that reveals the agent-assigned task number.
    awaiting_id: HashMap<String, (usize, usize)>,
}

impl TaskTracker {
    pub fn new(provider: Provider) -> Self {
        Self {
            provider,
            source: None,
            offset: 0,
            batches: Vec::new(),
            pending_prompt: None,
            awaiting_id: HashMap::new(),
        }
    }

    pub fn batches(&self) -> &[TaskBatch] {
        &self.batches
    }

    /// The batch the agent is working through right now (the newest one).
    pub fn current(&self) -> Option<&TaskBatch> {
        self.batches.last()
    }

    pub fn has_source(&self) -> bool {
        self.source.is_some()
    }

    /// Point a tracker at a specific log, skipping discovery (tests).
    #[cfg(test)]
    pub fn with_source(provider: Provider, path: PathBuf) -> Self {
        let mut tracker = Self::new(provider);
        tracker.source = Some(path);
        tracker
    }

    /// Locate the log if we haven't yet, then parse whatever is new.
    pub fn refresh(&mut self, ctx: &TaskSource) {
        if self.source.as_deref().map(Path::exists) != Some(true) {
            self.reset();
            self.source = locate(ctx);
        }
        let Some(path) = self.source.clone() else {
            return;
        };

        let len = match fs::metadata(&path) {
            Ok(m) => m.len(),
            Err(_) => return,
        };
        if len < self.offset {
            // Truncated/replaced underneath us — start over.
            let keep = self.source.take();
            self.reset();
            self.source = keep;
        }
        if len == self.offset {
            return;
        }

        let Ok(file) = File::open(&path) else {
            return;
        };
        let mut reader = BufReader::new(file);
        if reader.seek(SeekFrom::Start(self.offset)).is_err() {
            return;
        }

        let mut consumed = self.offset;
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(n) => {
                    // A line without a trailing newline is a partial write;
                    // leave it for the next refresh.
                    if !line.ends_with('\n') {
                        break;
                    }
                    consumed += n as u64;
                    if let Ok(value) = serde_json::from_str::<Value>(line.trim_end()) {
                        self.ingest(&value);
                    }
                }
                Err(_) => break,
            }
        }
        self.offset = consumed;
    }

    fn reset(&mut self) {
        self.source = None;
        self.offset = 0;
        self.batches.clear();
        self.pending_prompt = None;
        self.awaiting_id.clear();
    }

    fn ingest(&mut self, v: &Value) {
        match self.provider {
            Provider::Claude => self.ingest_claude(v),
            Provider::Codex => self.ingest_codex(v),
        }
    }

    // -- Claude ------------------------------------------------------------

    fn ingest_claude(&mut self, v: &Value) {
        // Subagent (Task tool) transcripts share the file; their task lists
        // are not the session's own.
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
                    self.note_prompt(text, ts);
                } else if let Some(blocks) = content.as_array() {
                    for block in blocks {
                        match block.get("type").and_then(Value::as_str) {
                            Some("text") => {
                                if let Some(text) = block.get("text").and_then(Value::as_str) {
                                    self.note_prompt(text, ts);
                                }
                            }
                            Some("tool_result") => self.apply_task_id(block),
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
                            let detail = input
                                .get("description")
                                .and_then(Value::as_str)
                                .filter(|s| !s.is_empty())
                                .map(str::to_string);
                            let batch_idx = self.open_batch(ts);
                            let tasks = &mut self.batches[batch_idx].tasks;
                            tasks.push(AgentTask {
                                // Provisional until the tool_result names it.
                                id: format!("?{}", tasks.len() + 1),
                                subject,
                                detail,
                                state: TaskState::Pending,
                            });
                            if let Some(tool_id) = block.get("id").and_then(Value::as_str) {
                                self.awaiting_id
                                    .insert(tool_id.to_string(), (batch_idx, tasks.len() - 1));
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
                            if let Some(task) = self.find_task_mut(&id) {
                                if let Some(state) = state {
                                    task.state = state;
                                }
                                if let Some(subject) =
                                    input.get("subject").and_then(Value::as_str)
                                {
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
                                        detail: None,
                                        state: todo
                                            .get("status")
                                            .and_then(Value::as_str)
                                            .map(TaskState::parse)
                                            .unwrap_or(TaskState::Pending),
                                    })
                                })
                                .collect();
                            let batch_idx = self.open_batch(ts);
                            self.batches[batch_idx].tasks = tasks;
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    /// "Task #3 created successfully: ..." → the id for a provisional task.
    fn apply_task_id(&mut self, block: &Value) {
        let Some(tool_id) = block.get("tool_use_id").and_then(Value::as_str) else {
            return;
        };
        let Some((batch_idx, task_idx)) = self.awaiting_id.remove(tool_id) else {
            return;
        };
        let content = block.get("content");
        let text = match content {
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
        if let Some(task) = self
            .batches
            .get_mut(batch_idx)
            .and_then(|b| b.tasks.get_mut(task_idx))
        {
            task.id = id;
        }
    }

    // -- Codex -------------------------------------------------------------

    fn ingest_codex(&mut self, v: &Value) {
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
                    self.note_prompt(text, ts);
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
                            detail: None,
                            state: step
                                .get("status")
                                .and_then(Value::as_str)
                                .map(TaskState::parse)
                                .unwrap_or(TaskState::Pending),
                        })
                    })
                    .collect();
                let batch_idx = self.open_batch(ts);
                // update_plan always carries the full plan — replace, never merge.
                self.batches[batch_idx].tasks = tasks;
            }
            _ => {}
        }
    }

    // -- Shared ------------------------------------------------------------

    fn note_prompt(&mut self, text: &str, at: Option<DateTime<Utc>>) {
        let text = text.trim();
        if !is_real_prompt(text) {
            return;
        }
        self.pending_prompt = Some((text.to_string(), at));
    }

    /// Index of the batch new tasks belong to: a fresh one if a prompt has
    /// arrived since the last mutation, otherwise the batch in progress.
    fn open_batch(&mut self, at: Option<DateTime<Utc>>) -> usize {
        if let Some((prompt, prompt_at)) = self.pending_prompt.take() {
            self.batches.push(TaskBatch {
                prompt,
                at: prompt_at.or(at),
                tasks: Vec::new(),
            });
        } else if self.batches.is_empty() {
            self.batches.push(TaskBatch {
                prompt: String::new(),
                at,
                tasks: Vec::new(),
            });
        }
        self.batches.len() - 1
    }

    fn find_task_mut(&mut self, id: &str) -> Option<&mut AgentTask> {
        self.batches
            .iter_mut()
            .rev()
            .find_map(|batch| batch.tasks.iter_mut().find(|t| t.id == id))
    }
}

/// Filters out the machinery that also arrives as "user" text: system
/// reminders, slash-command envelopes, tool output, interrupt notices.
fn is_real_prompt(text: &str) -> bool {
    if text.is_empty() || text.starts_with('<') {
        return false;
    }
    const NOISE: [&str; 4] = [
        "Caveat:",
        "[Request interrupted",
        "API Error",
        "This session is being continued from a previous conversation",
    ];
    !NOISE.iter().any(|p| text.starts_with(p))
}

fn timestamp(v: Option<&Value>) -> Option<DateTime<Utc>> {
    v.and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
}

/// Task ids appear as `"3"` or `3` depending on harness version.
fn json_id(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Locating a session's log
// ---------------------------------------------------------------------------

fn locate(ctx: &TaskSource) -> Option<PathBuf> {
    match ctx.provider {
        Provider::Claude => locate_claude(ctx),
        Provider::Codex => locate_codex(ctx),
    }
}

/// The log Claude would write for `session_uuid`, if one exists.
///
/// Doubles as the "is this id taken?" check at spawn time: Claude aborts with
/// "Session ID is already in use" if `--session-id` names an existing log, and
/// workbench reuses a session's uuid when you restart a stopped session.
pub fn claude_log_for_session(session_uuid: &str) -> Option<PathBuf> {
    let projects = dirs::home_dir()?.join(".claude").join("projects");
    let name = format!("{session_uuid}.jsonl");
    fs::read_dir(&projects)
        .ok()?
        .flatten()
        .map(|entry| entry.path().join(&name))
        .find(|path| path.is_file())
}

/// Sessions we spawn carry `--session-id <workbench uuid>`, so the log is
/// named after the session we already know. Sessions where that id was already
/// taken (a restart) or that resumed with `--continue` keep some other id, so
/// fall back to the newest log for this cwd.
fn locate_claude(ctx: &TaskSource) -> Option<PathBuf> {
    if let Some(pinned) = claude_log_for_session(&ctx.session_uuid) {
        return Some(pinned);
    }

    let projects = dirs::home_dir()?.join(".claude").join("projects");
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    let cutoff = ctx.started_at - ChronoDuration::minutes(1);

    for entry in fs::read_dir(&projects).ok()?.flatten() {
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

/// Codex has no "use this id" flag, so match on the `cwd` recorded in the
/// rollout's `session_meta` and take the newest one started with the session.
fn locate_codex(ctx: &TaskSource) -> Option<PathBuf> {
    let root = dirs::home_dir()?.join(".codex").join("sessions");
    let cutoff = ctx.started_at - ChronoDuration::minutes(1);

    // Rollout directories are YYYY/MM/DD in local time; only days from the
    // session's start onwards can hold its log.
    let start_day = cutoff.with_timezone(&Local).date_naive();
    let today = Local::now().date_naive();
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;

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
            if DateTime::<Utc>::from(modified) < cutoff {
                continue;
            }
            let is_newer = newest.as_ref().map_or(true, |(m, _)| modified > *m);
            if is_newer && codex_log_cwd(&path).as_deref() == Some(ctx.cwd.as_path()) {
                newest = Some((modified, path));
            }
        }
    }
    newest.map(|(_, p)| p)
}

fn codex_log_cwd(path: &Path) -> Option<PathBuf> {
    let file = File::open(path).ok()?;
    let mut first = String::new();
    BufReader::new(file).read_line(&mut first).ok()?;
    let value: Value = serde_json::from_str(first.trim_end()).ok()?;
    value
        .pointer("/payload/cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(tracker: &mut TaskTracker, lines: &[&str]) {
        for line in lines {
            tracker.ingest(&serde_json::from_str(line).unwrap());
        }
    }

    fn claude_user(text: &str) -> String {
        serde_json::json!({
            "type": "user",
            "timestamp": "2026-07-25T10:00:00.000Z",
            "message": {"content": [{"type": "text", "text": text}]}
        })
        .to_string()
    }

    fn claude_create(tool_id: &str, subject: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "timestamp": "2026-07-25T10:00:01.000Z",
            "message": {"content": [{
                "type": "tool_use", "id": tool_id, "name": "TaskCreate",
                "input": {"subject": subject, "description": "detail"}
            }]}
        })
        .to_string()
    }

    fn claude_created(tool_id: &str, n: usize, subject: &str) -> String {
        serde_json::json!({
            "type": "user",
            "message": {"content": [{
                "type": "tool_result", "tool_use_id": tool_id,
                "content": format!("Task #{n} created successfully: {subject}")
            }]}
        })
        .to_string()
    }

    fn claude_update(id: &str, status: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "message": {"content": [{
                "type": "tool_use", "id": "toolu_u", "name": "TaskUpdate",
                "input": {"taskId": id, "status": status}
            }]}
        })
        .to_string()
    }

    #[test]
    fn claude_batches_tasks_under_the_prompt_that_created_them() {
        let mut t = TaskTracker::new(Provider::Claude);
        feed(
            &mut t,
            &[
                &claude_user("build the tasks pane"),
                &claude_create("t1", "Parse session logs"),
                &claude_created("t1", 1, "Parse session logs"),
                &claude_create("t2", "Render the pane"),
                &claude_created("t2", 2, "Render the pane"),
                &claude_update("1", "completed"),
                &claude_update("2", "in_progress"),
            ],
        );

        assert_eq!(t.batches().len(), 1);
        let batch = t.current().unwrap();
        assert_eq!(batch.prompt, "build the tasks pane");
        assert_eq!(batch.tasks.len(), 2);
        assert_eq!(batch.tasks[0].id, "1");
        assert_eq!(batch.tasks[0].state, TaskState::Completed);
        assert_eq!(batch.tasks[1].state, TaskState::InProgress);
        assert_eq!(batch.completed(), 1);
    }

    #[test]
    fn claude_starts_a_new_batch_per_prompt_and_updates_reach_back() {
        let mut t = TaskTracker::new(Provider::Claude);
        feed(
            &mut t,
            &[
                &claude_user("first ask"),
                &claude_create("t1", "One"),
                &claude_created("t1", 1, "One"),
                &claude_user("second ask"),
                &claude_create("t2", "Two"),
                &claude_created("t2", 2, "Two"),
                // An update for the *earlier* batch still lands correctly.
                &claude_update("1", "completed"),
            ],
        );

        assert_eq!(t.batches().len(), 2);
        assert_eq!(t.batches()[0].prompt, "first ask");
        assert_eq!(t.batches()[0].tasks[0].state, TaskState::Completed);
        assert_eq!(t.batches()[1].prompt, "second ask");
        assert_eq!(t.batches()[1].tasks[0].state, TaskState::Pending);
    }

    #[test]
    fn claude_ignores_subagent_transcripts_and_synthetic_prompts() {
        let mut t = TaskTracker::new(Provider::Claude);
        feed(
            &mut t,
            &[
                &claude_user("<system-reminder>ignore me</system-reminder>"),
                &claude_user("real prompt"),
                &serde_json::json!({
                    "type": "assistant",
                    "isSidechain": true,
                    "message": {"content": [{
                        "type": "tool_use", "id": "sub", "name": "TaskCreate",
                        "input": {"subject": "subagent task"}
                    }]}
                })
                .to_string(),
                &claude_create("t1", "real task"),
            ],
        );

        assert_eq!(t.batches().len(), 1);
        assert_eq!(t.current().unwrap().prompt, "real prompt");
        assert_eq!(t.current().unwrap().tasks.len(), 1);
        assert_eq!(t.current().unwrap().tasks[0].subject, "real task");
    }

    #[test]
    fn claude_todowrite_snapshots_replace_the_list() {
        let snapshot = |items: Vec<(&str, &str)>| {
            serde_json::json!({
                "type": "assistant",
                "message": {"content": [{
                    "type": "tool_use", "id": "tw", "name": "TodoWrite",
                    "input": {"todos": items.iter().map(|(c, s)| serde_json::json!({
                        "content": c, "status": s, "activeForm": c
                    })).collect::<Vec<_>>()}
                }]}
            })
            .to_string()
        };

        let mut t = TaskTracker::new(Provider::Claude);
        feed(
            &mut t,
            &[
                &claude_user("legacy harness"),
                &snapshot(vec![("A", "in_progress"), ("B", "pending")]),
                &snapshot(vec![("A", "completed"), ("B", "in_progress")]),
            ],
        );

        assert_eq!(t.batches().len(), 1);
        let batch = t.current().unwrap();
        assert_eq!(batch.tasks.len(), 2);
        assert_eq!(batch.tasks[0].state, TaskState::Completed);
        assert_eq!(batch.tasks[1].state, TaskState::InProgress);
    }

    fn codex_user(text: &str) -> String {
        serde_json::json!({
            "type": "event_msg",
            "timestamp": "2026-07-25T10:00:00.000Z",
            "payload": {"type": "user_message", "message": text}
        })
        .to_string()
    }

    fn codex_plan(steps: Vec<(&str, &str)>) -> String {
        let plan: Vec<Value> = steps
            .iter()
            .map(|(s, st)| serde_json::json!({"step": s, "status": st}))
            .collect();
        serde_json::json!({
            "type": "response_item",
            "timestamp": "2026-07-25T10:00:01.000Z",
            "payload": {
                "type": "function_call",
                "name": "update_plan",
                "arguments": serde_json::json!({"plan": plan}).to_string()
            }
        })
        .to_string()
    }

    #[test]
    fn codex_plan_snapshots_replace_within_a_batch() {
        let mut t = TaskTracker::new(Provider::Codex);
        feed(
            &mut t,
            &[
                &codex_user("audit the repo"),
                &codex_plan(vec![("Map repo", "in_progress"), ("Report", "pending")]),
                &codex_plan(vec![("Map repo", "completed"), ("Report", "in_progress")]),
                &codex_user("now fix the worst one"),
                &codex_plan(vec![("Fix hotspot", "in_progress")]),
            ],
        );

        assert_eq!(t.batches().len(), 2);
        assert_eq!(t.batches()[0].prompt, "audit the repo");
        assert_eq!(t.batches()[0].tasks.len(), 2);
        assert_eq!(t.batches()[0].tasks[0].state, TaskState::Completed);
        assert_eq!(t.batches()[1].prompt, "now fix the worst one");
        assert_eq!(t.batches()[1].tasks.len(), 1);
    }

    #[test]
    fn tasks_without_a_preceding_prompt_still_land_somewhere() {
        let mut t = TaskTracker::new(Provider::Codex);
        feed(&mut t, &[&codex_plan(vec![("Orphan step", "pending")])]);
        assert_eq!(t.batches().len(), 1);
        assert!(t.current().unwrap().prompt.is_empty());
        assert_eq!(t.current().unwrap().tasks.len(), 1);
    }

    #[test]
    fn refresh_reads_only_what_is_new_and_skips_partial_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        fs::write(
            &path,
            format!("{}\n{}\n", codex_user("go"), codex_plan(vec![("One", "pending")])),
        )
        .unwrap();

        let ctx = TaskSource {
            provider: Provider::Codex,
            session_uuid: "x".into(),
            cwd: dir.path().to_path_buf(),
            started_at: Utc::now(),
        };
        let mut t = TaskTracker::new(Provider::Codex);
        t.source = Some(path.clone());
        t.refresh(&ctx);
        assert_eq!(t.current().unwrap().tasks.len(), 1);
        let after_first = t.offset;

        // Append a complete line plus a torn one.
        let more = format!("{}\n{{\"type\":\"resp", codex_plan(vec![("One", "completed"), ("Two", "pending")]));
        fs::write(&path, {
            let mut s = fs::read_to_string(&path).unwrap();
            s.push_str(&more);
            s
        })
        .unwrap();

        t.refresh(&ctx);
        assert_eq!(t.current().unwrap().tasks.len(), 2);
        assert_eq!(t.current().unwrap().tasks[0].state, TaskState::Completed);
        // The torn line was left unconsumed for the next pass.
        assert!(t.offset > after_first);
        assert!(t.offset < fs::metadata(&path).unwrap().len());
    }
}


