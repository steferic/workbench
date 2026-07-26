//! Live mirror of each agent's *own* task list.
//!
//! Coding agents break a prompt into a task list and keep it in their own
//! context — but they also journal every mutation to a session store on disk.
//! This module reads those stores and reconstructs, per workbench session: the
//! prompt that started a batch of work, the tasks the agent derived from it,
//! and each task's current state. The same lookup yields the agent's
//! conversation id, which is what lets a restart resume *this* session's
//! history instead of the directory's most recent (see `pty::Resume`).
//!
//! Two shapes of store, one model:
//!
//! ```text
//! files.rs   Claude Code  ~/.claude/projects/<slug>/<session-uuid>.jsonl
//!            Codex        ~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl
//!            Append-only JSONL, tailed from a byte offset.
//!
//! db.rs      opencode     ~/.local/share/opencode/opencode.db
//!            hermes       ~/.hermes/state.db
//!            SQLite, re-queried when a cheap change probe moves.
//! ```
//!
//! Nothing here writes to a store — the agent owns its list; the pane
//! influences it by talking to the agent (see `handlers::tasks`).

mod db;
mod files;

use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::models::AgentType;

pub use files::claude_log_for_session;

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
            "in_progress" | "active" => TaskState::InProgress,
            "completed" | "complete" | "done" => TaskState::Completed,
            _ => TaskState::Pending,
        }
    }
}

/// One entry in an agent's task list.
#[derive(Debug, Clone)]
pub struct AgentTask {
    /// Agent-assigned id ("3" for Claude, the plan index for Codex, the todo
    /// key for hermes). Used to apply later updates and to address the task
    /// when messaging the agent.
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

/// Which session-store format an agent writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Claude,
    Codex,
    OpenCode,
    Hermes,
}

impl Provider {
    /// `None` for agents whose task lists we cannot read (Gemini, Grok, plain
    /// terminals) — those panes simply say so.
    pub fn for_agent(agent: &AgentType) -> Option<Provider> {
        if agent.is_terminal() {
            return None;
        }
        // Matching on the command means a user-configured custom agent gets the
        // same treatment as a built-in one (see `config::user_config`).
        Provider::for_command(agent.command())
    }

    pub fn for_command(command: &str) -> Option<Provider> {
        match command {
            "claude" => Some(Provider::Claude),
            "codex" => Some(Provider::Codex),
            "opencode" => Some(Provider::OpenCode),
            "hermes" => Some(Provider::Hermes),
            _ => None,
        }
    }

    /// Append-only log file vs. SQLite store.
    fn is_file_log(&self) -> bool {
        matches!(self, Provider::Claude | Provider::Codex)
    }
}

/// Everything needed to find a session's store on disk.
#[derive(Debug, Clone)]
pub struct TaskSource {
    pub provider: Provider,
    /// Workbench session uuid. Claude sessions are spawned with
    /// `--session-id <this>`, which makes the log path deterministic.
    pub session_uuid: String,
    /// Directory the agent runs in (worktree path or workspace path).
    pub cwd: PathBuf,
    pub started_at: DateTime<Utc>,
    /// The conversation this session was told to resume, when known. Claude
    /// appends to that conversation's log, so it names the file outright.
    pub conversation: Option<String>,
    /// When the *current* process was spawned, if it is running. Codex forks
    /// a fresh rollout on every start, so its log is the one created after
    /// this moment — a far tighter anchor than "newest in this directory".
    pub spawned_at: Option<DateTime<Utc>>,
}

/// The resolved store for one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// An append-only JSONL log (Claude, Codex).
    File(PathBuf),
    /// One conversation inside an agent's SQLite store (opencode, hermes).
    DbSession { db: PathBuf, session: String },
}

impl Source {
    /// Identity used to stop two workbench sessions claiming one conversation.
    pub fn key(&self) -> String {
        match self {
            Source::File(path) => path.to_string_lossy().into_owned(),
            Source::DbSession { db, session } => format!("{}#{session}", db.display()),
        }
    }

    fn still_there(&self) -> bool {
        match self {
            Source::File(path) => path.exists(),
            Source::DbSession { db, .. } => db.exists(),
        }
    }
}

/// Reader over one session's task list.
#[derive(Debug, Clone)]
pub struct TaskTracker {
    provider: Provider,
    source: Option<Source>,
    /// File logs: how far we have parsed.
    offset: u64,
    /// SQLite stores: cheap probe telling us whether anything changed.
    change_token: Option<String>,
    batches: BatchBuilder,
}

impl TaskTracker {
    pub fn new(provider: Provider) -> Self {
        Self {
            provider,
            source: None,
            offset: 0,
            change_token: None,
            batches: BatchBuilder::default(),
        }
    }

