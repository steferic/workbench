use super::*;
use crate::app::{process_action, FocusPanel, InputMode};
use crate::config::user_config::AgentConfig;
use crate::jobs::history;
use crate::models::Workspace;
use crate::tui::event::EventHandler;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

const MANIFEST: &str = r#"
schema_version = 1
[[job]]
id = "tiktok-questions"
title = "TikTok: creation questions"
description = "Read-only research."
every = "1d"
prompt = "Use the tiktok skill. Do not post."
instructions = ["ops/tiktok.yaml"]
[[job]]
id = "signup-email"
title = "Signup email"
prompt = "Review signups."
"#;

/// A project on disk, a workbench with it open on the Jobs tab, and an
/// "agent" that records its environment and then waits to be stopped.
struct Fixture {
    dir: tempfile::TempDir,
    state: AppState,
    workspace: Uuid,
    pty: PtyManager,
    tx: mpsc::UnboundedSender<Action>,
    _rx: mpsc::UnboundedReceiver<Action>,
    pty_tx: mpsc::Sender<Action>,
    _pty_rx: mpsc::Receiver<Action>,
}

impl Fixture {
    fn new(manifest: Option<&str>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        if let Some(manifest) = manifest {
            std::fs::create_dir_all(root.join(".workbench")).unwrap();
            std::fs::write(root.join(jobs::MANIFEST), manifest).unwrap();
        }
        std::fs::create_dir_all(root.join("ops")).unwrap();
        std::fs::write(root.join("ops/tiktok.yaml"), "id: tiktok\n").unwrap();
        let script = root.join("agent");
        std::fs::write(
            &script,
            "#!/bin/sh\nprintf '%s %s' \"$WORKBENCH_JOB\" \"$WORKBENCH_JOB_RUN\" > \"$WORKBENCH_TEST_ENV_OUT\"\nexec sleep 30\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();

        let mut state = AppState::default();
        state.ui.banner_visible = false;
        state.ui.focus = FocusPanel::SessionList;
        state.system.user_config.agents = vec![AgentConfig {
            command: script.to_string_lossy().into(),
            display_name: "Fixture".into(),
            badge: "F".into(),
            hotkey: "1".into(),
            enabled: true,
        }];
        let workspace = Workspace::new("zeta".into(), root.to_path_buf());
        let workspace_id = workspace.id;
        state.data.workspaces = vec![workspace];
        let (tx, rx) = mpsc::unbounded_channel();
        let (pty_tx, pty_rx) = mpsc::channel(64);
        let mut fixture = Self {
            dir,
            state,
            workspace: workspace_id,
            pty: PtyManager::new(),
            tx,
            _rx: rx,
            pty_tx,
            _pty_rx: pty_rx,
        };
        fixture.rescan();
        fixture.act(Action::OpenJobs);
        fixture
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn rescan(&mut self) {
        let results = scan(scan_inputs(&self.state));
        apply_scan(&mut self.state, results);
    }

    fn press(&mut self, code: KeyCode) {
        let action = EventHandler::test_key(KeyEvent::new(code, KeyModifiers::NONE), &self.state);
        self.act(action);
    }

    fn act(&mut self, action: Action) {
        process_action(&mut self.state, action, &self.pty, &self.tx, &self.pty_tx).unwrap();
    }

    fn history_dir(&self) -> PathBuf {
        self.state.system.project_jobs[&self.workspace].history_dir()
    }

    fn ledger(&self, job_id: &str) -> Vec<RunRecord> {
        history::read(self.root(), &self.history_dir(), job_id)
    }

    fn sessions(&self) -> Vec<crate::models::Session> {
        self.state
            .data
            .sessions
            .get(&self.workspace)
            .cloned()
            .unwrap_or_default()
    }

    fn draw(&mut self, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| crate::tui::ui::draw(f, &mut self.state))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }
}

fn wait_for(condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    while !condition() {
        assert!(std::time::Instant::now() < deadline, "timed out waiting");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn enter_starts_a_session_aliased_after_the_job_with_the_prompt_queued_and_a_running_line() {
    let mut f = Fixture::new(Some(MANIFEST));
    let env_out = f.root().join("env.txt");
    std::env::set_var("WORKBENCH_TEST_ENV_OUT", &env_out);
    assert_eq!(
        rows(&f.state)
            .iter()
            .map(|r| r.key.id.as_str())
            .collect::<Vec<_>>(),
        ["tiktok-questions", "signup-email"]
    );
    assert!(rows(&f.state)[0].due, "on a cadence and never run");

    f.press(KeyCode::Enter);

    let sessions = f.sessions();
    assert_eq!(sessions.len(), 1);
    let session = &sessions[0];
    assert_eq!(session.alias.as_deref(), Some("tiktok-questions"));
    let link = session
        .job
        .clone()
        .expect("the session is linked to its run");
    assert_eq!(link.job_id, "tiktok-questions");
    let queued = session
        .todo_queue
        .next_pending()
        .expect("the prompt is queued, not typed");
    assert!(queued
        .text
        .starts_with("Use the tiktok skill. Do not post.\n\n---\n"));
    assert!(queued
        .text
        .contains(&format!("run `{}` of `tiktok-questions`", link.run_id)));
    assert!(queued.text.trim_end().ends_with("that is the user's call."));

    let runs = f.ledger("tiktok-questions");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].run_id, link.run_id);
    assert_eq!(runs[0].status, RunStatus::Running);
    assert_eq!(
        runs[0].session.as_deref(),
        Some(session.short_id().as_str())
    );
    assert_eq!(
        runs[0]
            .instructions_sha256_16
            .get("ops/tiktok.yaml")
            .map(String::len),
        Some(16)
    );
    assert_eq!(
        f.state.ui.input_mode,
        InputMode::Normal,
        "the window closes so the output pane, which it covered, shows the agent"
    );
    assert_eq!(f.state.ui.focus, FocusPanel::SessionList);
    assert_eq!(
        f.state.active_session_id(),
        Some(session.id),
        "and the new agent is what the output pane shows"
    );
    let row = &rows(&f.state)[0];
    assert_eq!(row.open_session, Some(session.id));
    assert!(!row.due, "a run just started is not due");

    wait_for(|| env_out.exists());
    assert_eq!(
        std::fs::read_to_string(&env_out).unwrap(),
        format!("tiktok-questions {}", link.run_id),
        "the agent's environment names the run"
    );
    std::env::remove_var("WORKBENCH_TEST_ENV_OUT");
}

#[test]
fn enter_on_a_running_job_goes_to_its_session_and_capital_r_starts_another() {
    let mut f = Fixture::new(Some(MANIFEST));
    f.press(KeyCode::Enter);
    let first = f.sessions()[0].id;
    f.state.set_active_session_id(None);

    f.act(Action::OpenJobs);
    f.press(KeyCode::Enter);
    assert_eq!(f.sessions().len(), 1, "no second run");
    assert_eq!(f.state.active_session_id(), Some(first));
    assert!(f
        .state
        .ui
        .jobs
        .message
        .as_deref()
        .unwrap()
        .contains("already running"));
    assert_eq!(
        f.state.ui.input_mode,
        InputMode::JobsWindow,
        "still open: nothing started"
    );

    f.press(KeyCode::Char('R'));
    assert_eq!(f.sessions().len(), 2);
    assert_eq!(f.ledger("tiktok-questions").len(), 2);
}

#[test]
fn a_retired_prompt_closes_the_run_as_unreported_unless_the_agent_already_reported() {
    let mut f = Fixture::new(Some(MANIFEST));
    f.press(KeyCode::Enter);
    let session_id = f.sessions()[0].id;
    let run_id = f.sessions()[0].job.clone().unwrap().run_id;
    // The prompt went out a while ago and the agent's hooks say it is idle.
    {
        let session = f.state.get_session_mut(session_id).unwrap();
        let id = session.todo_queue.items[0].id;
        session.todo_queue.mark_running(id);
        session.todo_queue.items[0].sent_at =
            Some(chrono::Utc::now() - chrono::TimeDelta::seconds(10));
    }
    f.state.system.agent_status.insert(
        session_id,
        crate::agent_status::AgentStatus {
            activity: crate::agent_status::Activity::Idle,
            reason: String::new(),
            at: chrono::Utc::now(),
            event: "Stop".into(),
            transcript: None,
            model: None,
        },
    );
    crate::app::todo_dispatch::tick(&mut f.state, &f.tx);
    let runs = f.ledger("tiktok-questions");
    assert_eq!(runs[0].status, RunStatus::Unreported);
    assert!(runs[0].ended_utc.is_some());
    assert_eq!(runs[0].run_id, run_id);
    assert_eq!(
        f.state.system.project_jobs[&f.workspace].runs["tiktok-questions"][0].status,
        RunStatus::Unreported,
        "memory follows the ledger"
    );

    // Second run: this time the agent reports (as the CLI would, straight to
    // the file) before its turn is seen to end. The report must win even
    // though memory still says running.
    f.act(Action::OpenJobs);
    f.press(KeyCode::Char('R'));
    let session_id = f.sessions()[1].id;
    let link = f.sessions()[1].job.clone().unwrap();
    let latest = history::find(f.root(), &f.history_dir(), &link.job_id, &link.run_id).unwrap();
    let mut reported = jobs::close_record(&latest, RunStatus::Completed);
    reported.summary = Some("done".into());
    history::append(f.root(), &f.history_dir(), &reported).unwrap();
    close_run(&mut f.state, session_id, RunStatus::Unreported);
    let runs = f.ledger("tiktok-questions");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[1].status, RunStatus::Completed);
    assert_eq!(runs[1].summary.as_deref(), Some("done"));
}

