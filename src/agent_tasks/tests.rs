//! Tests for reading agent task lists.
//!
//! File-log providers are driven line by line; SQLite providers get fixture
//! databases built with the real schemas.

use super::*;
use chrono::{Duration as ChronoDuration, Local};
use std::fs;

fn feed(tracker: &mut TaskTracker, lines: &[&str]) {
    for line in lines {
        tracker.ingest_line(&serde_json::from_str(line).unwrap());
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

/// Two agents in one project write two logs for the same cwd. Without
/// claiming, both sessions resolve to whichever is newest — they mirror
/// each other's tasks and, worse, would resume each other's conversation.
#[test]
fn a_claimed_log_is_not_handed_to_a_second_session() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();

    // Rollouts live under sessions/YYYY/MM/DD, by local date.
    let today = Local::now().date_naive();
    let day_dir = files::codex_sessions_root(home.path())
        .join(today.format("%Y").to_string())
        .join(today.format("%m").to_string())
        .join(today.format("%d").to_string());
    fs::create_dir_all(&day_dir).unwrap();

    let meta = |id: &str| {
        format!(
            "{}\n",
            serde_json::json!({
                "type": "session_meta",
                "payload": {"session_id": id, "cwd": cwd.path().to_string_lossy()}
            })
        )
    };
    let first_log = day_dir.join("rollout-a.jsonl");
    let second_log = day_dir.join("rollout-b.jsonl");
    fs::write(&first_log, meta("conv-a")).unwrap();
    fs::write(&second_log, meta("conv-b")).unwrap();

    let ctx = TaskSource {
        provider: Provider::Codex,
        session_uuid: "x".into(),
        cwd: cwd.path().to_path_buf(),
        started_at: Utc::now() - ChronoDuration::hours(1),
    };

    let root = files::codex_sessions_root(home.path());
    let first = files::locate_codex(&root, &ctx, &HashSet::new()).expect("a log for this cwd");

    // With that one claimed, a second session must land on the other log.
    let mut claimed: HashSet<String> = HashSet::from([first.to_string_lossy().into_owned()]);
    let second = files::locate_codex(&root, &ctx, &claimed).expect("the other log");
    assert_ne!(first, second);

    // Both claimed: nothing left to hand out, rather than a duplicate.
    claimed.insert(second.to_string_lossy().into_owned());
    assert!(files::locate_codex(&root, &ctx, &claimed).is_none());
}

#[test]
fn a_pinned_claude_log_wins_over_the_cwd_scan() {
    let home = tempfile::tempdir().unwrap();
    let projects = files::claude_projects_root(home.path()).join("-tmp-project");
    fs::create_dir_all(&projects).unwrap();

    let uuid = "11111111-2222-4333-8444-555555555555";
    let pinned = projects.join(format!("{uuid}.jsonl"));
    fs::write(&pinned, "").unwrap();
    // A newer log for the same cwd that must NOT be chosen.
    fs::write(
        projects.join("99999999-2222-4333-8444-555555555555.jsonl"),
        format!(
            "{}\n",
            serde_json::json!({"type": "user", "cwd": "/tmp/project"})
        ),
    )
    .unwrap();

    let ctx = TaskSource {
        provider: Provider::Claude,
        session_uuid: uuid.into(),
        cwd: PathBuf::from("/tmp/project"),
        started_at: Utc::now() - ChronoDuration::hours(1),
    };

    let found = files::locate_claude(&files::claude_projects_root(home.path()), &ctx, &HashSet::new());
    assert_eq!(found.as_deref(), Some(pinned.as_path()));
}

#[test]
fn the_resolved_log_yields_the_conversation_id_to_resume() {
    let dir = tempfile::tempdir().unwrap();

    // Claude names the log after the conversation.
    let claude_log = dir.path().join("2f1e3d4c-0000-4000-8000-000000000001.jsonl");
    fs::write(&claude_log, "").unwrap();
    let claude = TaskTracker::with_source(Provider::Claude, Source::File(claude_log));
    assert_eq!(
        claude.provider_session_id().as_deref(),
        Some("2f1e3d4c-0000-4000-8000-000000000001")
    );

    // Codex records it in the rollout's session_meta.
    let codex_log = dir.path().join("rollout-2026-07-25T10-00-00-abc.jsonl");
    fs::write(
        &codex_log,
        format!(
            "{}\n",
            serde_json::json!({
                "type": "session_meta",
                "payload": {"session_id": "019f950b-446f-7bd0", "cwd": "/tmp"}
            })
        ),
    )
    .unwrap();
    let codex = TaskTracker::with_source(Provider::Codex, Source::File(codex_log));
    assert_eq!(
        codex.provider_session_id().as_deref(),
        Some("019f950b-446f-7bd0")
    );
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
    t.source = Some(Source::File(path.clone()));
    t.refresh(&ctx, &HashSet::new());
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

    t.refresh(&ctx, &HashSet::new());
    assert_eq!(t.current().unwrap().tasks.len(), 2);
    assert_eq!(t.current().unwrap().tasks[0].state, TaskState::Completed);
    // The torn line was left unconsumed for the next pass.
    assert!(t.offset > after_first);
    assert!(t.offset < fs::metadata(&path).unwrap().len());
}

// ---------------------------------------------------------------------------
// SQLite providers
// ---------------------------------------------------------------------------

/// A hermes `state.db` with the columns we read, matching the real schema.
fn hermes_fixture(dir: &Path) -> PathBuf {
    let db = dir.join("state.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE sessions (
            id TEXT PRIMARY KEY, source TEXT NOT NULL, cwd TEXT,
            started_at REAL NOT NULL, archived INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL,
            role TEXT NOT NULL, content TEXT, tool_calls TEXT,
            timestamp REAL NOT NULL, active INTEGER NOT NULL DEFAULT 1
         );",
    )
    .unwrap();
    db
}

