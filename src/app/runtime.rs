use crate::app::{Action, AppState, Toast, ToastLevel};
use crate::audio::{AudioPlayer, LoopingAudio};
use crate::models::Workspace;
use crate::persistence;
use crate::pty::PtyManager;
use crate::tui;
use crate::tui::event::EventHandler;
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc;

use super::handler::process_action;
use super::session_start::{process_startup_queue, queue_selected_workspace_sessions};

// Audio constants
const WRTI_STREAM_URL: &str = "https://wrti-live.streamguys1.com/classical-mp3";
const VLC_BINARY: &str = "vlc";
const OCEAN_WAV: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/sounds/ocean_waterside.wav"
);
const CHIMES_WAV: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/sounds/wind_chimes.wav");
const RAIN_WAV: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/sounds/rainforest_rain.wav"
);

/// Is anything on screen moving on its own — without an action arriving?
/// These are the only things that justify repainting on a bare tick: the
/// banner marquee, working/loading spinners, and drag auto-scroll (which
/// keeps scrolling while the mouse holds still at a pane edge).
fn has_ambient_animation(state: &AppState) -> bool {
    if state.ui.banner_visible {
        return true;
    }
    if state.drag_mouse_pos().is_some() {
        return true;
    }
    if !state.system.startup_queue.is_empty() {
        return true;
    }
    state
        .data
        .workspaces
        .iter()
        .any(|ws| state.is_workspace_working(ws.id))
}

fn stop_radio_process(mut child: std::process::Child) {
    if let Err(err) = child.kill() {
        crate::logger::warn(format!("failed to stop radio process: {err}"));
    }
    if let Err(err) = child.wait() {
        crate::logger::warn(format!("failed to wait for radio process: {err}"));
    }
}

pub async fn run_tui(initial_workspace: Option<PathBuf>, use_alternate_screen: bool) -> Result<()> {
    // First thing, before anything that could fail: if the last instance
    // never said goodbye, say so while the log around the old heartbeats is
    // still warm.
    crate::lifecycle::note_boot();
    crate::lifecycle::watch_termination();

    // Initialize terminal
    let mut terminal = tui::init(use_alternate_screen)?;

    // Create app state and load persisted data
    let mut state = AppState::new();
    state.system.use_alternate_screen = use_alternate_screen;

    // Load persisted state
    match persistence::load() {
        Ok(persisted) => {
            state.data.workspaces = persisted.workspaces;
            state.data.sessions = persisted.sessions;

            // Load notepad content into TextArea widgets
            for (ws_id, content) in persisted.notepad_content {
                state.load_notepad_content(ws_id, content);
            }
            // Select the first workspace in list order.
            let visual_order = state.workspace_visual_order();
            if let Some(&first_idx) = visual_order.first() {
                state.ui.selected_workspace_idx = first_idx;
            }
            // Seed the selected workspace's per-workspace UI state so read
            // accessors see `last_active_session_id` before any write occurs.
            state.ws_ui_mut();
        }
        Err(_e) => {
            state.ui.toasts.push_back(Toast::new(
                "Failed to load saved state — starting fresh".to_string(),
                ToastLevel::Warning,
                std::time::Duration::from_secs(4),
            ));
        }
    }

    // Load global config
    match persistence::load_config() {
        Ok(config) => {
            state.ui.banner_visible = config.banner_visible;
            // Apply persisted pane ratios
            state.ui.layout.left_panel_ratio = config.left_panel_ratio;
            state.ui.layout.workspace_ratio = config.workspace_ratio;
            state.ui.layout.sessions_ratio = config.sessions_ratio;
            state.ui.layout.tasks_ratio = config.tasks_ratio;
            state.ui.layout.output_split_ratio = config.output_split_ratio;
            state.ui.theme_mode = config.theme_mode;
            state.ui.selected_theme = config.theme_mode;
        }
        Err(_e) => {
            state.ui.toasts.push_back(Toast::new(
                "Failed to load config — using defaults".to_string(),
                ToastLevel::Warning,
                std::time::Duration::from_secs(4),
            ));
        }
    }

    // Get terminal size
    let size = terminal.size()?;
    state.system.terminal_size = (size.width, size.height);

    // Add initial workspace if provided (and not already present)
    if let Some(path) = initial_workspace {
        let abs_path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()?.join(path)
        };
        if abs_path.exists() && abs_path.is_dir() {
            // Check if workspace already exists
            let already_exists = state.data.workspaces.iter().any(|w| w.path == abs_path);
            if !already_exists {
                let workspace = Workspace::from_path(abs_path);
                state.add_workspace(workspace);
            }
        }
    }

    // Create event handler
    let mut events = EventHandler::new();
    let action_tx = events.action_sender();
    let pty_tx = events.pty_sender();

    // Create PTY manager
    let pty_manager = PtyManager::new();

    // NOTE: Session auto-start is deferred to AFTER first render in run_main_loop
    // This ensures we have accurate pane dimensions from the actual Layout
    // (nvim and other full-screen apps can't handle resize events during startup)

    // Main loop
    let result = run_main_loop(
        &mut terminal,
        &mut state,
        &mut events,
        &pty_manager,
        action_tx,
        pty_tx,
    )
    .await;

    // Restore terminal
    tui::restore(use_alternate_screen)?;

    // This is the one deliberate way out; everything else is what the boot
    // marker exists to catch.
    crate::lifecycle::note_clean_exit();

    result
}

