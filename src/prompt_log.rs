//! Durable history of the messages submitted to agents through Workbench.
//!
//! Terminal input reaches the PTY as keystrokes, so this module mirrors the
//! small amount of line editing needed to recover the text present when Enter
//! is pressed. The resulting prompt is written to one global SQLite database
//! outside every repository, with the project/session/model metadata that was
//! true at submission time.

use anyhow::{Context, Result};
use chrono::{DateTime, Local, Utc};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

/// Process-local mirror of each agent composer's current text.
#[derive(Debug, Default)]
pub struct PromptCapture {
    drafts: HashMap<Uuid, Draft>,
}

#[derive(Debug, Default)]
struct Draft {
    text: String,
    cursor: usize,
    history: Vec<String>,
    history_index: Option<usize>,
    text_before_history: String,
}

impl PromptCapture {
    /// Observe bytes Workbench is forwarding to an agent.
    ///
    /// Returns the completed message only when Enter submits non-empty text.
    pub fn observe(&mut self, session_id: Uuid, bytes: &[u8]) -> Option<String> {
        let draft = self.drafts.entry(session_id).or_default();

        if bytes == b"\r" || bytes == b"\n" {
            let text = std::mem::take(&mut draft.text);
            draft.cursor = 0;
            draft.history_index = None;
            draft.text_before_history.clear();
            let text = text.trim().to_string();
            if !text.is_empty() {
                draft.history.push(text.clone());
            }
            return (!text.is_empty()).then_some(text);
        }

        if bytes.starts_with(PASTE_START) && bytes.ends_with(PASTE_END) {
            let content = &bytes[PASTE_START.len()..bytes.len() - PASTE_END.len()];
            if let Ok(text) = std::str::from_utf8(content) {
                draft.insert(text);
            }
            return None;
        }

        match bytes {
            // Backspace / Delete.
            b"\x7f" => draft.backspace(),
            b"\x1b[3~" => draft.delete(),
            // Home / End, including the control-key forms Workbench emits.
            b"\x01" | b"\x1b[H" => draft.cursor = 0,
            b"\x05" | b"\x1b[F" => draft.cursor = draft.text.len(),
            // Character and word navigation.
            b"\x1b[D" => draft.move_left(),
            b"\x1b[C" => draft.move_right(),
            b"\x1b[A" => draft.recall_previous(),
            b"\x1b[B" => draft.recall_next(),
            b"\x1bb" => draft.move_word_left(),
            b"\x1bf" => draft.move_word_right(),
            // Word/line deletion.
            b"\x17" | b"\x1b\x7f" => draft.delete_word_left(),
            b"\x1bd" => draft.delete_word_right(),
            b"\x15" => draft.delete_to_start(),
            b"\x03" => {
                draft.text.clear();
                draft.cursor = 0;
            }
            // Navigation, menus, tab completion and other control sequences
            // do not add text to the composer mirror.
            _ if bytes.first().is_some_and(|b| *b == 0x1b || *b < 0x20) => {}
            _ => {
                if let Ok(text) = std::str::from_utf8(bytes) {
                    draft.insert(text);
                }
            }
        }
        None
    }

    pub fn reset(&mut self, session_id: Uuid) {
        self.drafts.remove(&session_id);
    }
}

impl Draft {
    fn insert(&mut self, value: &str) {
        self.text.insert_str(self.cursor, value);
        self.cursor += value.len();
    }

    fn move_left(&mut self) {
        if let Some((at, _)) = self.text[..self.cursor].char_indices().next_back() {
            self.cursor = at;
        }
    }

    fn move_right(&mut self) {
        if let Some(ch) = self.text[self.cursor..].chars().next() {
            self.cursor += ch.len_utf8();
        }
    }

    fn backspace(&mut self) {
        let Some((at, _)) = self.text[..self.cursor].char_indices().next_back() else {
            return;
        };
        self.text.replace_range(at..self.cursor, "");
        self.cursor = at;
    }

    fn delete(&mut self) {
        let Some(ch) = self.text[self.cursor..].chars().next() else {
            return;
        };
        self.text
            .replace_range(self.cursor..self.cursor + ch.len_utf8(), "");
    }

    fn move_word_left(&mut self) {
        self.cursor = word_start(&self.text, self.cursor);
    }

    fn move_word_right(&mut self) {
        self.cursor = word_end(&self.text, self.cursor);
    }