/// One hermes `todo` tool call, as it appears in `messages.tool_calls`.
fn hermes_todo_call(todos: &[(&str, &str, &str)], merge: bool) -> String {
    let list: Vec<serde_json::Value> = todos
        .iter()
        .map(|(id, content, status)| {
            serde_json::json!({"id": id, "content": content, "status": status})
        })
        .collect();
    let mut args = serde_json::json!({"todos": list});
    if merge {
        args["merge"] = serde_json::Value::Bool(true);
    }
    serde_json::json!([{
        "id": "call_1", "type": "function",
        "function": {"name": "todo", "arguments": args.to_string()}
    }])
    .to_string()
}

#[test]
fn hermes_todo_calls_become_a_task_list_under_their_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let db = hermes_fixture(dir.path());
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute(
        "INSERT INTO sessions (id, source, cwd, started_at) VALUES ('s1', 'cli', NULL, 1000)",
        [],
    )
    .unwrap();
    let insert = |role: &str, content: Option<&str>, calls: Option<&str>, ts: f64| {
        conn.execute(
            "INSERT INTO messages (session_id, role, content, tool_calls, timestamp) \
             VALUES ('s1', ?1, ?2, ?3, ?4)",
            rusqlite::params![role, content, calls, ts],
        )
        .unwrap();
    };
    insert("user", Some("wire up the calendar"), None, 1001.0);
    insert(
        "assistant",
        None,
        Some(&hermes_todo_call(
            &[
                ("inspect", "Inspect existing files", "in_progress"),
                ("tests", "Write failing tests", "pending"),
            ],
            false,
        )),
        1002.0,
    );
    // A merging call patches only what it names.
    insert(
        "assistant",
        None,
        Some(&hermes_todo_call(&[("inspect", "Inspect existing files", "completed")], true)),
        1003.0,
    );

    let mut builder = BatchBuilder::default();
    db::load_hermes(&mut builder, &db, "s1");

    assert_eq!(builder.batches.len(), 1);
    let batch = &builder.batches[0];
    assert_eq!(batch.prompt, "wire up the calendar");
    assert_eq!(batch.tasks.len(), 2, "merge must not drop the untouched task");
    assert_eq!(batch.tasks[0].id, "inspect");
    assert_eq!(batch.tasks[0].state, TaskState::Completed);
    assert_eq!(batch.tasks[1].state, TaskState::Pending);
}

#[test]
fn hermes_prefers_a_cwd_match_and_never_reuses_a_claimed_session() {
    let dir = tempfile::tempdir().unwrap();
    let db = hermes_fixture(dir.path());
    let conn = rusqlite::Connection::open(&db).unwrap();
    let started = Utc::now().timestamp() as f64;
    conn.execute(
        "INSERT INTO sessions (id, source, cwd, started_at) VALUES \
         ('older', 'cli', NULL, ?1), ('match', 'cli', '/tmp/project', ?1), \
         ('newer', 'cli', NULL, ?1), ('a-cron', 'cron', NULL, ?1)",
        [started],
    )
    .unwrap();

    let ctx = TaskSource {
        provider: Provider::Hermes,
        session_uuid: "x".into(),
        cwd: PathBuf::from("/tmp/project"),
        started_at: Utc::now() - ChronoDuration::minutes(1),
    };

    // An exact cwd match wins over any newer session.
    let found = db::locate_hermes(&db, &ctx, &HashSet::new()).unwrap();
    assert_eq!(
        found,
        Source::DbSession {
            db: db.clone(),
            session: "match".into()
        }
    );

    // With it claimed, fall back to a cwd-less CLI session — and never the
    // same one twice.
    let mut claimed = HashSet::from([found.key()]);
    let second = db::locate_hermes(&db, &ctx, &claimed).unwrap();
    assert_ne!(second, found);
    claimed.insert(second.key());
    let third = db::locate_hermes(&db, &ctx, &claimed).unwrap();
    assert!(!claimed.contains(&third.key()));

    // Non-CLI sessions (cron, whatsapp) are never adopted.
    claimed.insert(third.key());
    assert!(db::locate_hermes(&db, &ctx, &claimed).is_none());
}

