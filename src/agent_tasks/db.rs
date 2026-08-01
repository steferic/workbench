//! Agents that keep their sessions in SQLite: opencode and hermes.
//!
//! Unlike the JSONL agents there is no append offset to advance, so a refresh
//! re-reads the conversation whenever a cheap probe (`change_token`) moves.
//! Everything is opened read-only — these databases belong to the agents, and
//! they may be writing while we read (both run in WAL mode, so concurrent
//! readers are fine).
//!
//! ```text
//! opencode  ~/.local/share/opencode/opencode.db
//!           session(id, directory, time_created)   ← cwd mapping is a column
//!           todo(session_id, content, status, position, time_updated)
//!           message(id, session_id, data) / part(message_id, data)
//!
//! hermes    ~/.hermes/state.db
//!           sessions(id, source, cwd, started_at)  ← cwd is often NULL
//!           messages(session_id, role, content, tool_calls, timestamp)
//!           the task list arrives as `todo` tool calls, optionally merging
//! ```

use chrono::{Duration as ChronoDuration, Utc};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::{epoch, AgentTask, BatchBuilder, Provider, Source, TaskSource, TaskState};

pub(super) fn hermes_db(home: &Path) -> PathBuf {
    home.join(".hermes").join("state.db")
}

pub(super) fn opencode_db(home: &Path) -> PathBuf {
    home.join(".local")
        .join("share")
        .join("opencode")
        .join("opencode.db")
}

/// Read-only handle. Never creates the file: if the agent has not run yet
/// there is simply nothing to show.
fn open(db: &Path) -> Option<Connection> {
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    // The agent may hold a write lock mid-turn; wait briefly rather than
    // dropping the refresh.
    let _ = conn.busy_timeout(std::time::Duration::from_millis(250));
    Some(conn)
}

/// Cheap "did anything change?" probe, so a tick that finds nothing new costs
/// one indexed query instead of rebuilding the list.
pub(super) fn change_token(provider: Provider, db: &Path, session: &str) -> Option<String> {
    let conn = open(db)?;
    let sql = match provider {
        Provider::Hermes => "SELECT count(*), coalesce(max(timestamp), 0) FROM messages WHERE session_id = ?1",
        Provider::OpenCode => {
            "SELECT (SELECT count(*) FROM todo WHERE session_id = ?1), \
             (SELECT coalesce(max(time_updated), 0) FROM todo WHERE session_id = ?1) \
             + (SELECT coalesce(max(time_created), 0) FROM message WHERE session_id = ?1)"
        }
        _ => return None,
    };
    conn.query_row(sql, [session], |row| {
        let count: i64 = row.get(0)?;
        let stamp: f64 = row.get(1)?;
        Ok(format!("{count}:{stamp}"))
    })
    .ok()
}

// ---------------------------------------------------------------------------
// hermes
// ---------------------------------------------------------------------------