    fn delete_word_left(&mut self) {
        let start = word_start(&self.text, self.cursor);
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    fn delete_word_right(&mut self) {
        let end = word_end(&self.text, self.cursor);
        self.text.replace_range(self.cursor..end, "");
    }

    fn delete_to_start(&mut self) {
        self.text.replace_range(..self.cursor, "");
        self.cursor = 0;
    }

    fn recall_previous(&mut self) {
        if self.history.is_empty() || (!self.text.is_empty() && self.history_index.is_none()) {
            return;
        }
        let index = match self.history_index {
            Some(index) => index.saturating_sub(1),
            None => {
                self.text_before_history = self.text.clone();
                self.history.len() - 1
            }
        };
        self.history_index = Some(index);
        self.text.clone_from(&self.history[index]);
        self.cursor = self.text.len();
    }

    fn recall_next(&mut self) {
        let Some(index) = self.history_index else {
            return;
        };
        if index + 1 < self.history.len() {
            self.history_index = Some(index + 1);
            self.text.clone_from(&self.history[index + 1]);
        } else {
            self.history_index = None;
            self.text = std::mem::take(&mut self.text_before_history);
        }
        self.cursor = self.text.len();
    }
}

fn word_start(text: &str, cursor: usize) -> usize {
    let mut at = cursor;
    while let Some((index, ch)) = text[..at].char_indices().next_back() {
        if !ch.is_whitespace() {
            break;
        }
        at = index;
    }
    while let Some((index, ch)) = text[..at].char_indices().next_back() {
        if ch.is_whitespace() {
            break;
        }
        at = index;
    }
    at
}

fn word_end(text: &str, cursor: usize) -> usize {
    let mut at = cursor;
    while let Some(ch) = text[at..].chars().next() {
        if !ch.is_whitespace() {
            break;
        }
        at += ch.len_utf8();
    }
    while let Some(ch) = text[at..].chars().next() {
        if ch.is_whitespace() {
            break;
        }
        at += ch.len_utf8();
    }
    at
}

#[derive(Debug, Clone)]
pub struct PromptMetadata {
    pub sent_at: DateTime<Utc>,
    pub workspace_id: Uuid,
    pub workspace_name: String,
    pub workspace_path: PathBuf,
    pub session_id: Uuid,
    pub agent: String,
    pub model: Option<String>,
    pub alias: Option<String>,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PromptEntry {
    pub id: i64,
    pub sent_at: String,
    pub project: String,
    pub project_path: String,
    pub session: String,
    pub agent: String,
    pub model: Option<String>,
    pub alias: Option<String>,
    pub branch: Option<String>,
    pub message: String,
}

pub fn database_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .context("could not locate the config directory")?
        .join("workbench");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("prompts.sqlite3"))
}

#[cfg(not(test))]
pub fn record_for_session(
    state: &crate::app::AppState,
    session_id: Uuid,
    text: &str,
) -> Result<()> {
    let session = state
        .get_session(session_id)
        .context("prompt target session no longer exists")?;
    let workspace = state
        .get_workspace(session.workspace_id)
        .context("prompt target project no longer exists")?;
    let metadata = PromptMetadata {
        sent_at: Utc::now(),
        workspace_id: workspace.id,
        workspace_name: workspace.name.clone(),
        workspace_path: workspace.path.clone(),
        session_id,
        agent: session.agent_type.display_name(),
        model: Some(
            state
                .session_model(session_id)
                .unwrap_or_else(|| session.agent_type.display_name()),
        ),
        alias: session.alias.clone(),
        branch: session.worktree_branch.clone(),
    };
    record_at(&database_path()?, &metadata, text)
}

// Session-handler tests exercise input delivery heavily. They should never
// seed the developer's real prompt history as a side effect of `cargo test`;
// storage itself is covered through `record_at` against a temporary database.
#[cfg(test)]
pub fn record_for_session(
    _state: &crate::app::AppState,
    _session_id: Uuid,
    _text: &str,
) -> Result<()> {
    Ok(())
}

#[cfg(not(test))]
pub fn record_canvas_prompt(
    workspace_id: &str,
    workspace_name: &str,
    workspace_path: &Path,
    note_id: &str,
    text: &str,
    model: &str,
) -> Result<()> {
    let metadata = PromptMetadata {
        sent_at: Utc::now(),
        workspace_id: workspace_id.parse().context("invalid canvas project id")?,
        workspace_name: workspace_name.to_string(),
        workspace_path: workspace_path.to_path_buf(),
        session_id: note_id.parse().context("invalid canvas note id")?,
        agent: "Claude Code".into(),
        model: Some(crate::models::model_label(model)),
        alias: Some("canvas note".into()),
        branch: None,
    };
    record_at(&database_path()?, &metadata, text)
}

