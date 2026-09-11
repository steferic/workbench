//! Opt-in visual fixture: no real agents, state persistence, or provider processes.
//! WORKBENCH_MEDIA_FIXTURE_DIR=/tmp/... cargo test media::fixture::visual -- --ignored --nocapture
use super::*;

#[test]
#[ignore = "interactive fixture; requires WORKBENCH_MEDIA_FIXTURE_DIR"]
fn visual() {
    let dir = std::path::PathBuf::from(
        std::env::var_os("WORKBENCH_MEDIA_FIXTURE_DIR").expect("fixture directory"),
    );
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("preview.png");
    let mut pixels = image::RgbImage::new(640, 360);
    for (x, y, p) in pixels.enumerate_pixels_mut() {
        *p = image::Rgb([
            ((x / 40 + y / 40) % 2 * 100 + 50) as u8,
            (x * 255 / 640) as u8,
            (y * 255 / 360) as u8,
        ]);
    }
    pixels.save(&source).unwrap();
    let video = dir.join("demo.mp4");
    assert!(std::process::Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x360:rate=24",
            "-t",
            "12",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart"
        ])
        .arg(&video)
        .status()
        .unwrap()
        .success());
    let mut state = crate::app::AppState::default();
    let workspace = crate::models::Workspace::new("Media test".into(), dir.clone());
    let mut session =
        crate::models::Session::new(workspace.id, crate::models::AgentType::Claude, false);
    session.mark_stopped();
    let sid = session.id;
    let owner = session.short_id();
    state.add_workspace(workspace);
    state.add_session(session);
    state.set_active_session_id(Some(sid));
    let shared = state.system.remote_state.clone();
    crate::remote::publish(&mut state, &shared);
    let (commands, mut incoming) = tokio::sync::mpsc::unbounded_channel();
    assert!(
        crate::control::socket_path().unwrap().starts_with(&dir),
        "fixture requires its own WORKBENCH_CONTROL_SOCK"
    );
    let _control = crate::control::start(shared.clone(), commands).unwrap();
    let library = preview::library(&state);
    let still = library.present(owner.clone(), &source).unwrap();
    let movie = library.present(owner, &video).unwrap();
    let still_url = library.local_url(&still.view.id).unwrap();
    let video_url = library.local_url(&movie.view.id).unwrap();
    let mobile = crate::remote::visual_fixture(shared.clone());
    crate::remote::publish(&mut state, &shared);
    std::fs::write(dir.join("ready.json"),serde_json::json!({"image":still_url,"video":video_url,"mobile":mobile,"agent":state.active_session().unwrap().short_id()}).to_string()).unwrap();
    if std::env::var_os("WORKBENCH_MEDIA_FIXTURE_TUI").is_some() {
        let mut terminal = crate::tui::init(true).unwrap();
        state.system.media_picker = ratatui_image::picker::Picker::from_query_stdio()
            .unwrap_or_else(|_| ratatui_image::picker::Picker::halfblocks());
        let mut parser = vt100::Parser::new(20, 100, 100);
        parser.process(format!("Media preview test\r\n\r\n\x1b]8;;{still_url}\x07\x1b[4mClick this embedded image link\x1b[0m\x1b]8;;\x07\r\n\r\nText selection should still work.\r\nUse Ctrl+P then Preview latest media to reopen the video.").as_bytes());
        state.system.output_buffers.insert(sid, parser);
        preview::show(&mut state, &still.view.id);
        let mut events = crate::tui::event::EventHandler::new();
        let action_tx = events.action_sender();
        let pty_tx = events.pty_sender();
        let pty = crate::pty::PtyManager::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            while !dir.join("stop").exists() {
                while let Ok(crate::remote::RemoteCommand::ShowMedia { id }) = incoming.try_recv() {
                    preview::show(&mut state, &id);
                }
                crate::remote::publish(&mut state, &shared);
                preview::flush_cleanup(&mut state);
                terminal
                    .draw(|f| crate::tui::ui::draw(f, &mut state))
                    .unwrap();
                match events.next(&state).await.unwrap() {
                    crate::app::Action::Tick => {}
                    crate::app::Action::InitiateQuit | crate::app::Action::Quit => break,
                    crate::app::Action::Resize(w, h) => state.system.terminal_size = (w, h),
                    action => {
                        let _ = crate::app::process_action(
                            &mut state, action, &pty, &action_tx, &pty_tx,
                        );
                    }
                }
            }
        });
        preview::close(&mut state);
        preview::flush_cleanup(&mut state);
        crate::tui::restore(true).unwrap();
    } else {
        for _ in 0..1200 {
            if dir.join("stop").exists() {
                break;
            }
            while incoming.try_recv().is_ok() {}
            crate::remote::publish(&mut state, &shared);
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
    library.shutdown();
}
