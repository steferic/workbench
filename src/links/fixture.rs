//! Isolated terminal fixture driven by tests/terminal-links.py. No real agents
//! are started or saved, and the driver substitutes a recording opener.
use crate::app::{Action, AppState};
use crate::models::{AgentType, Session, Workspace};
use ratatui::layout::Rect;

#[test]
#[ignore = "PTY fixture; run python3 tests/terminal-links.py"]
fn terminal() {
    let dir = std::path::PathBuf::from(
        std::env::var_os("WORKBENCH_LINK_FIXTURE").expect("isolated fixture directory"),
    );
    let mut state = AppState::default();
    state.ui.banner_visible = false;
    let workspace = Workspace::new("Links test".into(), dir.clone());
    let mut session = Session::new(workspace.id, AgentType::Claude, false);
    session.mark_stopped();
    let sid = session.id;
    state.add_workspace(workspace);
    state.add_session(session);
    state.set_active_session_id(Some(sid));
    let mut parser = vt100::Parser::new(20, 80, 20);
    parser.process(b"\x1b[?25l\x1b]8;;https://example.com/hidden?a=1;b=2\x1b\\reference\x1b]8;;\x1b\\\r\nhttps://example.com/plain\r\nordinary text");
    state.system.output_buffers.insert(sid, parser);
    let mut terminal = crate::tui::init(true).unwrap();
    let mut events = crate::tui::event::EventHandler::new();
    let action_tx = events.action_sender();
    let pty_tx = events.pty_sender();
    let pty = crate::pty::PtyManager::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    runtime.block_on(async {
        let mut step = 0;
        let mut last_event = String::from("ready");
        loop {
            let mut point_highlighted = false;
            terminal.draw(|frame| {
                crate::tui::ui::draw(frame, &mut state);
                if let Some(hit) = state.ui.link_hits.first() {
                    let Rect { x, y, .. } = hit.area;
                    point_highlighted = frame.buffer_mut()[(x, y)].modifier.contains(ratatui::style::Modifier::REVERSED);
                }
            }).unwrap();
            super::flush_pointer(&mut state);
            let hits: Vec<_> = state.ui.link_hits.iter().map(|hit| {
                serde_json::json!({"x":hit.area.x, "y":hit.area.y, "width":hit.area.width, "url":hit.target})
            }).collect();
            let selection = state.text_selection();
            let frame = serde_json::json!({"step":step, "event":last_event, "hits":hits,
                "highlighted":point_highlighted,
                "selection": selection.start.is_some() && selection.start != selection.end,
                "mode":format!("{:?}",state.ui.input_mode)});
            std::fs::write(dir.join("frame.tmp"), frame.to_string()).unwrap();
            std::fs::rename(dir.join("frame.tmp"), dir.join("frame.json")).unwrap();
            if std::time::Instant::now() >= deadline || dir.join("stop").exists() { break; }
            let action = events.next(&state).await.unwrap();
            if !matches!(action, Action::Tick) {
                last_event = format!("{action:?}");
            }
            match action {
                Action::Tick => {},
                Action::InitiateQuit | Action::Quit => break,
                action => crate::app::process_action(&mut state, action, &pty, &action_tx, &pty_tx).unwrap(),
            }
            step += 1;
        }
    });
    crate::tui::restore(true).unwrap();
}
