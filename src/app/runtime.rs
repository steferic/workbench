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
        Err(e) => {
            eprintln!("Warning: Could not load saved state: {}", e);
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
        Err(e) => {
            eprintln!("Warning: Could not load config: {}", e);
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

    // Auto-start all sessions in "Working" workspaces
    start_all_working_sessions(&mut state, &pty_manager, &pty_tx, &action_tx);

    // Auto-activate first agent session in currently selected workspace (if it's Working)
    if let Some(ws) = state.selected_workspace() {
        if ws.status == crate::models::WorkspaceStatus::Working {
            let workspace_id = ws.id;
            // Find first agent session (not terminal) in this workspace
            if let Some(sessions) = state.data.sessions.get(&workspace_id) {
                if let Some(first_agent) = sessions.iter()
                    .find(|s| !s.agent_type.is_terminal())
                {
                    state.ui.active_session_id = Some(first_agent.id);
                }
            }
        }
    }

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

    loop {
        // Draw UI with effects
        terminal.draw(|frame| tui::ui::draw(frame, state, effects))?;

        // Handle events - batch process multiple PTY outputs to avoid UI starvation
        let action = events.next(state).await?;

        // Process action
        process_action(state, action.clone(), pty_manager, &action_tx, &pty_tx)?;

        // If we just processed a PTY output, drain more from the queue without redrawing
        // This prevents UI starvation during heavy output
        if matches!(action, Action::PtyOutput(_, _)) {
            let mut batch_count = 0;
            const MAX_BATCH: usize = 50; // Process up to 50 PTY outputs per frame

            while batch_count < MAX_BATCH {
                // Check for more PTY outputs without blocking
                if let Ok(next_action) = events.try_recv_pty_action() {
                    if matches!(next_action, Action::PtyOutput(_, _)) {
                        process_action(state, next_action, pty_manager, &action_tx, &pty_tx)?;
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

        if state.system.should_quit {
            break;
        }
    }

    Ok(())
}
