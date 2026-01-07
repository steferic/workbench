use crate::app::{Action, AppState};
use crate::audio::AudioPlayer;
use crate::models::Workspace;
use crate::persistence;
use crate::pty::PtyManager;
use crate::tui;
use crate::tui::event::EventHandler;
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc;

use super::handler::process_action;
use super::session_start::start_all_working_sessions;

pub async fn run_tui(initial_workspace: Option<PathBuf>) -> Result<()> {
    // Initialize terminal
    let mut terminal = tui::init()?;

    // Create app state and load persisted data
    let mut state = AppState::new();

    // Load persisted state
    match persistence::load() {
        Ok(persisted) => {
            state.workspaces = persisted.workspaces;
            state.sessions = persisted.sessions;
        }
        Err(e) => {
            eprintln!("Warning: Could not load saved state: {}", e);
        }
    }

    // Load global config
    match persistence::load_config() {
        Ok(config) => {
            state.banner_visible = config.banner_visible;
        }
        Err(e) => {
            eprintln!("Warning: Could not load config: {}", e);
        }
    }

    // Get terminal size
    let size = terminal.size()?;
    state.terminal_size = (size.width, size.height);

    // Add initial workspace if provided (and not already present)
    if let Some(path) = initial_workspace {
        let abs_path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()?.join(path)
        };
        if abs_path.exists() && abs_path.is_dir() {
            // Check if workspace already exists
            let already_exists = state.workspaces.iter().any(|w| w.path == abs_path);
            if !already_exists {
                let workspace = Workspace::from_path(abs_path);
                state.add_workspace(workspace);
            }
        }
    }

    // Create event handler
    let mut events = EventHandler::new();
    let action_tx = events.action_sender();

    // Create PTY manager
    let pty_manager = PtyManager::new();

    // Auto-start all sessions in "Working" workspaces
    start_all_working_sessions(&mut state, &pty_manager, &action_tx);

    // Main loop
    let result = run_main_loop(&mut terminal, &mut state, &mut events, &pty_manager, action_tx).await;

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
) -> Result<()> {
    // Audio player for brown noise (created lazily)
    let mut audio_player: Option<AudioPlayer> = None;
    let mut audio_was_playing = false;

    loop {
        // Draw UI
        terminal.draw(|frame| tui::ui::draw(frame, state))?;

        // Handle events
        let action = events.next(state).await?;

        // Process action
        process_action(state, action, pty_manager, &action_tx)?;

        // Sync audio player with state
        if state.brown_noise_playing != audio_was_playing {
            if state.brown_noise_playing {
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
            audio_was_playing = state.brown_noise_playing;
        }

        if state.should_quit {
            break;
        }
    }

    Ok(())
}