#[cfg(test)]
pub fn record_canvas_prompt(
    _workspace_id: &str,
    _workspace_name: &str,
    _workspace_path: &Path,
    _note_id: &str,
    _text: &str,
    _model: &str,
) -> Result<()> {
    Ok(())
}

fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(2))?;
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS prompts (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             sent_at_ms INTEGER NOT NULL,
             workspace_id TEXT NOT NULL,
             workspace_name TEXT NOT NULL,
             workspace_path TEXT NOT NULL,
             session_id TEXT NOT NULL,
             agent TEXT NOT NULL,
             model TEXT,
             alias TEXT,
             branch TEXT,
             message TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS prompts_by_time ON prompts(sent_at_ms DESC);
         CREATE INDEX IF NOT EXISTS prompts_by_project ON prompts(workspace_id, sent_at_ms DESC);
         CREATE INDEX IF NOT EXISTS prompts_by_model ON prompts(model, sent_at_ms DESC);",
    )?;
    Ok(connection)
}

fn record_at(path: &Path, metadata: &PromptMetadata, text: &str) -> Result<()> {
    let connection = open(path)?;
    connection.execute(
        "INSERT INTO prompts (
             sent_at_ms, workspace_id, workspace_name, workspace_path,
             session_id, agent, model, alias, branch, message
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            metadata.sent_at.timestamp_millis(),
            metadata.workspace_id.to_string(),
            metadata.workspace_name,
            metadata.workspace_path.to_string_lossy(),
            metadata.session_id.to_string(),
            metadata.agent,
            metadata.model,
            metadata.alias,
            metadata.branch,
            text,
        ],
    )?;
    Ok(())
}

pub fn recent(limit: usize) -> Result<Vec<PromptEntry>> {
    recent_at(&database_path()?, limit)
}

