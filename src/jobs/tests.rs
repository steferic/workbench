use super::*;

const TWO_JOBS: &str = r#"
schema_version = 1
[[job]]
id = "tiktok-questions"
title = "TikTok: creation questions"
every = "1d"
prompt = "Use the tiktok skill. Do not post."
instructions = ["ops/jobs/tiktok.yaml"]
[[job]]
id = "signup-email"
prompt_file = "ops/prompts/email.md"
"#;

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn job(id: &str) -> JobDef {
    JobDef {
        id: id.into(),
        title: id.into(),
        description: String::new(),
        agent: None,
        every: None,
        tags: vec![],
        prompt: PromptSource::Inline("do the thing".into()),
        instructions: vec![],
        skip_permissions: false,
    }
}

#[test]
fn a_manifest_with_two_jobs_parses_in_manifest_order() {
    let manifest = manifest::parse(TWO_JOBS).unwrap();
    let ids: Vec<&str> = manifest.jobs.iter().map(|j| j.id.as_str()).collect();
    assert_eq!(ids, ["tiktok-questions", "signup-email"]);
    assert_eq!(manifest.jobs[0].title, "TikTok: creation questions");
    assert_eq!(
        manifest.jobs[1].title, "signup-email",
        "title falls back to the id"
    );
    assert_eq!(manifest.jobs[0].every, Some(Duration::from_secs(86_400)));
    assert_eq!(
        manifest.jobs[1].prompt,
        PromptSource::File("ops/prompts/email.md".into())
    );
    assert_eq!(
        manifest.history_dir,
        PathBuf::from(manifest::DEFAULT_HISTORY_DIR)
    );
}

#[test]
fn bad_ids_duplicate_ids_and_two_prompt_forms_are_rejected_by_name() {
    let dup = manifest::parse(
        "schema_version = 1\n[[job]]\nid = \"a\"\nprompt = \"x\"\n[[job]]\nid = \"a\"\nprompt = \"y\"\n",
    )
    .unwrap_err();
    assert!(dup.contains("`a`") && dup.contains("twice"), "{dup}");
    let bad = manifest::parse("schema_version = 1\n[[job]]\nid = \"Bad_Id\"\nprompt = \"x\"\n")
        .unwrap_err();
    assert!(bad.contains("`Bad_Id`"), "{bad}");
    let both = manifest::parse(
        "schema_version = 1\n[[job]]\nid = \"a\"\nprompt = \"x\"\nprompt_file = \"y\"\n",
    )
    .unwrap_err();
    assert!(both.contains("`a`") && both.contains("not both"), "{both}");
    let none = manifest::parse("schema_version = 1\n[[job]]\nid = \"a\"\n").unwrap_err();
    assert!(none.contains("needs a prompt"), "{none}");
    let unknown = manifest::parse(
        "schema_version = 1\n[[job]]\nid = \"a\"\nprompt = \"x\"\nmode = \"publish\"\n",
    )
    .unwrap_err();
    assert!(
        unknown.contains("mode"),
        "an unknown field is named: {unknown}"
    );
}

#[test]
fn every_accepts_minutes_hours_days_weeks_and_rejects_the_rest() {
    use manifest::{describe_every, parse_every};
    assert_eq!(parse_every("30m"), Some(Duration::from_secs(1800)));
    assert_eq!(parse_every("12h"), Some(Duration::from_secs(12 * 3600)));
    assert_eq!(parse_every("1d"), Some(Duration::from_secs(86_400)));
    assert_eq!(parse_every("1w"), Some(Duration::from_secs(7 * 86_400)));
    assert_eq!(parse_every("2x"), None);
    assert_eq!(parse_every("0d"), None);
    assert_eq!(parse_every("d"), None);
    assert_eq!(describe_every(Duration::from_secs(12 * 3600)), "12h");
    assert_eq!(describe_every(Duration::from_secs(14 * 86_400)), "2w");
    let err = manifest::parse(
        "schema_version = 1\n[[job]]\nid = \"a\"\nprompt = \"x\"\nevery = \"2x\"\n",
    )
    .unwrap_err();
    assert!(err.contains("2x"), "{err}");
}

