use crate::app::{Action, AppState};
use crate::audio::AudioPlayer;
use crate::models::Workspace;
use crate::persistence;
use crate::pty::PtyManager;
use crate::tui;
use crate::tui::effects::EffectsManager;
use crate::tui::event::EventHandler;
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc;

use super::handler::process_action;
use super::session_start::{process_startup_queue, start_all_working_sessions};

pub async fn run_tui(initial_workspace: Option<PathBuf>) -> Result<()> {
    // Initialize terminal
    let mut terminal = tui::init()?;

    // Create app state and load persisted data
    let mut state = AppState::new();

    // Load persisted state
    match persistence::load() {
        Ok(persisted) => {
            state.data.workspaces = persisted.workspaces;
            state.data.sessions = persisted.sessions;
            // Load notepad content into TextArea widgets
            for (ws_id, content) in persisted.notepad_content {
                state.load_notepad_content(ws_id, content);
            }
            // Select first workspace in visual order (Working workspaces first)
            let visual_order = state.workspace_visual_order();
            if let Some(&first_idx) = visual_order.first() {
                state.ui.selected_workspace_idx = first_idx;
            }
        }
        Err(_e) => {
            // Don't use eprintln! in TUI - it corrupts the display
            // Failed to load saved state, will start fresh
        }
    }

    // Load global config
    match persistence::load_config() {
        Ok(config) => {
            state.ui.banner_visible = config.banner_visible;
            // Apply persisted pane ratios
            state.ui.left_panel_ratio = config.left_panel_ratio;
            state.ui.workspace_ratio = config.workspace_ratio;
            state.ui.sessions_ratio = config.sessions_ratio;
            state.ui.todos_ratio = config.todos_ratio;
            state.ui.output_split_ratio = config.output_split_ratio;
        }
        Err(_e) => {
            // Don't use eprintln! in TUI - it corrupts the display
            // Failed to load config, will use defaults
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

    // Create effects manager for animations
    let mut effects = EffectsManager::new();

    // Main loop
    let result = run_main_loop(
        &mut terminal,
        &mut state,
        &mut events,
        &pty_manager,
        action_tx,
        pty_tx,
        &mut effects,
    )
    .await;

    // Restore terminal
    tui::restore()?;

    result
}

async fn run_main_loop(
    terminal: &mut tui::Terminal,
    state: &mut AppState,
    events: &mut EventHandler,
    pty_manager: &PtyManager,
    action_tx: mpsc::UnboundedSender<Action>,
    pty_tx: mpsc::Sender<Action>,
    effects: &mut EffectsManager,
) -> Result<()> {
    // Audio player for brown noise (created lazily)
    let mut audio_player: Option<AudioPlayer> = None;
    let mut audio_was_playing = false;

    // Classical radio stream process
    let mut radio_process: Option<std::process::Child> = None;
    const WRTI_STREAM_URL: &str = "https://wrti-live.streamguys1.com/classical-mp3";

    // Local sound file processes (paths embedded at compile time)
    let mut ocean_process: Option<std::process::Child> = None;
    let mut ocean_was_playing = false;
    const OCEAN_WAV: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/sounds/ocean_waterside.wav");

    let mut chimes_process: Option<std::process::Child> = None;
    let mut chimes_was_playing = false;
    const CHIMES_WAV: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/sounds/wind_chimes.wav");

    let mut rain_process: Option<std::process::Child> = None;
    let mut rain_was_playing = false;
    const RAIN_WAV: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/sounds/rainforest_rain.wav");

    // Track if we've done initial session start after first render
    let mut initial_sessions_started = false;

    loop {
        // Start frame timing
        state.system.perf.frame_start();

        // Draw UI with effects
        terminal.draw(|frame| tui::ui::draw(frame, state, effects))?;

        // End frame timing (measures render + event processing)
        state.system.perf.frame_end();

        // After first render, start sessions with accurate pane dimensions
        // This is critical because nvim and other full-screen apps can't handle
        // resize events during startup - they lock to the first size they see
        if !initial_sessions_started && state.ui.output_pane_area.is_some() {
            start_all_working_sessions(state, pty_manager, &pty_tx, &action_tx);

            // Auto-activate first agent session in currently selected workspace
            if let Some(ws) = state.selected_workspace() {
                if ws.status == crate::models::WorkspaceStatus::Working {
                    let workspace_id = ws.id;
                    if let Some(sessions) = state.data.sessions.get(&workspace_id) {
                        if let Some(first_agent) = sessions.iter()
                            .find(|s| !s.agent_type.is_terminal())
                        {
                            state.ui.active_session_id = Some(first_agent.id);
                        }
                    }
                }
            }

            initial_sessions_started = true;
        }

        // Handle events - batch process multiple PTY outputs to avoid UI starvation
        let action = events.next(state).await?;

        // Process action
        process_action(state, action.clone(), pty_manager, &action_tx, &pty_tx)?;

        // If we just processed a PTY output, drain more from the queue without redrawing
        // This prevents UI starvation during heavy output
        if matches!(action, Action::PtyOutput(_, _)) {
            state.system.perf.record_pty_output(); // Track first PTY output
            let mut batch_count = 0;
            const MAX_BATCH: usize = 50; // Process up to 50 PTY outputs per frame

            while batch_count < MAX_BATCH {
                // Check for more PTY outputs without blocking
                if let Ok(next_action) = events.try_recv_pty_action() {
                    if matches!(next_action, Action::PtyOutput(_, _)) {
                        process_action(state, next_action, pty_manager, &action_tx, &pty_tx)?;
                        state.system.perf.record_pty_output(); // Track batched PTY output
                        batch_count += 1;
                    } else {
                        // Non-PTY action, process it and stop batching
                        process_action(state, next_action, pty_manager, &action_tx, &pty_tx)?;
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

        // Sync audio player with state
        if state.system.brown_noise_playing != audio_was_playing {
            if state.system.brown_noise_playing {
                // Start playing
                if audio_player.is_none() {
                    audio_player = AudioPlayer::new().ok();
                }
                if let Some(ref player) = audio_player {
                    player.play();
                }
            } else {
                // Stop playing
                if let Some(ref player) = audio_player {
                    player.pause();
                }
            }
            audio_was_playing = state.system.brown_noise_playing;
        }

        // Sync classical radio stream with state
        // Also check if process died and needs restart
        if let Some(ref mut child) = radio_process {
            // Check if process exited (non-blocking)
            if let Ok(Some(_)) = child.try_wait() {
                // Process died, clear it so it can restart
                radio_process = None;
            }
        }

        let should_play_radio = state.system.classical_radio_playing;
        let is_playing_radio = radio_process.is_some();

        if should_play_radio && !is_playing_radio {
            // Start streaming with VLC - more robust than ffplay for streams
            radio_process = std::process::Command::new("/opt/homebrew/bin/vlc")
                .args([
                    "--intf", "dummy",      // No GUI
                    "--no-video",           // Audio only
                    "--quiet",              // Suppress output
                    WRTI_STREAM_URL,
                ])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok();
        } else if !should_play_radio && is_playing_radio {
            // Stop streaming
            if let Some(mut child) = radio_process.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }

        // Sync ocean waves sound with state
        if state.system.ocean_waves_playing != ocean_was_playing {
            if state.system.ocean_waves_playing {
                if ocean_process.is_none() {
                    ocean_process = std::process::Command::new("ffplay")
                        .args(["-nodisp", "-loglevel", "quiet", "-loop", "0", OCEAN_WAV])
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()
                        .ok();
                }
            } else if let Some(mut child) = ocean_process.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            ocean_was_playing = state.system.ocean_waves_playing;
        }

        // Sync wind chimes sound with state
        if state.system.wind_chimes_playing != chimes_was_playing {
            if state.system.wind_chimes_playing {
                if chimes_process.is_none() {
                    chimes_process = std::process::Command::new("ffplay")
                        .args(["-nodisp", "-loglevel", "quiet", "-loop", "0", CHIMES_WAV])
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()
                        .ok();
                }
            } else if let Some(mut child) = chimes_process.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            chimes_was_playing = state.system.wind_chimes_playing;
        }

        // Sync rainforest rain sound with state
        if state.system.rainforest_rain_playing != rain_was_playing {
            if state.system.rainforest_rain_playing {
                if rain_process.is_none() {
                    rain_process = std::process::Command::new("ffplay")
                        .args(["-nodisp", "-loglevel", "quiet", "-loop", "0", RAIN_WAV])
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()
                        .ok();
                }
            } else if let Some(mut child) = rain_process.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            rain_was_playing = state.system.rainforest_rain_playing;
        }

        if state.system.should_quit {
            // Clean up all sound processes on quit
            if let Some(mut child) = radio_process.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            if let Some(mut child) = ocean_process.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            if let Some(mut child) = chimes_process.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            if let Some(mut child) = rain_process.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            break;
        }
    }

    Ok(())
}
