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
    let batch = t.batches().last().unwrap();
    assert_eq!(batch.tasks.len(), 2);
    assert_eq!(batch.tasks[0].id, "1");
    assert_eq!(batch.tasks[0].state, TaskState::Completed);
    assert_eq!(batch.tasks[1].state, TaskState::InProgress);
    assert_eq!(
        batch.tasks.iter().filter(|t| t.state == TaskState::Completed).count(),
        1
    );
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
    assert_eq!(t.batches()[0].tasks[0].state, TaskState::Completed);
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
    assert_eq!(t.batches().last().unwrap().tasks.len(), 1);
    assert_eq!(t.batches().last().unwrap().tasks[0].subject, "real task");
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
    let batch = t.batches().last().unwrap();
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

/// gpt-5.6-era rollouts carry the prompt as a completed item, not as a
/// user_message event. A new prompt must still open a new batch.
fn codex_user_item(text: &str) -> String {
    serde_json::json!({
        "type": "event_msg",
        "timestamp": "2026-08-27T10:00:00.000Z",
        "payload": {"type": "item_completed", "item": {
            "type": "UserMessage", "id": "u1",
            "content": [{"type": "text", "text": text}]
        }}
    })
    .to_string()
}

#[test]
fn a_new_format_prompt_still_opens_a_batch() {
    let mut t = TaskTracker::new(Provider::Codex);
    feed(
        &mut t,
        &[
            &codex_user_item("audit the repo"),
            &codex_plan(vec![("Map repo", "in_progress")]),
            &codex_user_item("now fix the worst one"),
            &codex_plan(vec![("Fix hotspot", "in_progress")]),
        ],
    );
    assert_eq!(t.batches().len(), 2, "each spoken prompt starts its batch");
    assert_eq!(t.batches()[1].tasks[0].subject, "Fix hotspot");
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
    assert_eq!(t.batches()[0].tasks.len(), 2);
    assert_eq!(t.batches()[0].tasks[0].state, TaskState::Completed);
    assert_eq!(t.batches()[1].tasks.len(), 1);
}

#[test]
fn tasks_without_a_preceding_prompt_still_land_somewhere() {
    let mut t = TaskTracker::new(Provider::Codex);
    feed(&mut t, &[&codex_plan(vec![("Orphan step", "pending")])]);
    assert_eq!(t.batches().len(), 1);
    assert_eq!(t.batches().last().unwrap().tasks.len(), 1);
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
        conversation: None,
        spawned_at: None,
        reported: None,
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
        conversation: None,
        spawned_at: None,
        reported: None,
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

    // Codex names the rollout after the conversation `codex resume <id>`
    // reopens. Resuming forks a NEW rollout whose `session_meta.session_id` is
    // the lineage root, so taking the id from the metadata would rewind the
    // agent past everything it did since that fork.
    let root = "019f4ca0-d760-7610-a9a8-427c266d1cfc";
    let fork = "019f6212-1d59-79e2-a71a-7d92e739d58c";
    let codex_log = dir.path().join(format!("rollout-2026-07-25T10-00-00-{fork}.jsonl"));
    fs::write(
        &codex_log,
        format!(
            "{}\n",
            serde_json::json!({
                "type": "session_meta",
                "payload": {
                    "session_id": root,
                    "forked_from_id": root,
                    "parent_thread_id": root,
                    "cwd": "/tmp"
                }
            })
        ),
    )
    .unwrap();
    let codex = TaskTracker::with_source(Provider::Codex, Source::File(codex_log));
    assert_eq!(codex.provider_session_id().as_deref(), Some(fork));
}