#[test]
fn killing_or_deleting_a_job_session_closes_its_run_as_aborted() {
    let mut f = Fixture::new(Some(MANIFEST));
    f.press(KeyCode::Enter);
    let first = f.sessions()[0].id;
    f.act(Action::KillSession(first));
    assert_eq!(f.ledger("tiktok-questions")[0].status, RunStatus::Aborted);

    f.act(Action::OpenJobs);
    f.press(KeyCode::Char('R'));
    let second = f.sessions()[1].id;
    f.act(Action::InitiateDeleteSession(second, "x".into()));
    f.act(Action::ConfirmDeleteSession);
    let runs = f.ledger("tiktok-questions");
    assert_eq!(runs[1].status, RunStatus::Aborted);
    assert!(f.sessions().iter().all(|s| s.id != second));
    assert!(rows(&f.state)[0].open_session.is_none());
}

#[test]
fn a_broken_manifest_is_one_error_row_and_enter_only_reports_it() {
    let mut f = Fixture::new(Some("schema_version = 1\n[[job]]\nid = \"a\"\n"));
    let rows = rows(&f.state);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].key.id, MANIFEST_ERROR_ID);
    assert!(rows[0].error.as_deref().unwrap().contains("needs a prompt"));
    f.press(KeyCode::Enter);
    assert!(f.sessions().is_empty());
    assert!(f
        .state
        .ui
        .jobs
        .message
        .as_deref()
        .unwrap()
        .contains("needs a prompt"));
    let text = f.draw(100, 30);
    assert!(text.contains("needs a prompt"));
    assert_eq!(f.state.ui.input_mode, InputMode::JobsWindow);
}