    pub fn batches(&self) -> &[TaskBatch] {
        &self.batches.batches
    }

    /// The batch the agent is working through right now (the newest one).
    pub fn current(&self) -> Option<&TaskBatch> {
        self.batches.batches.last()
    }

    pub fn has_source(&self) -> bool {
        self.source.is_some()
    }

    pub fn source(&self) -> Option<&Source> {
        self.source.as_ref()
    }

    /// The agent's own conversation id, read off the store we resolved. This
    /// is what `claude --resume` / `codex resume` / `hermes --resume` need to
    /// restore THIS session's history rather than the directory's most recent.
    pub fn provider_session_id(&self) -> Option<String> {
        match self.source.as_ref()? {
            // ~/.claude/projects/<slug>/<session-uuid>.jsonl
            Source::File(path) if self.provider == Provider::Claude => {
                path.file_stem().map(|s| s.to_string_lossy().into_owned())
            }
            // The rollout's *filename*, which is what `codex resume <id>`
            // reopens. Its `session_meta.session_id` is the lineage root
            // whenever the rollout was itself forked from another, so
            // resuming that would rewind to before the fork.
            Source::File(path) => files::codex_rollout_id(path),
            Source::DbSession { session, .. } => Some(session.clone()),
        }
    }

    /// Point a tracker at a specific store, skipping discovery (tests).
    #[cfg(test)]
    pub fn with_source(provider: Provider, source: Source) -> Self {
        let mut tracker = Self::new(provider);
        tracker.source = Some(source);
        tracker
    }

    /// Feed one already-parsed log line, as `tail_file` would (tests).
    #[cfg(test)]
    fn ingest_line(&mut self, value: &Value) {
        match self.provider {
            Provider::Claude => files::ingest_claude(&mut self.batches, value),
            Provider::Codex => files::ingest_codex(&mut self.batches, value),
            _ => {}
        }
    }

    /// Locate the store if we haven't yet, then read whatever is new.
    ///
    /// `claimed` holds the conversations other sessions already own. Two agents
    /// running in one directory would otherwise both resolve to the newest
    /// conversation there and mirror each other.
    pub fn refresh(&mut self, ctx: &TaskSource, claimed: &HashSet<String>) {
        if !self.source.as_ref().map(Source::still_there).unwrap_or(false) {
            self.reset();
            self.source = locate(ctx, claimed);
        }
        let Some(source) = self.source.clone() else {
            return;
        };

        match &source {
            Source::File(path) if self.provider.is_file_log() => self.tail_file(path),
            Source::DbSession { db, session } => self.requery_db(db, session),
            // A file source for a DB provider (or vice versa) cannot happen —
            // `locate` builds both — but don't loop on it if it ever does.
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.source = None;
        self.offset = 0;
        self.change_token = None;
        self.batches = BatchBuilder::default();
    }

    /// Parse the bytes appended since the last pass.
    fn tail_file(&mut self, path: &Path) {
        use std::fs::{self, File};
        use std::io::{BufRead, BufReader, Seek, SeekFrom};

        let len = match fs::metadata(path) {
            Ok(m) => m.len(),
            Err(_) => return,
        };
        if len < self.offset {
            // Truncated/replaced underneath us — start over.
            self.offset = 0;
            self.batches = BatchBuilder::default();
        }
        if len == self.offset {
            return;
        }

        let Ok(file) = File::open(path) else {
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
                        match self.provider {
                            Provider::Claude => files::ingest_claude(&mut self.batches, &value),
                            Provider::Codex => files::ingest_codex(&mut self.batches, &value),
                            _ => {}
                        }
                    }
                }
                Err(_) => break,
            }
        }
        self.offset = consumed;
    }

    /// SQLite stores have no append offset, so rebuild the list when a cheap
    /// probe says something moved. Sessions are small — a few dozen rows.
    fn requery_db(&mut self, db: &Path, session: &str) {
        let token = db::change_token(self.provider, db, session);
        if token.is_some() && token == self.change_token {
            return;
        }
        self.change_token = token;

        let mut builder = BatchBuilder::default();
        match self.provider {
            Provider::Hermes => db::load_hermes(&mut builder, db, session),
            Provider::OpenCode => db::load_opencode(&mut builder, db, session),
            _ => {}
        }
        self.batches = builder;
    }
}

fn locate(ctx: &TaskSource, claimed: &HashSet<String>) -> Option<Source> {
    let home = dirs::home_dir()?;
    match ctx.provider {
        Provider::Claude => files::locate_claude(&files::claude_projects_root(&home), ctx, claimed)
            .map(Source::File),
        Provider::Codex => files::locate_codex(&files::codex_sessions_root(&home), ctx, claimed)
            .map(Source::File),
        Provider::Hermes => db::locate_hermes(&db::hermes_db(&home), ctx, claimed),
        Provider::OpenCode => db::locate_opencode(&db::opencode_db(&home), ctx, claimed),
    }
}