#[test]
fn a_rollout_id_is_only_taken_from_a_well_formed_name() {
    let id = "019f6212-1d59-79e2-a71a-7d92e739d58c";
    assert_eq!(
        files::codex_rollout_id(Path::new(&format!(
            "/s/rollout-2026-07-25T10-00-00-{id}.jsonl"
        )))
        .as_deref(),
        Some(id)
    );
    // Nothing uuid-shaped on the end: better no id than a wrong one, which
    // would resume some other conversation.
    assert_eq!(files::codex_rollout_id(Path::new("/s/rollout-abc.jsonl")), None);
    assert_eq!(files::codex_rollout_id(Path::new("/s/short.jsonl")), None);
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
        conversation: None,
        spawned_at: None,
        reported: None,
    };
    let mut t = TaskTracker::new(Provider::Codex);
    t.source = Some(Source::File(path.clone()));
    t.refresh(&ctx, &HashSet::new());
    assert_eq!(t.batches().last().unwrap().tasks.len(), 1);
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
    assert_eq!(t.batches().last().unwrap().tasks.len(), 2);
    assert_eq!(t.batches().last().unwrap().tasks[0].state, TaskState::Completed);
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
        conversation: None,
        spawned_at: None,
        reported: None,
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
    let subjects: Vec<&str> = batch.tasks.iter().map(|t| t.subject.as_str()).collect();
    assert_eq!(
        subjects,
        vec!["Read the grammar", "Write the parser", "Add tests"],
        "tasks must keep their stored order"
    );
    assert_eq!(batch.tasks[0].state, TaskState::Completed);
    assert_eq!(batch.tasks[1].state, TaskState::InProgress);
    assert_eq!(
        batch.tasks.iter().filter(|t| t.state == TaskState::Completed).count(),
        1
    );
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
        conversation: None,
        spawned_at: None,
        reported: None,
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


// ---------------------------------------------------------------------------
// Recognising the rollout a *running* codex process owns
// ---------------------------------------------------------------------------

/// Today's rollout directory under `home`, created.
fn codex_day_dir(home: &Path) -> PathBuf {
    let today = Local::now().date_naive();
    let dir = files::codex_sessions_root(home)
        .join(today.format("%Y").to_string())
        .join(today.format("%m").to_string())
        .join(today.format("%d").to_string());
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The model every codex fixture rollout says it is answering with.
const FIXTURE_CODEX_MODEL: &str = "gpt-5.6-sol";

/// A rollout whose `session_meta` says when it was opened and for which cwd,
/// last written at `created` — a conversation nobody has resumed.
fn codex_rollout(dir: &Path, id: &str, cwd: &Path, created: DateTime<Utc>) -> PathBuf {
    codex_rollout_touched(dir, id, cwd, created, created)
}

/// The same, but appended to since — which is what a resumed conversation
/// looks like on disk, and the only thing that distinguishes it from an
/// abandoned one.
fn codex_rollout_touched(
    dir: &Path,
    id: &str,
    cwd: &Path,
    created: DateTime<Utc>,
    touched: DateTime<Utc>,
) -> PathBuf {
    let path = dir.join(format!("rollout-2026-07-26T00-00-00-{id}.jsonl"));
    // A real rollout opens with `session_meta` and then names its model in a
    // `turn_context` ahead of the first turn. Both lines matter: `locate_codex`
    // reads only the first, and the model is only ever on the second — a
    // fixture with just the header cannot tell the two apart.
    fs::write(
        &path,
        format!(
            "{}\n{}\n",
            serde_json::json!({
                "timestamp": created.to_rfc3339(),
                "type": "session_meta",
                "payload": {"session_id": id, "cwd": cwd.to_string_lossy()}
            }),
            serde_json::json!({
                "timestamp": created.to_rfc3339(),
                "type": "turn_context",
                "payload": {"model": FIXTURE_CODEX_MODEL, "effort": "xhigh"}
            })
        ),
    )
    .unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(std::time::SystemTime::from(touched))
        .unwrap();
    path
}

fn codex_ctx(cwd: &Path, spawned_at: Option<DateTime<Utc>>) -> TaskSource {
    TaskSource {
        provider: Provider::Codex,
        session_uuid: "x".into(),
        cwd: cwd.to_path_buf(),
        started_at: Utc::now() - ChronoDuration::hours(6),
        conversation: None,
        spawned_at,
        reported: None,
    }
}

#[test]
fn a_restarted_codex_session_takes_the_rollout_its_new_process_opened() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let dir = codex_day_dir(home.path());
    let root = files::codex_sessions_root(home.path());

    // The conversation this session had before the restart...
    let before = codex_rollout(
        &dir,
        "00000000-0000-4000-8000-00000000aaaa",
        cwd.path(),
        Utc::now() - ChronoDuration::hours(2),
    );
    // ...and the fork its new process opened. Resuming always forks, so the
    // pre-restart rollout still exists and is still the newest *by name*.
    let spawned_at = Utc::now() - ChronoDuration::seconds(30);
    let after = codex_rollout(
        &dir,
        "00000000-0000-4000-8000-00000000bbbb",
        cwd.path(),
        spawned_at + ChronoDuration::seconds(1),
    );

    let found = files::locate_codex(&root, &codex_ctx(cwd.path(), Some(spawned_at)), &HashSet::new());
    assert_eq!(
        found.as_deref(),
        Some(after.as_path()),
        "must follow the running process, not the conversation it left"
    );
    assert!(before.exists(), "the old rollout is still on disk");
}