/// An opencode `opencode.db` with the columns we read.
fn opencode_fixture(dir: &Path) -> PathBuf {
    let db = dir.join("opencode.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE session (
            id TEXT PRIMARY KEY, directory TEXT NOT NULL,
            time_created INTEGER NOT NULL, time_archived INTEGER
         );
         CREATE TABLE todo (
            session_id TEXT NOT NULL, content TEXT NOT NULL, status TEXT NOT NULL,
            priority TEXT NOT NULL, position INTEGER NOT NULL,
            time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL
         );
         CREATE TABLE message (
            id TEXT PRIMARY KEY, session_id TEXT NOT NULL,
            time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL
         );
         CREATE TABLE part (
            id TEXT PRIMARY KEY, message_id TEXT NOT NULL, session_id TEXT NOT NULL,
            time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL
         );",
    )
    .unwrap();
    db
}

#[test]
fn opencode_todo_rows_become_the_task_list_with_its_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let db = opencode_fixture(dir.path());
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute(
        "INSERT INTO session (id, directory, time_created) VALUES ('ses_1', '/tmp/p', 1000)",
        [],
    )
    .unwrap();
    // Two user turns; the task list was written after the first one.
    for (id, created, text) in [("m1", 1100i64, "add the parser"), ("m2", 9000, "later ask")] {
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data) \
             VALUES (?1, 'ses_1', ?2, ?2, ?3)",
            rusqlite::params![id, created, r#"{"role":"user"}"#],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO part (id, message_id, session_id, time_created, time_updated, data) \
             VALUES (?1, ?2, 'ses_1', ?3, ?3, ?4)",
            rusqlite::params![
                format!("prt_{id}"),
                id,
                created,
                serde_json::json!({"type": "text", "text": text}).to_string()
            ],
        )
        .unwrap();
    }
    for (pos, content, status) in [
        (0, "Read the grammar", "completed"),
        (1, "Write the parser", "in_progress"),
        (2, "Add tests", "pending"),
    ] {
        conn.execute(
            "INSERT INTO todo (session_id, content, status, priority, position, time_created, time_updated) \
             VALUES ('ses_1', ?1, ?2, 'medium', ?3, 1200, 1200)",
            rusqlite::params![content, status, pos],
        )
        .unwrap();
    }

    let mut builder = BatchBuilder::default();
    db::load_opencode(&mut builder, &db, "ses_1");

    assert_eq!(builder.batches.len(), 1);
    let batch = &builder.batches[0];
    // The prompt in flight when the list was written, not the newest one.
    assert_eq!(batch.prompt, "add the parser");
    let subjects: Vec<&str> = batch.tasks.iter().map(|t| t.subject.as_str()).collect();
    assert_eq!(
        subjects,
        vec!["Read the grammar", "Write the parser", "Add tests"],
        "tasks must keep their stored order"
    );
    assert_eq!(batch.tasks[0].state, TaskState::Completed);
    assert_eq!(batch.tasks[1].state, TaskState::InProgress);
    assert_eq!(batch.completed(), 1);
}

#[test]
fn opencode_maps_a_session_by_its_directory() {
    let dir = tempfile::tempdir().unwrap();
    let db = opencode_fixture(dir.path());
    let conn = rusqlite::Connection::open(&db).unwrap();
    let now = Utc::now().timestamp_millis();
    conn.execute(
        "INSERT INTO session (id, directory, time_created) VALUES \
         ('ses_here', '/tmp/p', ?1), ('ses_other', '/tmp/elsewhere', ?1), ('ses_here2', '/tmp/p', ?1)",
        [now],
    )
    .unwrap();

    let ctx = TaskSource {
        provider: Provider::OpenCode,
        session_uuid: "x".into(),
        cwd: PathBuf::from("/tmp/p"),
        started_at: Utc::now() - ChronoDuration::minutes(1),
    };

    let first = db::locate_opencode(&db, &ctx, &HashSet::new()).unwrap();
    let claimed = HashSet::from([first.key()]);
    let second = db::locate_opencode(&db, &ctx, &claimed).unwrap();
    assert_ne!(second, first, "a claimed session must not be handed out twice");

    // Sessions in another directory are never adopted.
    let both = HashSet::from([first.key(), second.key()]);
    assert!(db::locate_opencode(&db, &ctx, &both).is_none());
}

#[test]
fn a_db_sessions_conversation_id_is_what_resume_needs() {
    let tracker = TaskTracker::with_source(
        Provider::Hermes,
        Source::DbSession {
            db: PathBuf::from("/tmp/state.db"),
            session: "20260725_101010_abc".into(),
        },
    );
    assert_eq!(
        tracker.provider_session_id().as_deref(),
        Some("20260725_101010_abc")
    );
}