// ---------------------------------------------------------------------------
// Assembling batches
// ---------------------------------------------------------------------------

/// Turns a stream of prompts and task-list mutations into `TaskBatch`es.
///
/// Every provider expresses the same two ideas — "the user asked for this" and
/// "the list now looks like that" — so they all drive this one state machine.
#[derive(Debug, Clone, Default)]
pub(crate) struct BatchBuilder {
    batches: Vec<TaskBatch>,
    /// Prompt seen since the last mutation — starts the next batch.
    pending_prompt: Option<(String, Option<DateTime<Utc>>)>,
    /// Claude only: tool_use id → (batch, task), awaiting the tool_result that
    /// reveals the agent-assigned task number.
    awaiting_id: HashMap<String, (usize, usize)>,
}

impl BatchBuilder {
    pub(crate) fn note_prompt(&mut self, text: &str, at: Option<DateTime<Utc>>) {
        let text = text.trim();
        if !is_real_prompt(text) {
            return;
        }
        self.pending_prompt = Some((text.to_string(), at));
    }

    /// Index of the batch new tasks belong to: a fresh one if a prompt has
    /// arrived since the last mutation, otherwise the batch in progress.
    pub(crate) fn open_batch(&mut self, at: Option<DateTime<Utc>>) -> usize {
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

    /// The whole list, as published by the agent (Codex plans, TodoWrite and
    /// hermes' non-merging todo calls all carry every task every time).
    pub(crate) fn replace_tasks(&mut self, tasks: Vec<AgentTask>, at: Option<DateTime<Utc>>) {
        let batch = self.open_batch(at);
        self.batches[batch].tasks = tasks;
    }

    /// A partial update: entries already in the list are patched in place,
    /// new ones are appended (hermes `todo` with `merge: true`).
    pub(crate) fn merge_tasks(&mut self, tasks: Vec<AgentTask>, at: Option<DateTime<Utc>>) {
        let batch = self.open_batch(at);
        for task in tasks {
            match self.batches[batch]
                .tasks
                .iter_mut()
                .find(|existing| existing.id == task.id)
            {
                Some(existing) => {
                    existing.state = task.state;
                    if !task.subject.is_empty() {
                        existing.subject = task.subject;
                    }
                }
                None => self.batches[batch].tasks.push(task),
            }
        }
    }

    pub(crate) fn push_task(&mut self, task: AgentTask, at: Option<DateTime<Utc>>) -> (usize, usize) {
        let batch = self.open_batch(at);
        self.batches[batch].tasks.push(task);
        (batch, self.batches[batch].tasks.len() - 1)
    }

    pub(crate) fn find_task_mut(&mut self, id: &str) -> Option<&mut AgentTask> {
        self.batches
            .iter_mut()
            .rev()
            .find_map(|batch| batch.tasks.iter_mut().find(|t| t.id == id))
    }

    pub(crate) fn await_id(&mut self, tool_use_id: String, at: (usize, usize)) {
        self.awaiting_id.insert(tool_use_id, at);
    }

    /// Give a provisional task the id the agent assigned it.
    pub(crate) fn assign_id(&mut self, tool_use_id: &str, id: String) {
        let Some((batch, task)) = self.awaiting_id.remove(tool_use_id) else {
            return;
        };
        if let Some(task) = self
            .batches
            .get_mut(batch)
            .and_then(|b| b.tasks.get_mut(task))
        {
            task.id = id;
        }
    }
}

/// Filters out the machinery that also arrives as "user" text: system
/// reminders, slash-command envelopes, tool output, interrupt notices.
fn is_real_prompt(text: &str) -> bool {
    if text.is_empty() || text.starts_with('<') {
        return false;
    }
    const NOISE: [&str; 5] = [
        "Caveat:",
        "[Request interrupted",
        "API Error",
        "This session is being continued from a previous conversation",
        "[IMPORTANT: ",
    ];
    !NOISE.iter().any(|p| text.starts_with(p))
}

fn timestamp(v: Option<&Value>) -> Option<DateTime<Utc>> {
    v.and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
}

/// Epoch seconds (hermes) or milliseconds (opencode) to a timestamp.
fn epoch(value: f64, millis: bool) -> Option<DateTime<Utc>> {
    let secs = if millis { value / 1000.0 } else { value };
    DateTime::from_timestamp(secs as i64, 0)
}

/// Task ids appear as `"3"` or `3` depending on harness version.
fn json_id(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