#[test]
fn two_codex_agents_in_one_project_get_the_rollout_each_one_opened() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let dir = codex_day_dir(home.path());
    let root = files::codex_sessions_root(home.path());

    // Two agents spawned a few seconds apart in the same directory; each
    // rollout appears just after its own spawn.
    let first_spawn = Utc::now() - ChronoDuration::seconds(60);
    let second_spawn = Utc::now() - ChronoDuration::seconds(20);
    let first_log = codex_rollout(
        &dir,
        "00000000-0000-4000-8000-000000000001",
        cwd.path(),
        first_spawn + ChronoDuration::seconds(1),
    );
    let second_log = codex_rollout(
        &dir,
        "00000000-0000-4000-8000-000000000002",
        cwd.path(),
        second_spawn + ChronoDuration::seconds(1),
    );

    // Resolved in spawn order, as `handler::refresh_agent_tasks` does.
    let mut claimed = HashSet::new();
    let first = files::locate_codex(&root, &codex_ctx(cwd.path(), Some(first_spawn)), &claimed)
        .expect("a rollout for the first agent");
    claimed.insert(first.to_string_lossy().into_owned());
    let second = files::locate_codex(&root, &codex_ctx(cwd.path(), Some(second_spawn)), &claimed)
        .expect("a rollout for the second agent");

    assert_eq!(first.as_path(), first_log.as_path());
    assert_eq!(second.as_path(), second_log.as_path());
}

#[test]
fn a_rollout_opened_before_this_process_is_never_claimed_by_it() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let dir = codex_day_dir(home.path());
    let root = files::codex_sessions_root(home.path());

    codex_rollout(
        &dir,
        "00000000-0000-4000-8000-00000000cccc",
        cwd.path(),
        Utc::now() - ChronoDuration::minutes(10),
    );

    let spawned_at = Utc::now();
    assert!(
        files::locate_codex(&root, &codex_ctx(cwd.path(), Some(spawned_at)), &HashSet::new())
            .is_none(),
        "a rollout nobody has written to since we spawned belongs to some other session"
    );
}

/// The failure this guards: codex 0.146 appends to the rollout it resumes
/// instead of forking a new one, so after a workbench restart every codex
/// session was writing to a file opened days ago — and nothing that looked
/// only at creation time could find it. The pane and the phone both went
/// blank for codex while claude was fine.
#[test]
fn a_resumed_codex_session_keeps_writing_to_the_rollout_it_reopened() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let dir = codex_day_dir(home.path());
    let root = files::codex_sessions_root(home.path());
    let spawned_at = Utc::now() - ChronoDuration::seconds(30);

    let reopened = codex_rollout_touched(
        &dir,
        "00000000-0000-4000-8000-0000000000aa",
        cwd.path(),
        Utc::now() - ChronoDuration::days(11),
        Utc::now(),
    );
    // An older conversation for the same directory, left alone.
    codex_rollout(
        &dir,
        "00000000-0000-4000-8000-0000000000bb",
        cwd.path(),
        Utc::now() - ChronoDuration::days(2),
    );

    let found = files::locate_codex(&root, &codex_ctx(cwd.path(), Some(spawned_at)), &HashSet::new());
    assert_eq!(found.as_deref(), Some(reopened.as_path()));
}