#[test]
fn a_rescan_skips_a_project_whose_files_have_not_changed() {
    let mut f = Fixture::new(Some(MANIFEST));
    let results = scan(scan_inputs(&f.state));
    assert!(matches!(results[0].1, Scanned::Unchanged));

    let job = f.state.system.project_jobs[&f.workspace]
        .job("signup-email")
        .unwrap()
        .clone();
    let record = jobs::start_record(f.root(), &job, "r1", "claude", None);
    history::append(f.root(), &f.history_dir(), &record).unwrap();
    let results = scan(scan_inputs(&f.state));
    assert!(matches!(&results[0].1, Scanned::Loaded(p) if p.last_run("signup-email").is_some()));
    apply_scan(&mut f.state, results);
    assert!(matches!(
        scan(scan_inputs(&f.state))[0].1,
        Scanned::Unchanged
    ));

    std::fs::remove_file(f.root().join(jobs::MANIFEST)).unwrap();
    let results = scan(scan_inputs(&f.state));
    assert!(matches!(results[0].1, Scanned::Absent));
    apply_scan(&mut f.state, results);
    assert!(rows(&f.state).is_empty());
    assert!(
        f.draw(100, 30).contains("jobs.toml"),
        "the empty state says what to create"
    );
}

#[test]
fn the_window_and_its_click_targets_fit_small_terminals_and_own_the_mouse() {
    for (width, height) in [(160, 48), (120, 40), (80, 24), (60, 18), (40, 12)] {
        let mut f = Fixture::new(Some(MANIFEST));
        let text = f.draw(width, height);
        assert_eq!(f.state.ui.input_mode, InputMode::JobsWindow);
        for (rect, _) in &f.state.ui.click_hits {
            assert!(rect.right() <= width && rect.bottom() <= height, "{rect:?}");
        }
        assert!(f.state.ui.link_hits.is_empty(), "the window owns the mouse");
        if width >= 80 {
            assert!(text.contains("Jobs · zeta · 2"), "{text}");
            assert!(text.contains("TikTok: creation questions"), "{text}");
            assert!(text.contains("1 Overview"), "{text}");
            assert!(text.contains("never run"), "{text}");
        }
        // A click outside every target does nothing to the app underneath.
        let focus = f.state.ui.focus;
        f.act(Action::MouseClick(0, 0));
        assert_eq!(f.state.ui.focus, focus);
        assert_eq!(f.state.ui.input_mode, InputMode::JobsWindow);
    }
}

