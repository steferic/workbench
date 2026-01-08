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
            state.notepad_content = persisted.notepad_content;
        }
        Err(e) => {
            eprintln!("Warning: Could not load saved state: {}", e);
        }
    }

    // Load global config
    match persistence::load_config() {
        Ok(config) => {
            state.banner_visible = config.banner_visible;
            // Apply persisted pane ratios
            state.left_panel_ratio = config.left_panel_ratio;
            state.workspace_ratio = config.workspace_ratio;
            state.sessions_ratio = config.sessions_ratio;
            state.todos_ratio = config.todos_ratio;
            state.output_split_ratio = config.output_split_ratio;
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

    // Auto-activate first agent session in currently selected workspace (if it's Working)
    if let Some(ws) = state.selected_workspace() {
        if ws.status == crate::models::WorkspaceStatus::Working {
            let workspace_id = ws.id;
            // Find first agent session (not terminal) in this workspace
            if let Some(sessions) = state.sessions.get(&workspace_id) {
                if let Some(first_agent) = sessions.iter()
                    .find(|s| !s.agent_type.is_terminal())
                {
                    state.active_session_id = Some(first_agent.id);
                }
            }
        }
    }

    // Create effects manager for animations
    let mut effects = EffectsManager::new();

    // Main loop
    let result = run_main_loop(&mut terminal, &mut state, &mut events, &pty_manager, action_tx, &mut effects).await;

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
    effects: &mut EffectsManager,
) -> Result<()> {
    // Audio player for brown noise (created lazily)
    let mut audio_player: Option<AudioPlayer> = None;
    let mut audio_was_playing = false;

    loop {
        // Draw UI with effects
        terminal.draw(|frame| tui::ui::draw(frame, state, effects))?;

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