/// And once the id is known, none of that guessing is needed — even for a
/// rollout filed under a day we would never have scanned.
#[test]
fn a_resumed_codex_session_is_found_by_the_conversation_it_resumed() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let root = files::codex_sessions_root(home.path());
    let old_day = root.join("2026").join("07").join("23");
    fs::create_dir_all(&old_day).unwrap();

    let conversation = "00000000-0000-4000-8000-0000000000cc";
    let resumed = codex_rollout(
        &old_day,
        conversation,
        cwd.path(),
        Utc::now() - ChronoDuration::days(11),
    );
    // A newer conversation for the same cwd, which the search would prefer.
    codex_rollout_touched(
        &codex_day_dir(home.path()),
        "00000000-0000-4000-8000-0000000000dd",
        cwd.path(),
        Utc::now(),
        Utc::now(),
    );

    let mut ctx = codex_ctx(cwd.path(), Some(Utc::now() - ChronoDuration::seconds(30)));
    ctx.conversation = Some(conversation.to_string());

    let found = files::locate_codex(&root, &ctx, &HashSet::new());
    assert_eq!(
        found.as_deref(),
        Some(resumed.as_path()),
        "the rollout we named on the command line is the one it is writing to"
    );
}

#[test]
fn a_resumed_claude_session_is_found_by_the_conversation_it_resumed() {
    let home = tempfile::tempdir().unwrap();
    let projects = files::claude_projects_root(home.path()).join("-tmp-project");
    fs::create_dir_all(&projects).unwrap();

    // Claude appends to the conversation it resumed, so the log keeps the
    // conversation's name — not this workbench session's uuid.
    let conversation = "aaaaaaaa-2222-4333-8444-555555555555";
    let resumed = projects.join(format!("{conversation}.jsonl"));
    fs::write(&resumed, "").unwrap();
    // A newer log for the same cwd that the cwd fallback would have picked.
    fs::write(
        projects.join("bbbbbbbb-2222-4333-8444-555555555555.jsonl"),
        format!(
            "{}\n",
            serde_json::json!({"type": "user", "cwd": "/tmp/project"})
        ),
    )
    .unwrap();

    let ctx = TaskSource {
        provider: Provider::Claude,
        session_uuid: "99999999-2222-4333-8444-555555555555".into(),
        cwd: PathBuf::from("/tmp/project"),
        started_at: Utc::now() - ChronoDuration::hours(1),
        conversation: Some(conversation.to_string()),
        spawned_at: Some(Utc::now()),
        reported: None,
    };

    let found = files::locate_claude(&files::claude_projects_root(home.path()), &ctx, &HashSet::new());
    assert_eq!(found.as_deref(), Some(resumed.as_path()));
}

#[test]
fn a_rollout_with_no_opening_timestamp_is_skipped_not_fatal() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let dir = codex_day_dir(home.path());
    let root = files::codex_sessions_root(home.path());
    let spawned_at = Utc::now() - ChronoDuration::seconds(30);

    // A malformed rollout for the same cwd, listed before the real one.
    fs::write(
        dir.join("rollout-2026-07-26T00-00-00-00000000-0000-4000-8000-00000000dddd.jsonl"),
        format!(
            "{}\n",
            serde_json::json!({
                "type": "session_meta",
                "payload": {"cwd": cwd.path().to_string_lossy()}
            })
        ),
    )
    .unwrap();
    let good = codex_rollout(
        &dir,
        "00000000-0000-4000-8000-00000000eeee",
        cwd.path(),
        spawned_at + ChronoDuration::seconds(1),
    );

    let found = files::locate_codex(&root, &codex_ctx(cwd.path(), Some(spawned_at)), &HashSet::new());
    assert_eq!(found.as_deref(), Some(good.as_path()));
}


// ---------------------------------------------------------------------------
// Following the journal the agent says it is writing
// ---------------------------------------------------------------------------