#[test]
fn clicks_and_keys_walk_the_window_and_f4_and_escape_open_and_close_it() {
    let mut f = Fixture::new(Some(MANIFEST));
    f.press(KeyCode::Esc);
    assert_eq!(f.state.ui.input_mode, InputMode::Normal);
    f.state.ui.focus = FocusPanel::OutputPane;
    f.press(KeyCode::F(4));
    assert_eq!(
        f.state.ui.input_mode,
        InputMode::JobsWindow,
        "the global hotkey opens it from anywhere"
    );
    f.draw(120, 40);
    let second = f
        .state
        .ui
        .click_hits
        .iter()
        .find(|(_, a)| matches!(a, Action::SelectJob(key) if key.id == "signup-email"))
        .unwrap()
        .0;
    f.act(Action::MouseClick(second.x, second.y));
    assert_eq!(
        f.state.ui.jobs.selected.as_ref().unwrap().id,
        "signup-email"
    );
    f.press(KeyCode::Char('k'));
    assert_eq!(
        f.state.ui.jobs.selected.as_ref().unwrap().id,
        "tiktok-questions"
    );
    f.press(KeyCode::Char('j'));
    f.press(KeyCode::Char('j'));
    assert_eq!(
        f.state.ui.jobs.selected.as_ref().unwrap().id,
        "signup-email",
        "the cursor stops at the end"
    );
    f.draw(120, 40);
    let runs_tab = f
        .state
        .ui
        .click_hits
        .iter()
        .find(|(_, a)| matches!(a, Action::JobsTab(DetailTab::Runs)))
        .unwrap()
        .0;
    f.act(Action::MouseClick(runs_tab.x, runs_tab.y));
    assert_eq!(f.state.ui.jobs.tab, DetailTab::Runs);
    assert_eq!(f.state.ui.jobs.focus, WindowFocus::Detail);
    f.press(KeyCode::Tab);
    assert_eq!(f.state.ui.jobs.focus, WindowFocus::List);
    f.press(KeyCode::Char('4'));
    assert_eq!(f.state.ui.jobs.tab, DetailTab::Prompt);
    let text = f.draw(120, 40);
    assert!(
        text.contains("Review signups."),
        "the prompt tab shows the body: {text}"
    );
    assert!(text.contains("workbench jobs report"), "and the footer");
    f.press(KeyCode::Char('q'));
    assert_eq!(f.state.ui.input_mode, InputMode::Normal);
}