/// Pick the hermes conversation belonging to this workbench session.
///
/// Prefer a recorded `cwd` match. Hermes only started recording cwd recently,
/// so fall back to the *oldest* unclaimed CLI session started since we spawned:
/// with several agents launched in order, each then claims the conversation it
/// actually started, rather than every one racing for the newest.
pub(super) fn locate_hermes(
    db: &Path,
    ctx: &TaskSource,
    claimed: &HashSet<String>,
) -> Option<Source> {
    let conn = open(db)?;
    let cutoff = (ctx.started_at - ChronoDuration::minutes(1)).timestamp() as f64;

    let mut stmt = conn
        .prepare(
            "SELECT id, cwd FROM sessions \
             WHERE source = 'cli' AND started_at >= ?1 AND coalesce(archived, 0) = 0 \
             ORDER BY started_at ASC, rowid ASC",
        )
        .ok()?;
    let rows = stmt
        .query_map([cutoff], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .ok()?;

    let cwd = ctx.cwd.to_string_lossy().into_owned();
    let mut fallback: Option<String> = None;
    for row in rows.flatten() {
        let (id, session_cwd) = row;
        let source = Source::DbSession {
            db: db.to_path_buf(),
            session: id.clone(),
        };
        if claimed.contains(&source.key()) {
            continue;
        }
        if session_cwd.as_deref() == Some(cwd.as_str()) {
            return Some(source);
        }
        if session_cwd.is_none() && fallback.is_none() {
            fallback = Some(id);
        }
    }

    fallback.map(|session| Source::DbSession {
        db: db.to_path_buf(),
        session,
    })
}

/// Replay a hermes conversation: user turns become prompts, `todo` tool calls
/// become the task list.
pub(super) fn load_hermes(batches: &mut BatchBuilder, db: &Path, session: &str) {
    let Some(conn) = open(db) else { return };
    let Ok(mut stmt) = conn.prepare(
        "SELECT role, content, tool_calls, timestamp FROM messages \
         WHERE session_id = ?1 AND coalesce(active, 1) = 1 \
         ORDER BY timestamp, id",
    ) else {
        return;
    };
    let rows = stmt.query_map([session], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, f64>(3)?,
        ))
    });
    let Ok(rows) = rows else { return };

    for (role, content, tool_calls, stamp) in rows.flatten() {
        let at = epoch(stamp, false);
        if role == "user" {
            if let Some(text) = content.as_deref() {
                batches.note_prompt(text, at);
            }
            continue;
        }
        let Some(raw) = tool_calls else { continue };
        let Ok(calls) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        for call in calls.as_array().into_iter().flatten() {
            if call.pointer("/function/name").and_then(Value::as_str) != Some("todo") {
                continue;
            }
            let Some(args) = call
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
            else {
                continue;
            };
            let Some(todos) = args.get("todos").and_then(Value::as_array) else {
                continue;
            };
            let tasks: Vec<AgentTask> = todos
                .iter()
                .enumerate()
                .filter_map(|(i, todo)| {
                    let subject = todo.get("content").and_then(Value::as_str)?;
                    Some(AgentTask {
                        id: todo
                            .get("id")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .unwrap_or_else(|| (i + 1).to_string()),
                        subject: subject.to_string(),
                        state: todo
                            .get("status")
                            .and_then(Value::as_str)
                            .map(TaskState::parse)
                            .unwrap_or(TaskState::Pending),
                    })
                })
                .collect();

            // `merge: true` patches the named entries; otherwise the call
            // carries the whole list.
            if args.get("merge").and_then(Value::as_bool) == Some(true) {
                batches.merge_tasks(tasks, at);
            } else {
                batches.replace_tasks(tasks, at);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// opencode
// ---------------------------------------------------------------------------

/// opencode records the working directory on the session row, so mapping is
/// exact; the claim check only breaks ties between two agents in one project.
pub(super) fn locate_opencode(
    db: &Path,
    ctx: &TaskSource,
    claimed: &HashSet<String>,
) -> Option<Source> {
    let conn = open(db)?;
    // Session times are epoch milliseconds.
    let cutoff = (ctx.started_at - ChronoDuration::minutes(1)).timestamp_millis();

    let ids: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT id FROM session \
                 WHERE directory = ?1 AND time_created >= ?2 AND time_archived IS NULL \
                 ORDER BY time_created DESC",
            )
            .ok()?;
        let rows = stmt
            .query_map(
                rusqlite::params![ctx.cwd.to_string_lossy().as_ref(), cutoff],
                |row| row.get::<_, String>(0),
            )
            .ok()?;
        rows.flatten().collect()
    };

    ids.into_iter()
        .map(|session| Source::DbSession {
            db: db.to_path_buf(),
            session,
        })
        .find(|source| !claimed.contains(&source.key()))
}

/// opencode keeps only the *current* todo list per session (no history), so
/// this is one batch: the list, under the prompt that was last sent before it
/// was written.
pub(super) fn load_opencode(batches: &mut BatchBuilder, db: &Path, session: &str) {
    let Some(conn) = open(db) else { return };

    let Ok(mut stmt) = conn.prepare(
        "SELECT content, status, time_updated FROM todo WHERE session_id = ?1 ORDER BY position",
    ) else {
        return;
    };
    let rows = stmt.query_map([session], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    });
    let Ok(rows) = rows else { return };

    let mut tasks = Vec::new();
    let mut written_at = 0i64;
    for (i, (content, status, updated)) in rows.flatten().enumerate() {
        written_at = written_at.max(updated);
        tasks.push(AgentTask {
            id: (i + 1).to_string(),
            subject: content,
            state: TaskState::parse(&status),
        });
    }

    // The prompt that produced it: the last user turn at or before the list was
    // written (falling back to the latest one, for a list written mid-turn).
    if let Some((prompt, at)) = last_prompt(&conn, session, written_at) {
        batches.note_prompt(&prompt, at);
    }
    if !tasks.is_empty() {
        batches.replace_tasks(tasks, epoch(written_at as f64, true));
    }
}

/// Text of the newest user message no later than `before` (0 = no bound).
fn last_prompt(
    conn: &Connection,
    session: &str,
    before: i64,
) -> Option<(String, Option<chrono::DateTime<Utc>>)> {
    let mut stmt = conn
        .prepare(
            "SELECT m.id, m.data, m.time_created FROM message m \
             WHERE m.session_id = ?1 AND (?2 = 0 OR m.time_created <= ?2) \
             ORDER BY m.time_created DESC",
        )
        .ok()?;
    let rows = stmt
        .query_map(rusqlite::params![session, before], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .ok()?;

    for (message_id, data, created) in rows.flatten() {
        let role = serde_json::from_str::<Value>(&data)
            .ok()
            .and_then(|v| v.get("role").and_then(Value::as_str).map(str::to_string));
        if role.as_deref() != Some("user") {
            continue;
        }
        // The text lives in the message's parts.
        let Ok(mut parts) = conn.prepare(
            "SELECT data FROM part WHERE message_id = ?1 ORDER BY time_created, id",
        ) else {
            return None;
        };
        let texts = parts
            .query_map([&message_id], |row| row.get::<_, String>(0))
            .ok()?
            .flatten()
            .filter_map(|raw| {
                let part = serde_json::from_str::<Value>(&raw).ok()?;
                (part.get("type").and_then(Value::as_str) == Some("text"))
                    .then(|| part.get("text").and_then(Value::as_str).map(str::to_string))
                    .flatten()
            })
            .collect::<Vec<_>>();
        if !texts.is_empty() {
            return Some((texts.join(" "), epoch(created as f64, true)));
        }
    }
    None
}