/// The scenario found live: an agent resumed as conversation A, then `/clear`
/// rotated its transcript to a new session id, B. A's file still exists — it
/// just stops growing — so both the resumed-id branch of `locate_claude` and
/// the tracker's "source is still there" check hold onto it forever, and the
/// phone shows a conversation the desktop's terminal has long since left.
/// The hook report is the agent saying "I write to B now", and it must win.
#[test]
fn a_cleared_agent_is_followed_to_its_new_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("aaaaaaaa-resumed.jsonl");
    let new = dir.path().join("bbbbbbbb-cleared.jsonl");
    fs::write(
        &old,
        format!("{}\n", claude_assistant_with_model("claude-opus-5")),
    )
    .unwrap();

    let mut ctx = TaskSource {
        provider: Provider::Claude,
        session_uuid: "not-on-disk".into(),
        cwd: dir.path().to_path_buf(),
        started_at: Utc::now(),
        conversation: Some("aaaaaaaa-resumed".into()),
        spawned_at: None,
        reported: None,
    };

    // Before any hook has spoken, the tracker sits on the resumed file.
    let mut tracker = TaskTracker::new(Provider::Claude);
    tracker.source = Some(Source::File(old.clone()));
    tracker.refresh(&ctx, &HashSet::new());
    assert_eq!(tracker.source(), Some(&Source::File(old.clone())));
    assert_eq!(tracker.model(), Some("claude-opus-5"));

    // /clear: a new file, and the next hook names it. The old file is intact.
    fs::write(
        &new,
        format!("{}\n", claude_assistant_with_model("claude-sonnet-5")),
    )
    .unwrap();
    ctx.reported = Some(new.clone());
    tracker.refresh(&ctx, &HashSet::new());
    assert_eq!(
        tracker.source(),
        Some(&Source::File(new.clone())),
        "the agent said where it writes now, and the old file existing is no reason to stay"
    );
    // And everything downstream reads the new conversation, not the old.
    assert_eq!(tracker.model(), Some("claude-sonnet-5"));

    // A report pointing at a file that is gone is not followed off a cliff.
    ctx.reported = Some(dir.path().join("never-existed.jsonl"));
    tracker.refresh(&ctx, &HashSet::new());
    assert_eq!(
        tracker.source(),
        Some(&Source::File(new)),
        "a stale report falls back to what is known rather than unseating it"
    );
}

// ---------------------------------------------------------------------------
// Which model a session is answering with
// ---------------------------------------------------------------------------

/// Shapes taken from real journals: Claude puts the model on each assistant
/// turn, Codex writes a `turn_context` before one.
fn claude_assistant_with_model(model: &str) -> String {
    serde_json::json!({
        "type": "assistant",
        "timestamp": "2026-07-25T10:00:00.000Z",
        "message": {"model": model, "content": [{"type": "text", "text": "hi"}]}
    })
    .to_string()
}

fn codex_turn_context(model: &str) -> String {
    serde_json::json!({
        "type": "turn_context",
        "timestamp": "2026-07-25T10:00:00.000Z",
        "payload": {"model": model, "effort": "xhigh", "summary": "auto"}
    })
    .to_string()
}

#[test]
fn a_session_reports_the_model_its_own_journal_names() {
    let mut claude = TaskTracker::new(Provider::Claude);
    assert_eq!(claude.model(), None, "nothing is known before a turn");
    feed(
        &mut claude,
        &[&claude_assistant_with_model("claude-opus-5")],
    );
    assert_eq!(claude.model(), Some("claude-opus-5"));

    let mut codex = TaskTracker::new(Provider::Codex);
    feed(&mut codex, &[&codex_turn_context("gpt-5.6-sol")]);
    assert_eq!(codex.model(), Some("gpt-5.6-sol"));
}

/// The point of reading this per line rather than once: both agents let you
/// switch model without restarting, and both say so on the next turn.
#[test]
fn switching_model_mid_session_is_picked_up() {
    let mut claude = TaskTracker::new(Provider::Claude);
    feed(
        &mut claude,
        &[
            &claude_assistant_with_model("claude-opus-5"),
            &claude_user("now use something cheaper"),
            &claude_assistant_with_model("claude-haiku-4-5"),
        ],
    );
    assert_eq!(claude.model(), Some("claude-haiku-4-5"));

    let mut codex = TaskTracker::new(Provider::Codex);
    feed(
        &mut codex,
        &[
            &codex_turn_context("gpt-5.6-sol"),
            &codex_turn_context("gpt-5.6"),
        ],
    );
    assert_eq!(codex.model(), Some("gpt-5.6"));
}

