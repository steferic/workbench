use super::*;
use crate::models::Workspace;
use crate::pty::proc_identity::{self, ProcStart};
use ratatui::{backend::TestBackend, Terminal};

fn fixture() -> AppState {
    let mut state = AppState::default();
    state.ui.banner_visible = false;
    state.data.workspaces = vec![
        Workspace::new("alpha".into(), "/projects/alpha".into()),
        Workspace::new("beta".into(), "/projects/beta".into()),
    ];
    state.system.dev_servers = vec![server(10, 3000, "alpha"), server(20, 4000, "beta")];
    state.set_sessions_tab(SessionsTab::Servers);
    reconcile(&mut state);
    state
}

fn server(pid: u32, port: u16, project: &str) -> DevServer {
    DevServer {
        pid,
        port,
        start: Some(ProcStart { sec: 1, usec: 0 }),
        command: "node".into(),
        cwd: format!("/projects/{project}").into(),
        loopback_only: true,
        ..DevServer::default()
    }
}

#[test]
fn selection_survives_insertions_and_scope_changes_without_retargeting_confirmation() {
    let mut state = fixture();
    let (tx, _) = tokio::sync::mpsc::unbounded_channel();
    move_selection(&mut state, false);
    let selected = state.ui.servers.selected;
    handle(&mut state, Action::ServerAskStop, &tx);
    state
        .system
        .dev_servers
        .insert(0, server(30, 2000, "alpha"));
    reconcile(&mut state);
    assert_eq!(state.ui.servers.selected, selected);
    handle(&mut state, Action::ServerScope, &tx);
    assert_eq!(rows(&state).len(), 2);
    assert_eq!(
        state.ui.servers.dialog.as_ref().unwrap().row.server.key(),
        selected.unwrap()
    );
    handle(&mut state, Action::ServerClose, &tx);
    assert!(
        state.ui.servers.stopping.is_empty(),
        "cancel never stops a process"
    );
}

#[test]
fn servers_and_click_targets_fit_small_windows_and_modals_own_the_mouse() {
    for (width, height) in [(120, 40), (80, 24), (60, 18), (20, 8)] {
        let mut state = fixture();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| crate::tui::ui::draw(f, &mut state))
            .unwrap();
        for (rect, _) in &state.ui.servers.hits {
            assert!(rect.right() <= width && rect.bottom() <= height, "{rect:?}");
        }
        if width >= 60 {
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect();
            assert!(text.contains("Servers"));
            assert!(text.contains(":3000"));
        }
        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        handle(&mut state, Action::ServerAskStop, &tx);
        terminal
            .draw(|f| crate::tui::ui::draw(f, &mut state))
            .unwrap();
        assert!(state.ui.link_hits.is_empty());
        assert!(state
            .ui
            .servers
            .hits
            .iter()
            .all(|(_, a)| matches!(a, Action::ServerConfirmStop | Action::ServerClose)));
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
struct ListeningChild {
    child: std::process::Child,
    dir: tempfile::TempDir,
    port: u16,
    worker: u32,
    worker_start: ProcStart,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl ListeningChild {
    fn spawn() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "app::servers::tests::listener_fixture",
                "--exact",
                "--ignored",
            ])
            .env("WORKBENCH_SERVER_FIXTURE", dir.path())
            .env("WORKBENCH_PROCESS_OWNER", "server-isolation-test")
            .current_dir(dir.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Ok(bytes) = std::fs::read(dir.path().join("ready.json")) {
                if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                    let worker = value["worker"].as_u64().unwrap() as u32;
                    return Self {
                        port: value["port"].as_u64().unwrap() as u16,
                        worker,
                        worker_start: proc_identity::start_time(worker).unwrap(),
                        child,
                        dir,
                    };
                }
            }
            if std::time::Instant::now() >= deadline || child.try_wait().unwrap().is_some() {
                let _ = child.kill();
                let _ = child.wait();
                panic!("listener fixture did not start");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    fn server(&self) -> DevServer {
        crate::ports::scan()
            .unwrap()
            .into_iter()
            .find(|s| s.pid == self.child.id() && s.port == self.port)
            .expect("real listener is scanned")
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl Drop for ListeningChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if proc_identity::owner(self.worker, self.worker_start) == proc_identity::PidOwner::Ours {
            unsafe {
                libc::kill(self.worker as i32, libc::SIGKILL);
            }
        }
    }
}

#[test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn stopping_a_server_closes_its_port_and_preserves_siblings_with_the_same_agent_marker() {
    let mut owned = ListeningChild::spawn();
    let mut sibling = ListeningChild::spawn();
    let server = owned.server();
    assert_eq!(
        server.cwd.canonicalize().unwrap(),
        owned.dir.path().canonicalize().unwrap()
    );
    let mut stale = server.clone();
    stale.start.as_mut().unwrap().sec += 1;
    assert!(crate::ports::stop(&stale).is_err());
    assert!(owned.child.try_wait().unwrap().is_none());
    crate::ports::stop(&server).unwrap();
    owned.child.wait().unwrap();
    assert!(
        !crate::pty::process_tree::running(owned.worker),
        "server's child exits too"
    );
    assert!(std::net::TcpStream::connect(server.endpoint()).is_err());
    assert!(
        sibling.child.try_wait().unwrap().is_none(),
        "never stop the owning agent's other servers"
    );
    assert!(std::net::TcpStream::connect(("127.0.0.1", sibling.port)).is_ok());
    assert!(
        crate::ports::stop(&server).is_err(),
        "an exited row cannot target a replacement process"
    );
}