#[test]
fn history_folds_a_runs_lines_into_the_latest_and_skips_what_will_not_parse() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let history = Path::new(manifest::DEFAULT_HISTORY_DIR);
    let job = job("a");
    let started = start_record(
        root,
        &job,
        "20260917T100000Z-0001",
        "claude",
        Some("abcd1234".into()),
    );
    history::append(root, history, &started).unwrap();
    let mut other = start_record(root, &job, "20260917T090000Z-0002", "codex", None);
    other.started_utc = started.started_utc - chrono::TimeDelta::hours(1);
    history::append(root, history, &other).unwrap();
    let mut closed = close_record(&started, RunStatus::Completed);
    closed.summary = Some("27 questions, 5 fits".into());
    history::append(root, history, &closed).unwrap();
    // A bad merge leaves a half line behind.
    std::fs::OpenOptions::new()
        .append(true)
        .open(history::path(root, history, "a"))
        .unwrap()
        .write_all(b"{\"run_id\": \"broken\"\n")
        .unwrap();

    let runs = history::read(root, history, "a");
    assert_eq!(runs.len(), 2);
    assert_eq!(
        runs[0].run_id, "20260917T090000Z-0002",
        "oldest start first"
    );
    assert_eq!(runs[1].status, RunStatus::Completed);
    assert_eq!(runs[1].summary.as_deref(), Some("27 questions, 5 fits"));
    assert_eq!(
        runs[1].session.as_deref(),
        Some("abcd1234"),
        "the close keeps what launch recorded"
    );
    assert!(runs[1].ended_utc.is_some());
    assert_eq!(
        history::find(root, history, "a", "20260917T100000Z-0001")
            .unwrap()
            .status,
        RunStatus::Completed
    );
}

#[test]
fn two_writers_appending_leave_two_independent_lines() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let history = Path::new("ops/history");
    let job = job("a");
    history::append(
        root,
        history,
        &start_record(root, &job, "r1", "claude", None),
    )
    .unwrap();
    history::append(
        root,
        history,
        &start_record(root, &job, "r2", "claude", None),
    )
    .unwrap();
    let text = std::fs::read_to_string(history::path(root, history, "a")).unwrap();
    assert_eq!(text.lines().count(), 2);
    assert!(text
        .lines()
        .all(|line| serde_json::from_str::<RunRecord>(line).is_ok()));
    assert!(
        text.ends_with('\n'),
        "each line is whole, so a concurrent append starts fresh"
    );
}

#[test]
fn compose_reads_the_prompt_file_from_the_root_and_ends_with_the_report_command() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "ops/prompts/email.md",
        "Review signups.\n\nDo not send.\n",
    );
    let mut job = job("signup-email");
    job.prompt = PromptSource::File("ops/prompts/email.md".into());
    let text = prompt::compose(root, &job, "20260917T100000Z-0001").unwrap();
    assert!(
        text.starts_with("Review signups.\n\nDo not send.\n\n---\n"),
        "{text}"
    );
    assert!(text.contains("run `20260917T100000Z-0001` of `signup-email`"));
    assert!(text.contains("workbench jobs report --status"));
    assert!(text.contains(".workbench/jobs/lessons/signup-email.md"));
    assert!(text.trim_end().ends_with("that is the user's call."));
    job.prompt = PromptSource::File("missing.md".into());
    let err = prompt::compose(root, &job, "r").unwrap_err().to_string();
    assert!(err.contains("missing.md"), "{err}");
}

#[test]
fn load_reads_manifest_runs_and_lessons_and_marks_due_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, MANIFEST, TWO_JOBS);
    write(root, "ops/jobs/tiktok.yaml", "id: tiktok\n");
    assert_eq!(find_root(&root.join("ops/jobs")), Some(root.to_path_buf()));
    assert_eq!(find_root(dir.path().parent().unwrap()), None);

    let project = load(root).unwrap();
    let tiktok = project.job("tiktok-questions").unwrap().clone();
    let now = chrono::Utc::now();
    assert!(project.due(&tiktok, now), "never run and on a cadence: due");
    assert!(
        !project.due(project.job("signup-email").unwrap(), now),
        "on demand: never due"
    );

    let hashes = instruction_hashes(root, &tiktok);
    assert_eq!(
        hashes.get("ops/jobs/tiktok.yaml").map(String::len),
        Some(16)
    );
    let email = project.job("signup-email").unwrap();
    assert_eq!(
        instruction_hashes(root, email)
            .get("ops/prompts/email.md")
            .map(String::as_str),
        Some("missing"),
        "the prompt file counts as an instruction and a missing one says so"
    );

    let record = start_record(root, &tiktok, "r1", "claude", None);
    history::append(root, &project.history_dir(), &record).unwrap();
    history::append_lesson(
        root,
        "tiktok-questions",
        "process queries  beat\noutput queries",
    )
    .unwrap();
    let project = load(root).unwrap();
    assert!(!project.due(&tiktok, now), "just started: not due");
    assert!(project.due(&tiktok, now + chrono::TimeDelta::days(2)));
    assert_eq!(project.last_run("tiktok-questions").unwrap().run_id, "r1");
    let lessons = &project.lessons["tiktok-questions"];
    assert!(lessons.starts_with("# Lessons: tiktok-questions"));
    assert!(
        lessons
            .trim_end()
            .ends_with(": process queries beat output queries"),
        "{lessons}"
    );
    assert_ne!(project.stamp, Stamp::default());
    assert_eq!(project.stamp, project.stamp.refresh(root));
    assert_eq!(
        project
            .stamp
            .watched
            .iter()
            .map(|(p, _)| p.display().to_string())
            .collect::<Vec<_>>(),
        ["ops/jobs/tiktok.yaml", "ops/prompts/email.md"],
        "instruction and prompt files are watched too"
    );

    write(root, MANIFEST, "schema_version = 1\n[[job]]\nid = \"a\"\n");
    let broken = load(root).unwrap();
    assert!(matches!(&broken.manifest, Some(Err(e)) if e.contains("needs a prompt")));
    assert!(broken.jobs().is_empty());
    assert!(
        load(dir.path().parent().unwrap()).is_none(),
        "no manifest, no project"
    );
}