/// Two ways the field lies, both seen in real logs.
#[test]
fn what_sits_in_the_model_field_but_is_not_one_is_ignored() {
    // Claude labels harness-generated turns `<synthetic>`.
    let mut claude = TaskTracker::new(Provider::Claude);
    feed(
        &mut claude,
        &[
            &claude_assistant_with_model("claude-opus-5"),
            &claude_assistant_with_model("<synthetic>"),
        ],
    );
    assert_eq!(claude.model(), Some("claude-opus-5"));

    // A subagent's transcript shares the file and may be running a different
    // model on this session's behalf. That is not what *this* session answers
    // with, so it must not overwrite it.
    let mut sidechain = TaskTracker::new(Provider::Claude);
    let sub = serde_json::json!({
        "type": "assistant",
        "isSidechain": true,
        "message": {"model": "claude-haiku-4-5", "content": []}
    })
    .to_string();
    feed(
        &mut sidechain,
        &[&claude_assistant_with_model("claude-opus-5"), &sub],
    );
    assert_eq!(sidechain.model(), Some("claude-opus-5"));
}

/// A model carried over from a conversation we no longer read would be a
/// confident wrong answer, so relocating clears it.
#[test]
fn losing_the_store_forgets_the_model() {
    let mut t = TaskTracker::new(Provider::Claude);
    feed(&mut t, &[&claude_assistant_with_model("claude-opus-5")]);
    assert_eq!(t.model(), Some("claude-opus-5"));
    t.reset();
    assert_eq!(t.model(), None);
}

/// The tests above drive `ingest_line`, which only exists for them. This one
/// goes the way production does — bytes on disk, through `refresh` — because
/// that is the path that has to work.
#[test]
fn the_model_is_read_by_the_same_pass_that_reads_the_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    fs::write(
        &path,
        format!(
            "{}\n{}\n",
            claude_user("go"),
            claude_assistant_with_model("claude-opus-5")
        ),
    )
    .unwrap();

    let ctx = TaskSource {
        provider: Provider::Claude,
        session_uuid: "x".into(),
        cwd: dir.path().to_path_buf(),
        started_at: Utc::now(),
        conversation: None,
        spawned_at: None,
        reported: None,
    };
    let mut t = TaskTracker::new(Provider::Claude);
    t.source = Some(Source::File(path.clone()));
    t.refresh(&ctx, &HashSet::new());
    assert_eq!(t.model(), Some("claude-opus-5"));

    // And the switch arrives with the bytes that announce it, not a pass later.
    let mut appended = fs::read_to_string(&path).unwrap();
    appended.push_str(&format!(
        "{}\n",
        claude_assistant_with_model("claude-sonnet-5")
    ));
    fs::write(&path, appended).unwrap();
    t.refresh(&ctx, &HashSet::new());
    assert_eq!(t.model(), Some("claude-sonnet-5"));
}

/// The codex half of the above, starting a step earlier: the rollout is *found*
/// rather than handed over.
///
/// Both halves were previously only covered by `ingest_line` against a
/// hand-written `turn_context`, which is the one arrangement that cannot fail —
/// it skips discovery, and every codex fixture on disk stopped at the
/// `session_meta` header, so no test ever read a model off a located file.
#[test]
fn a_located_codex_rollout_names_the_model_it_answers_with() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let dir = codex_day_dir(home.path());
    let root = files::codex_sessions_root(home.path());

    let spawned_at = Utc::now() - ChronoDuration::seconds(30);
    codex_rollout(
        &dir,
        "11111111-1111-4111-8111-111111111111",
        cwd.path(),
        spawned_at + ChronoDuration::seconds(1),
    );

    let ctx = codex_ctx(cwd.path(), Some(spawned_at));
    let found = files::locate_codex(&root, &ctx, &HashSet::new()).expect("the rollout it opened");

    let mut t = TaskTracker::new(Provider::Codex);
    t.source = Some(Source::File(found));
    t.refresh(&ctx, &HashSet::new());
    assert_eq!(t.model(), Some(FIXTURE_CODEX_MODEL));
}