#[tokio::test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
async fn keyboard_and_mouse_controls_stop_only_the_confirmed_server() {
    use crate::app::{process_action, FocusPanel};
    use crate::tui::event::EventHandler;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut first = ListeningChild::spawn();
    let mut second = ListeningChild::spawn();
    let mut state = AppState::default();
    state.ui.banner_visible = false;
    state.ui.focus = FocusPanel::SessionList;
    state.data.workspaces = vec![
        Workspace::new("alpha".into(), first.dir.path().into()),
        Workspace::new("beta".into(), second.dir.path().into()),
    ];
    let pty = crate::pty::PtyManager::new();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (pty_tx, _) = tokio::sync::mpsc::channel(1);
    let result = scan(workspace_roots(&state));
    process_action(&mut state, Action::PortsScanned(result), &pty, &tx, &pty_tx).unwrap();
    let press = |state: &mut AppState, code| {
        let action = EventHandler::test_key(KeyEvent::new(code, KeyModifiers::NONE), state);
        process_action(state, action, &pty, &tx, &pty_tx).unwrap();
    };
    press(&mut state, KeyCode::Tab);
    assert_eq!(state.sessions_tab(), SessionsTab::Terminals);
    press(&mut state, KeyCode::Tab);
    assert_eq!(state.sessions_tab(), SessionsTab::Servers);
    press(&mut state, KeyCode::Char('a'));
    assert_eq!(rows(&state).len(), 1);
    press(&mut state, KeyCode::Char('a'));
    assert_eq!(rows(&state).len(), 2);
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|f| crate::tui::ui::draw(f, &mut state))
        .unwrap();
    let hit = state
        .ui
        .servers
        .hits
        .iter()
        .find(|(_, a)| matches!(a, Action::SelectServer(key) if key.pid == second.child.id()))
        .unwrap()
        .0;
    process_action(
        &mut state,
        Action::MouseClick(hit.x, hit.y),
        &pty,
        &tx,
        &pty_tx,
    )
    .unwrap();
    assert_eq!(state.ui.servers.selected.unwrap().pid, second.child.id());
    press(&mut state, KeyCode::Enter);
    assert!(!state.ui.servers.dialog.as_ref().unwrap().confirming);
    press(&mut state, KeyCode::Char('x'));
    assert!(state.ui.servers.dialog.as_ref().unwrap().confirming);
    press(&mut state, KeyCode::Esc);
    assert!(second.child.try_wait().unwrap().is_none());
    press(&mut state, KeyCode::Char('x'));
    press(&mut state, KeyCode::Enter);
    let stopped = tokio::time::timeout(std::time::Duration::from_secs(8), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(&stopped, Action::ServerStopped(_, Ok(()))),
        "{stopped:?}"
    );
    process_action(&mut state, stopped, &pty, &tx, &pty_tx).unwrap();
    let result = scan(workspace_roots(&state));
    process_action(&mut state, Action::PortsScanned(result), &pty, &tx, &pty_tx).unwrap();
    assert_eq!(rows(&state).len(), 1);
    assert!(first.child.try_wait().unwrap().is_none());
    second.child.wait().unwrap();
    assert!(state.ui.servers.stopping.is_empty());
    assert!(state
        .ui
        .servers
        .message
        .as_ref()
        .unwrap()
        .starts_with("Stopped"));
}

#[test]
#[ignore = "child fixture for server lifecycle tests"]
fn listener_fixture() {
    let dir = PathBuf::from(std::env::var_os("WORKBENCH_SERVER_FIXTURE").expect("fixture only"));
    let start = 20000 + (std::process::id() % 20000) as u16;
    let listener = (start..49000)
        .find_map(|p| std::net::TcpListener::bind(("127.0.0.1", p)).ok())
        .unwrap();
    let mut worker = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    std::fs::write(
        dir.join("ready.json"),
        serde_json::json!({"port":listener.local_addr().unwrap().port(),"worker":worker.id()})
            .to_string(),
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_secs(60));
    let _ = worker.kill();
    let _ = worker.wait();
}