fn recent_at(path: &Path, limit: usize) -> Result<Vec<PromptEntry>> {
    let connection = open(path)?;
    let mut statement = connection.prepare(
        "SELECT id, sent_at_ms, workspace_name, workspace_path, session_id,
                agent, model, alias, branch, message
         FROM prompts ORDER BY sent_at_ms DESC, id DESC LIMIT ?1",
    )?;
    let rows = statement.query_map([limit as i64], |row| {
        let sent_at_ms: i64 = row.get(1)?;
        let sent_at = DateTime::<Utc>::from_timestamp_millis(sent_at_ms)
            .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
            .to_rfc3339();
        Ok(PromptEntry {
            id: row.get(0)?,
            sent_at,
            project: row.get(2)?,
            project_path: row.get(3)?,
            session: row.get(4)?,
            agent: row.get(5)?,
            model: row.get(6)?,
            alias: row.get(7)?,
            branch: row.get(8)?,
            message: row.get(9)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Human-readable global analysis used by both the existing Util pane and CLI.
pub fn analysis_lines(recent_limit: usize) -> Result<Vec<String>> {
    analysis_lines_at(&database_path()?, recent_limit)
}

fn analysis_lines_at(path: &Path, recent_limit: usize) -> Result<Vec<String>> {
    let connection = open(path)?;
    let since = (Utc::now() - chrono::TimeDelta::days(7)).timestamp_millis();
    let (total, projects, average, last_week, multiline, questions, verification): (
        i64,
        i64,
        f64,
        i64,
        i64,
        i64,
        i64,
    ) = connection.query_row(
        "SELECT count(*), count(DISTINCT workspace_id), coalesce(avg(length(message)), 0),
                coalesce(sum(sent_at_ms >= ?1), 0),
                coalesce(sum(instr(message, char(10)) > 0), 0),
                coalesce(sum(instr(message, '?') > 0), 0),
                coalesce(sum(lower(message) LIKE '%test%'
                          OR lower(message) LIKE '%verify%'
                          OR lower(message) LIKE '%check%'), 0)
         FROM prompts",
        [since],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        },
    )?;

    let percent = |count: i64| {
        if total == 0 {
            0
        } else {
            ((count as f64 / total as f64) * 100.0).round() as i64
        }
    };
    let mut lines = vec![
        String::new(),
        "  Prompt Log".to_string(),
        "  ==========".to_string(),
        String::new(),
        format!("  {total} messages across {projects} projects · {last_week} in the last 7 days"),
        format!("  Average length: {:.0} characters", average),
        format!(
            "  Shape: {}% multiline · {}% questions · {}% mention tests/checks",
            percent(multiline),
            percent(questions),
            percent(verification)
        ),
    ];

    append_breakdown(
        &connection,
        &mut lines,
        "By project",
        "SELECT workspace_name, count(*) FROM prompts
         GROUP BY workspace_id, workspace_name ORDER BY count(*) DESC, workspace_name LIMIT 8",
    )?;
    append_breakdown(
        &connection,
        &mut lines,
        "By model",
        "SELECT coalesce(model, agent), count(*) FROM prompts
         GROUP BY coalesce(model, agent) ORDER BY count(*) DESC, coalesce(model, agent) LIMIT 8",
    )?;

    lines.push(String::new());
    lines.push("  Recent messages".to_string());
    lines.push("  ---------------".to_string());
    for entry in recent_at(path, recent_limit)? {
        let stamp = DateTime::parse_from_rfc3339(&entry.sent_at)
            .ok()
            .map(|at| at.with_timezone(&Local).format("%b %d %H:%M").to_string())
            .unwrap_or_else(|| entry.sent_at.clone());
        let model = entry.model.as_deref().unwrap_or(&entry.agent);
        lines.push(format!("  {stamp} · {} · {model}", entry.project));
        lines.push(format!("    {}", preview(&entry.message, 110)));
    }
    if total == 0 {
        lines.push("  Messages you submit to an agent will appear here.".to_string());
    }
    lines.push(String::new());
    lines.push(format!("  Database: {}", path.display()));
    Ok(lines)
}

fn append_breakdown(
    connection: &Connection,
    lines: &mut Vec<String>,
    title: &str,
    sql: &str,
) -> Result<()> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.is_empty() {
        return Ok(());
    }
    lines.push(String::new());
    lines.push(format!("  {title}"));
    lines.push(format!("  {}", "-".repeat(title.len())));
    for (label, count) in rows {
        lines.push(format!("  {label:<24} {count:>5}"));
    }
    Ok(())
}

fn preview(text: &str, limit: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= limit {
        return flat;
    }
    let mut out: String = flat.chars().take(limit.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_reconstructs_edits_pastes_and_submission() {
        let mut capture = PromptCapture::default();
        let session = Uuid::new_v4();
        for bytes in [b"h".as_slice(), b"e", b"l", b"o"] {
            assert!(capture.observe(session, bytes).is_none());
        }
        capture.observe(session, b"\x1b[D");
        capture.observe(session, b"l");
        capture.observe(session, b"\x05");
        capture.observe(session, b"\x1b[200~ world\x1b[201~");

        assert_eq!(
            capture.observe(session, b"\r").as_deref(),
            Some("hello world")
        );
        assert!(
            capture.observe(session, b"\r").is_none(),
            "empty Enter is not a prompt"
        );

        capture.observe(session, b"\x1b[A");
        assert_eq!(
            capture.observe(session, b"\r").as_deref(),
            Some("hello world"),
            "a prompt recalled through agent history is still recorded"
        );
    }

    #[test]
    fn prompt_database_keeps_metadata_and_builds_analysis() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("prompts.sqlite3");
        let workspace_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let metadata = PromptMetadata {
            sent_at: Utc::now(),
            workspace_id,
            workspace_name: "Workbench".into(),
            workspace_path: PathBuf::from("/code/workbench"),
            session_id,
            agent: "Claude".into(),
            model: Some("Sonnet 5".into()),
            alias: Some("reviewer".into()),
            branch: Some("main".into()),
        };
        record_at(&path, &metadata, "verify the mobile layout\nand run tests").unwrap();

        let entries = recent_at(&path, 10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].project, "Workbench");
        assert_eq!(entries[0].project_path, "/code/workbench");
        assert_eq!(entries[0].session, session_id.to_string());
        assert_eq!(entries[0].agent, "Claude");
        assert_eq!(entries[0].model.as_deref(), Some("Sonnet 5"));
        assert_eq!(entries[0].alias.as_deref(), Some("reviewer"));
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert_eq!(
            entries[0].message,
            "verify the mobile layout\nand run tests"
        );

        let analysis = analysis_lines_at(&path, 10).unwrap().join("\n");
        assert!(analysis.contains("1 messages across 1 projects"));
        assert!(analysis.contains("Workbench"));
        assert!(analysis.contains("Sonnet 5"));
        assert!(analysis.contains("100% multiline"));
        assert!(analysis.contains("100% mention tests/checks"));
    }
}