// ---------------------------------------------------------------------------
// pi
// ---------------------------------------------------------------------------

/// Shapes taken from a real pi journal.
fn pi_model_change(model: &str) -> String {
    serde_json::json!({
        "type": "model_change",
        "id": "ee8bb56d",
        "timestamp": "2026-08-21T21:30:52.002Z",
        "provider": "openrouter",
        "modelId": model
    })
    .to_string()
}

fn pi_assistant(model: &str) -> String {
    serde_json::json!({
        "type": "message",
        "id": "9805e817",
        "timestamp": "2026-08-21T21:31:28.131Z",
        "message": {
            "role": "assistant",
            "provider": "openrouter",
            "model": model,
            "content": [{"type": "text", "text": "hi"}]
        }
    })
    .to_string()
}

fn pi_dir(home: &Path, cwd: &Path) -> PathBuf {
    let encoded = cwd
        .to_string_lossy()
        .trim_start_matches('/')
        .replace('/', "-");
    let dir = home
        .join(".pi")
        .join("agent")
        .join("sessions")
        .join(format!("--{encoded}--"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn pi_ctx(cwd: &Path, session_uuid: &str) -> TaskSource {
    TaskSource {
        provider: Provider::Pi,
        session_uuid: session_uuid.to_string(),
        cwd: cwd.to_path_buf(),
        started_at: Utc::now() - ChronoDuration::minutes(1),
        conversation: None,
        spawned_at: None,
        reported: None,
    }
}

/// The bug this fixes: a pi session showed as plain "Pi" because nothing read
/// its journal. It names the model twice and both are read — the opening
/// `model_change` so the pane is right before the first answer, then each
/// assistant turn.
#[test]
fn a_pi_session_reports_the_model_its_journal_names() {
    let mut pi = TaskTracker::new(Provider::Pi);
    assert_eq!(pi.model(), None, "nothing is known before it opens");
    feed(&mut pi, &[&pi_model_change("google/gemini-3.7-flash")]);
    assert_eq!(pi.model(), Some("google/gemini-3.7-flash"));
    assert_eq!(
        crate::models::model_label(pi.model().unwrap()),
        "Gemini 3.7 Flash",
        "which is what the sessions pane prints"
    );
}

#[test]
fn switching_model_mid_pi_session_is_picked_up() {
    let mut pi = TaskTracker::new(Provider::Pi);
    feed(
        &mut pi,
        &[
            &pi_model_change("z-ai/glm-5v-turbo"),
            &pi_assistant("z-ai/glm-5v-turbo"),
            // Ctrl+P cycles the model without restarting the session.
            &pi_model_change("google/gemini-3.7-flash"),
            &pi_assistant("google/gemini-3.7-flash"),
        ],
    );
    assert_eq!(pi.model(), Some("google/gemini-3.7-flash"));
}

/// A user message carries no model, and must not blank the one we have.
#[test]
fn a_pi_user_turn_does_not_clear_the_model() {
    let mut pi = TaskTracker::new(Provider::Pi);
    let user = serde_json::json!({
        "type": "message",
        "message": {"role": "user", "content": [{"type": "text", "text": "hi"}]}
    })
    .to_string();
    feed(
        &mut pi,
        &[&pi_model_change("google/gemini-3.7-flash"), &user],
    );
    assert_eq!(pi.model(), Some("google/gemini-3.7-flash"));
}

/// pi is spawned with `--session-id <ours>` and files the log as
/// `<stamp>_<id>.jsonl`, so the right one is found by name — even with a
/// newer log from another agent sitting beside it.
#[test]
fn a_pi_log_is_found_by_the_session_id_we_pinned() {
    let home = tempfile::tempdir().unwrap();
    let project = home.path().join("Code").join("app");
    fs::create_dir_all(&project).unwrap();
    let dir = pi_dir(home.path(), &project);

    let ours = "52cb8b04-c985-4506-9ff1-27e3dbcd15ae";
    let mine = dir.join(format!("2026-08-21T21-30-51-948Z_{ours}.jsonl"));
    fs::write(&mine, pi_model_change("google/gemini-3.7-flash") + "\n").unwrap();
    let theirs = dir.join("2026-08-21T22-00-00-000Z_86b7783e-6ec1-4f8f-a396-f8e285bb14c4.jsonl");
    fs::write(&theirs, "{}\n").unwrap();

    let root = home.path().join(".pi").join("agent").join("sessions");
    let found = files::locate_pi(&root, &pi_ctx(&project, ours), &HashSet::new());
    assert_eq!(found.as_deref(), Some(mine.as_path()));

    // And that path is what `pi --session <id>` reopens on restart.
    let mut tracker = TaskTracker::with_source(Provider::Pi, Source::File(mine.clone()));
    tracker.refresh(&pi_ctx(&project, ours), &HashSet::new());
    assert_eq!(tracker.provider_session_id().as_deref(), Some(ours));
    assert_eq!(tracker.model(), Some("google/gemini-3.7-flash"));
}

/// Started by hand rather than spawned by us: nothing names the log, so the
/// newest one in this project wins — and never one another session owns.
#[test]
fn an_unpinned_pi_session_takes_the_newest_unclaimed_log() {
    let home = tempfile::tempdir().unwrap();
    let project = home.path().join("Code").join("app");
    fs::create_dir_all(&project).unwrap();
    let dir = pi_dir(home.path(), &project);

    let now = Utc::now();
    let older = dir.join("2026-08-21T21-00-00-000Z_11111111-1111-4111-8111-111111111111.jsonl");
    let newer = dir.join("2026-08-21T22-00-00-000Z_22222222-2222-4222-8222-222222222222.jsonl");
    for (path, touched) in [(&older, now - ChronoDuration::seconds(30)), (&newer, now)] {
        fs::write(path, "{}\n").unwrap();
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(std::time::SystemTime::from(touched))
            .unwrap();
    }

    let root = home.path().join(".pi").join("agent").join("sessions");
    let ctx = pi_ctx(&project, "33333333-3333-4333-8333-333333333333");

    let claimed: HashSet<String> = [newer.to_string_lossy().into_owned()].into_iter().collect();
    assert_eq!(
        files::locate_pi(&root, &ctx, &claimed).as_deref(),
        Some(older.as_path()),
        "the newest log is another session's"
    );
}

/// A project outside this cwd has its own directory, and is never read.
#[test]
fn a_pi_log_from_another_project_is_not_picked_up() {
    let home = tempfile::tempdir().unwrap();
    let mine = home.path().join("Code").join("app");
    let other = home.path().join("Code").join("other");
    fs::create_dir_all(&mine).unwrap();
    fs::create_dir_all(&other).unwrap();
    let dir = pi_dir(home.path(), &other);
    fs::write(
        dir.join("2026-08-21T22-00-00-000Z_44444444-4444-4444-8444-444444444444.jsonl"),
        "{}\n",
    )
    .unwrap();
    // The cwd's own directory exists but is empty.
    pi_dir(home.path(), &mine);

    let root = home.path().join(".pi").join("agent").join("sessions");
    let ctx = pi_ctx(&mine, "55555555-5555-4555-8555-555555555555");
    assert_eq!(files::locate_pi(&root, &ctx, &HashSet::new()), None);
}

/// The stamp ahead of the id carries its own dashes, so only a tail actually
/// shaped like a uuid counts as one.
#[test]
fn a_pi_session_id_is_only_taken_from_a_well_formed_name() {
    assert_eq!(
        files::pi_session_id(Path::new(
            "/s/2026-08-21T21-30-51-948Z_52cb8b04-c985-4506-9ff1-27e3dbcd15ae.jsonl"
        ))
        .as_deref(),
        Some("52cb8b04-c985-4506-9ff1-27e3dbcd15ae")
    );
    assert_eq!(
        files::pi_session_id(Path::new("/s/2026-08-21T21-30-51-948Z.jsonl")),
        None
    );
    assert_eq!(files::pi_session_id(Path::new("/s/notes.jsonl")), None);
}