#[test]
fn init_scaffolds_once_and_adds_the_union_merge_line_once() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join(".gitattributes"), "*.png binary").unwrap();
    let written = init(root).unwrap();
    for rel in [
        MANIFEST,
        ".workbench/jobs/README.md",
        ".workbench/jobs/history/.gitkeep",
        ".workbench/jobs/lessons/.gitkeep",
        ".gitattributes",
    ] {
        assert!(written.contains(&root.join(rel)), "{rel} written");
    }
    let attributes = std::fs::read_to_string(root.join(".gitattributes")).unwrap();
    assert_eq!(
        attributes,
        "*.png binary\n.workbench/jobs/history/*.jsonl merge=union\n"
    );
    let template = load(root).unwrap();
    assert_eq!(template.jobs().len(), 1, "the template's example parses");
    let err = init(root).unwrap_err().to_string();
    assert!(err.contains("already exists"), "{err}");
    assert_eq!(
        std::fs::read_to_string(root.join(".gitattributes")).unwrap(),
        attributes,
        "a refused init touches nothing"
    );
}

#[test]
fn stats_count_reported_outcomes_and_average_only_reported_durations() {
    let job = job("a");
    let root = Path::new("/");
    let now = chrono::Utc::now();
    let mut runs = Vec::new();
    for (i, status, minutes) in [
        (1, RunStatus::Completed, Some(4)),
        (2, RunStatus::Completed, Some(6)),
        (3, RunStatus::Partial, Some(2)),
        (4, RunStatus::Aborted, Some(1)),
        (5, RunStatus::Unreported, Some(1)),
        (6, RunStatus::Running, None),
    ] {
        let mut run = start_record(root, &job, &format!("r{i}"), "claude", None);
        run.started_utc = now - chrono::TimeDelta::days(i);
        run.by.user = if i % 2 == 0 {
            "ann".into()
        } else {
            "bo".into()
        };
        if status != RunStatus::Running {
            run = close_record(&run, status);
            run.ended_utc = minutes.map(|m| run.started_utc + chrono::TimeDelta::minutes(m));
        }
        runs.push(run);
    }
    let s = stats(&runs, now);
    assert_eq!(
        (
            s.total,
            s.completed,
            s.partial,
            s.aborted,
            s.unreported,
            s.running
        ),
        (6, 2, 1, 1, 1, 1)
    );
    assert_eq!(
        s.success_pct,
        Some(66),
        "2 of the 3 reported runs completed"
    );
    assert_eq!(
        s.mean_seconds,
        Some(4 * 60),
        "aborted and unreported runs say nothing about how long the job takes"
    );
    assert_eq!(s.last_7_days, 6);
    assert_eq!(s.people, vec!["bo".to_string(), "ann".to_string()]);
    assert_eq!(stats(&[], now), Stats::default());
}

#[test]
fn run_ids_sort_by_time_and_statuses_round_trip() {
    let a = new_run_id();
    assert_eq!(a.len(), "20260917T100000Z-0000".len(), "{a}");
    for status in [
        RunStatus::Running,
        RunStatus::Completed,
        RunStatus::Partial,
        RunStatus::Blocked,
        RunStatus::Failed,
        RunStatus::Unreported,
        RunStatus::Aborted,
    ] {
        assert_eq!(RunStatus::parse(status.label()), Some(status));
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, format!("\"{}\"", status.label()));
    }
    assert_eq!(RunStatus::parse("done"), Some(RunStatus::Completed));
    assert_eq!(RunStatus::parse("great"), None);
    let actor = actor(Path::new("/"));
    assert!(!actor.user.is_empty() && !actor.host.is_empty());
}

use std::io::Write;
use std::time::Duration;
