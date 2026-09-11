//! Real PTY/process tests. The fixture is this test binary, so no Python,
//! provider CLI, or user configuration is needed.
use super::{PtyHandle, PtyManager, Resume, SessionSpawnConfig};
use crate::app::{Action, AppState};
use crate::models::{AgentType, Session, Workspace};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use uuid::Uuid;

const FIXTURE: &str = "pty::cleanup_tests::fixture";

#[test]
fn fixture() {
    let Ok(mode) = std::env::var("WB_CLEANUP_TEST") else {
        return;
    };
    unsafe {
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
        libc::signal(
            libc::SIGINT,
            if mode == "leaf" {
                libc::SIG_IGN
            } else {
                libc::SIG_DFL
            },
        );
    }
    if mode == "leaf" {
        std::fs::write("leaf.pid", std::process::id().to_string()).unwrap();
        std::thread::sleep(Duration::from_secs(20));
        return;
    }
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", FIXTURE, "--nocapture"])
        .env("WB_CLEANUP_TEST", "leaf");
    if mode != "hold_slave" {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
    }
    if mode == "detached" || mode == "orphan" {
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    if mode == "clear_env" {
        command.env_clear().env("WB_CLEANUP_TEST", "leaf");
    }
    let mut child = command.spawn().unwrap();
    wait_for(|| Path::new("leaf.pid").exists());
    std::fs::write("ready", b"ready").unwrap();
    if mode == "orphan" || mode == "hold_slave" {
        return;
    }
    let _ = child.wait();
}

fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for process fixture"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct Case {
    _dir: tempfile::TempDir,
    handle: Option<PtyHandle>,
    rx: mpsc::Receiver<Action>,
    session: Uuid,
    leaf: u32,
    leaf_start: Option<super::proc_identity::ProcStart>,
}

impl Case {
    fn new(mode: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("agent");
        let executable = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .replace('\'', "'\\''");
        std::fs::write(&script, format!("#!/bin/sh\nexport WB_CLEANUP_TEST={mode}\nexec '{executable}' --exact {FIXTURE} --nocapture\n")).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let (tx, rx) = mpsc::channel(32);
        let session = Uuid::new_v4();
        let handle = PtyManager::new()
            .spawn_session(SessionSpawnConfig {
                session_id: session,
                workspace_id: Uuid::new_v4(),
                agent_type: AgentType::Custom {
                    command: script.to_string_lossy().into(),
                    display_name: "fixture".into(),
                    badge: "T".into(),
                },
                working_dir: dir.path(),
                rows: 24,
                cols: 80,
                pty_tx: tx,
                resume: Resume::No,
                dangerously_skip_permissions: false,
                use_alternate_screen: false,
            })
            .unwrap();
        wait_for(|| dir.path().join("ready").exists());
        let leaf = std::fs::read_to_string(dir.path().join("leaf.pid"))
            .unwrap()
            .parse()
            .unwrap();
        Self {
            _dir: dir,
            handle: Some(handle),
            rx,
            session,
            leaf,
            leaf_start: super::proc_identity::start_time(leaf),
        }
    }

    fn assert_leaf_gone(&self) {
        wait_for(|| !super::process_tree::running(self.leaf));
    }
}

impl Drop for Case {
    fn drop(&mut self) {
        // Even a failing regression test must not leave its fixture behind.
        if self.leaf_start.is_some_and(|start| {
            super::proc_identity::owner(self.leaf, start) == super::proc_identity::PidOwner::Ours
        }) {
            unsafe {
                libc::kill(self.leaf as i32, libc::SIGKILL);
            }
        }
    }
}

#[test]
fn killing_an_agent_cleans_children_in_another_session() {
    let mut case = Case::new("detached");
    case.handle.as_mut().unwrap().kill().unwrap();
    case.assert_leaf_gone();
}

#[test]
fn escalation_keeps_children_after_the_leader_is_reaped() {
    let mut case = Case::new("grace");
    case.handle
        .as_mut()
        .unwrap()
        .interrupt_then_kill(Duration::from_millis(80))
        .unwrap();
    case.assert_leaf_gone();
}

#[test]
fn parent_links_find_children_that_clear_their_environment() {
    let mut case = Case::new("clear_env");
    case.handle.as_mut().unwrap().kill().unwrap();
    case.assert_leaf_gone();
}

#[test]
fn natural_exit_cleans_reparented_children_and_does_not_wait_for_pty_eof() {
    for mode in ["orphan", "hold_slave"] {
        let mut case = Case::new(mode);
        wait_for(|| {
            while let Ok(action) = case.rx.try_recv() {
                if matches!(action, Action::SessionExited(id, _, 0) if id == case.session) {
                    return true;
                }
            }
            false
        });
        case.assert_leaf_gone();
    }
}

#[test]
fn shutdown_waits_for_terminal_cleanup_including_previous_deletions() {
    let mut case = Case::new("grace");
    let mut state = AppState::default();
    let ws = Workspace::new("fixture".into(), case._dir.path().to_owned());
    let mut session = Session::new(ws.id, AgentType::Terminal("fixture".into()), false);
    session.id = case.session;
    state.data.workspaces.push(ws);
    state.add_session(session);
    state
        .system
        .pty_handles
        .insert(case.session, case.handle.take().unwrap());
    crate::app::cleanup::stop_session(&mut state, case.session);
    crate::app::cleanup::shutdown(&mut state);
    assert!(
        !super::process_tree::running(case.leaf),
        "shutdown returned before its cleanup finished"
    );
}

#[test]
fn cleaning_one_launch_does_not_signal_another() {
    let mut first = Case::new("detached");
    let second = Case::new("detached");
    first.handle.as_mut().unwrap().kill().unwrap();
    first.assert_leaf_gone();
    assert!(super::process_tree::running(second.leaf));
    assert!(super::process_tree::running(
        second.handle.as_ref().unwrap().process_id.unwrap()
    ));
}

#[test]
fn a_late_exit_event_cannot_remove_a_new_launch() {
    let mut case = Case::new("detached");
    let mut state = AppState::default();
    let generation = case.handle.as_ref().unwrap().generation;
    state
        .system
        .pty_handles
        .insert(case.session, case.handle.take().unwrap());
    let (tx, _) = mpsc::unbounded_channel();
    let (pty_tx, _) = mpsc::channel(8);
    crate::app::handlers::session::handle_session_action(
        &mut state,
        Action::SessionExited(case.session, Uuid::new_v4(), 0),
        &PtyManager::new(),
        &tx,
        &pty_tx,
    )
    .unwrap();
    assert_eq!(
        state.system.pty_handles[&case.session].generation,
        generation
    );
    assert!(super::process_tree::running(case.leaf));
    crate::app::cleanup::shutdown(&mut state);
}