#[test]
fn improve_starts_a_helper_without_a_ledger_line_and_new_scaffolds_a_bare_project_first() {
    let mut f = Fixture::new(Some(MANIFEST));
    f.press(KeyCode::Char('i'));
    assert!(f.sessions().is_empty(), "nothing to learn from yet");
    assert!(f
        .state
        .ui
        .jobs
        .message
        .as_deref()
        .unwrap()
        .contains("no runs"));

    f.press(KeyCode::Enter);
    f.act(Action::OpenJobs);
    f.press(KeyCode::Char('i'));
    let sessions = f.sessions();
    assert_eq!(sessions.len(), 2);
    assert_eq!(
        sessions[1].alias.as_deref(),
        Some("improve:tiktok-questions")
    );
    assert!(sessions[1].job.is_none());
    let prompt = &sessions[1].todo_queue.items[0].text;
    assert!(prompt.contains("Do not run it"));
    assert!(prompt.contains("ops/tiktok.yaml"));
    assert_eq!(
        f.ledger("tiktok-questions").len(),
        1,
        "improving is not a run"
    );

    let mut bare = Fixture::new(None);
    assert!(bare.draw(100, 30).contains("n creates"));
    bare.press(KeyCode::Char('n'));
    assert!(bare.root().join(jobs::MANIFEST).is_file());
    assert!(bare.root().join(".workbench/jobs/README.md").is_file());
    let sessions = bare.sessions();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].alias.as_deref(), Some("new-job"));
    assert!(sessions[0].todo_queue.items[0]
        .text
        .contains(jobs::MANIFEST));
}

/// What went wrong live: the files were read while the Global workspace was
/// selected, so the cursor was never placed; switching to the project and
/// pressing Enter then did nothing, though the first row looked selected.
#[test]
fn enter_runs_the_highlighted_first_row_even_before_the_cursor_was_placed() {
    let mut f = Fixture::new(Some(MANIFEST));
    f.state.ui.jobs.selected = None;
    f.press(KeyCode::Enter);
    let sessions = f.sessions();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].alias.as_deref(), Some("tiktok-questions"));
    assert_eq!(
        f.state.ui.jobs.selected.as_ref().map(|k| k.id.as_str()),
        Some("tiktok-questions")
    );
}

#[test]
fn the_overview_counts_runs_and_flags_instructions_that_changed_since_the_last_one() {
    let mut f = Fixture::new(Some(MANIFEST));
    let job = f.state.system.project_jobs[&f.workspace]
        .job("tiktok-questions")
        .unwrap()
        .clone();
    let mut done = jobs::start_record(f.root(), &job, "r1", "claude", None);
    done.started_utc = chrono::Utc::now() - chrono::TimeDelta::minutes(30);
    let mut done = jobs::close_record(&done, RunStatus::Completed);
    done.ended_utc = Some(done.started_utc + chrono::TimeDelta::minutes(4));
    done.summary = Some("Twenty-seven questions. Five fits.".into());
    done.artifacts = Some("ops/runs/r1".into());
    history::append(f.root(), &f.history_dir(), &done).unwrap();
    let failed = jobs::close_record(
        &jobs::start_record(f.root(), &job, "r2", "claude", None),
        RunStatus::Failed,
    );
    history::append(f.root(), &f.history_dir(), &failed).unwrap();
    f.rescan();
    let row = selected(&f.state).unwrap();
    let ov = overview(&f.state, &row);
    assert_eq!(ov.stats.total, 2);
    assert_eq!(ov.stats.completed, 1);
    assert_eq!(ov.stats.failed, 1);
    assert_eq!(ov.stats.success_pct, Some(50));
    assert_eq!(ov.stats.last_7_days, 2);
    assert_eq!(ov.changed_since_last_run, 0);
    let text = f.draw(140, 40);
    assert!(text.contains("2 runs · 1 completed · 1 failed"), "{text}");
    assert!(text.contains("success 50%"), "{text}");
    assert!(text.contains("same as last run"), "{text}");

    std::fs::write(
        f.root().join("ops/tiktok.yaml"),
        "id: tiktok\nqueries: [x]\n",
    )
    .unwrap();
    f.rescan();
    let ov = overview(&f.state, &row);
    assert_eq!(ov.changed_since_last_run, 1);
    let text = f.draw(140, 40);
    assert!(text.contains("1 changed since the last run"), "{text}");

    f.press(KeyCode::Char('2'));
    f.press(KeyCode::Char('j'));
    assert_eq!(
        f.state.ui.jobs.run_cursor, 1,
        "j walks the runs when the detail has the cursor"
    );
    let text = f.draw(140, 40);
    assert!(text.contains("Run r1"), "{text}");
    assert!(text.contains("Twenty-seven questions"), "{text}");
    f.press(KeyCode::Char('j'));
    assert_eq!(f.state.ui.jobs.run_cursor, 1, "and stops at the oldest");
}