/// Run one action, and keep the session alive when it fails.
///
/// An action that cannot be carried out is ordinary: "no workspace selected"
/// is what a keystroke means at the wrong moment, and the tree behind
/// `process_action` reaches git, the disk, and other processes, none of which
/// fail for reasons that have anything to do with the agents currently
/// running. Propagating that out of the loop ended the session and killed
/// every one of them — which is how workbench came to just stop and drop the
/// user back at a shell, with nothing in the log to say why.
///
/// So a failed action is reported and the loop goes on. The things that really
/// are fatal — the terminal itself going away — still end it, and now say so
/// on the way out.
fn dispatch(
    state: &mut AppState,
    action: Action,
    pty_manager: &PtyManager,
    action_tx: &mpsc::UnboundedSender<Action>,
    pty_tx: &mpsc::Sender<Action>,
) {
    if let Err(err) = process_action(state, action, pty_manager, action_tx, pty_tx) {
        crate::logger::warn(format!("action failed: {err:#}"));
    }
}

async fn run_main_loop(
    terminal: &mut tui::Terminal,
    state: &mut AppState,
    events: &mut EventHandler,
    pty_manager: &PtyManager,
    action_tx: mpsc::UnboundedSender<Action>,
    pty_tx: mpsc::Sender<Action>,
) -> Result<()> {
    // Audio player for brown noise (created lazily)
    let mut audio_player: Option<AudioPlayer> = None;
    let mut audio_was_playing = false;

    // Looping ambient audio processes
    let mut radio_process: Option<std::process::Child> = None;
    let mut ocean = LoopingAudio::new(OCEAN_WAV);
    let mut chimes = LoopingAudio::new(CHIMES_WAV);
    let mut rain = LoopingAudio::new(RAIN_WAV);

    // Track if we've done initial session start after first render
    let mut initial_sessions_started = false;

    // The input thread ticks every 50ms even when idle. Drawing on every tick
    // burns CPU repainting an unchanged screen, so draws are gated: real
    // actions draw immediately, ambient animations (spinners, banner marquee)
    // repaint on a slower cadence, and a heartbeat bounds how stale anything —
    // the header clock, a missed dirty flag — can ever get.
    const ANIMATION_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
    const IDLE_HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(1);
    let mut needs_draw = true;
    let mut last_draw = std::time::Instant::now();

    loop {
        let since_last_draw = last_draw.elapsed();
        let draw_now = needs_draw
            || since_last_draw >= IDLE_HEARTBEAT
            || (since_last_draw >= ANIMATION_INTERVAL && has_ambient_animation(state));

        if draw_now {
            // Start frame timing
            state.system.perf.frame_start();

            if let Err(err) = terminal.draw(|frame| tui::ui::draw(frame, state)) {
                crate::logger::warn(format!("giving up: the terminal could not be drawn: {err}"));
                return Err(err.into());
            }

            // End frame timing (measures render time)
            state.system.perf.frame_end();

            last_draw = std::time::Instant::now();
            needs_draw = false;

            // Sync PTY sizes to pane sizes now that this frame's layout rects
            // are stored (see request_pty_resize — doing this from action
            // handlers would use the previous layout and leave PTYs one
            // resize behind).
            if state.system.pty_resize_pending {
                state.system.pty_resize_pending = false;
                super::pty_ops::resize_ptys_to_panes(state);
            }
        }

        // After first render, start sessions with accurate pane dimensions
        // This is critical because nvim and other full-screen apps can't handle
        // resize events during startup - they lock to the first size they see
        if !initial_sessions_started && state.ui.output_pane_area.is_some() {
            queue_selected_workspace_sessions(state);

            // Auto-activate first agent session in currently selected workspace
            let first_agent_id = state
                .selected_workspace()
                .and_then(|ws| state.data.sessions.get(&ws.id))
                .and_then(|sessions| sessions.iter().find(|s| !s.agent_type.is_terminal()))
                .map(|s| s.id);
            if let Some(id) = first_agent_id {
                state.set_active_session_id(Some(id));
            }

            initial_sessions_started = true;
        }

        // Handle events - batch process multiple PTY outputs to avoid UI starvation
        let action = match events.next(state).await {
            Ok(action) => action,
            Err(err) => {
                crate::logger::warn(format!("giving up: input ended: {err:#}"));
                return Err(err);
            }
        };

        // Check discriminant before consuming the action to avoid cloning
        let is_pty_output = matches!(&action, Action::PtyOutput(_, _));

        // Anything but a bare tick changes state worth showing. A tick that
        // carries a queued palette action does too — it executes inside the
        // tick handler and shouldn't wait out the heartbeat.
        if !matches!(&action, Action::Tick) || state.ui.palette.pending_action.is_some() {
            needs_draw = true;
        }

        // ForceRedraw needs the terminal handle, which only this loop holds:
        // drop ratatui's back buffer so the next draw repaints every cell.
        if matches!(&action, Action::ForceRedraw) {
            terminal.clear()?;
        }

        // Process action (takes ownership, no clone needed)
        dispatch(state, action, pty_manager, &action_tx, &pty_tx);

        // If we just processed a PTY output, drain more from the queue without redrawing
        // This prevents UI starvation during heavy output
        if is_pty_output {
            state.system.perf.record_pty_output(); // Track first PTY output
            let mut batch_count = 0;
            const MAX_BATCH: usize = 50; // Process up to 50 PTY outputs per frame

            while batch_count < MAX_BATCH {
                // Check for more PTY outputs without blocking
                if let Ok(next_action) = events.try_recv_pty_action() {
                    if matches!(next_action, Action::PtyOutput(_, _)) {
                        dispatch(state, next_action, pty_manager, &action_tx, &pty_tx);
                        state.system.perf.record_pty_output(); // Track batched PTY output
                        batch_count += 1;
                    } else {
                        // Non-PTY action, process it and stop batching
                        dispatch(state, next_action, pty_manager, &action_tx, &pty_tx);
                        break;
                    }
                } else {
                    break; // No more actions in queue
                }
            }
        }

        // Process startup queue (staggered session startup - one per frame)
        if !state.system.startup_queue.is_empty() {
            process_startup_queue(state, pty_manager, &pty_tx, &action_tx);
        }

        // Persist pending state changes (debounced; the write runs off-loop)
        super::handlers::flush_dirty_state(state, &action_tx, false);

        // Sync audio player with state
        if state.system.brown_noise_playing != audio_was_playing {
            if state.system.brown_noise_playing {
                if audio_player.is_none() {
                    audio_player = AudioPlayer::new().ok();
                }
                if let Some(ref player) = audio_player {
                    player.play();
                }
            } else if let Some(ref player) = audio_player {
                player.pause();
            }
            audio_was_playing = state.system.brown_noise_playing;
        }

        // Sync classical radio stream (VLC-based, auto-restarts on crash)
        if let Some(ref mut child) = radio_process {
            if let Ok(Some(_)) = child.try_wait() {
                radio_process = None;
            }
        }
        let should_play_radio = state.system.classical_radio_playing;
        let is_playing_radio = radio_process.is_some();
        if should_play_radio && !is_playing_radio {
            radio_process = std::process::Command::new(VLC_BINARY)
                .args(["--intf", "dummy", "--no-video", "--quiet", WRTI_STREAM_URL])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok();
        } else if !should_play_radio && is_playing_radio {
            if let Some(child) = radio_process.take() {
                stop_radio_process(child);
            }
        }

        // Sync looping ambient sounds
        ocean.sync(state.system.ocean_waves_playing);
        chimes.sync(state.system.wind_chimes_playing);
        rain.sync(state.system.rainforest_rain_playing);

        // A SIGTERM/SIGHUP arrives here as a notice, not a death: log who
        // asked, then leave through the same door as Ctrl+Q — agents shut
        // down, terminal restored, marker rewritten. Before this, a polite
        // kill was indistinguishable from a power cut.
        if let Some(notice) = crate::lifecycle::termination_notice() {
            crate::logger::warn(format!("shutting down: asked to terminate by {notice}"));
            state.system.should_quit = true;
        }

        if state.system.should_quit {
            // Final synchronous save so pending changes survive shutdown.
            super::handlers::flush_dirty_state(state, &action_tx, true);
            if let Some(child) = radio_process.take() {
                stop_radio_process(child);
            }
            ocean.kill();
            chimes.kill();
            rain.kill();
            break;
        }
    }

    Ok(())
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;
    use crate::models::AgentType;

    /// The bug behind "workbench just stops": an action that could not be
    /// carried out returned `Err`, the error travelled out of the event loop,
    /// and the process ended — taking every running agent with it and leaving
    /// nothing in the log to explain the empty terminal.
    ///
    /// Starting a parallel task with no workspace selected is the cheapest
    /// example: "No workspace selected" is a sentence about a keystroke, not a
    /// reason to end the session.
    #[test]
    fn a_failing_action_no_longer_ends_the_session() {
        let pty = PtyManager::new();
        let (action_tx, _action_rx) = mpsc::unbounded_channel();
        let (pty_tx, _pty_rx) = mpsc::channel(8);

        let mut state = AppState::default();
        // Enough to get past the modal's own guards and reach the workspace
        // lookup, which is the thing that fails.
        state.ui.parallel_task.agents = vec![(AgentType::Claude, true)];
        state.ui.parallel_task.prompt = "do the thing".to_string();
        assert!(state.data.workspaces.is_empty());

        // It really does fail — without this the test would pass for the wrong
        // reason the day the action stops erroring.
        assert!(
            process_action(
                &mut state,
                Action::StartParallelTask,
                &pty,
                &action_tx,
                &pty_tx,
            )
            .is_err(),
            "expected this action to fail with no workspace"
        );

        // And the loop's dispatch absorbs it: returning at all is the assertion.
        dispatch(
            &mut state,
            Action::StartParallelTask,
            &pty,
            &action_tx,
            &pty_tx,
        );
    }
}
