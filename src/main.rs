mod app;
mod models;
mod persistence;
mod pty;
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tokio::sync::mpsc;

use app::{Action, AppState, ConfigItem, Divider, FocusPanel, InputMode, TextSelection, UtilityItem, UtilitySection};
use models::{AgentType, Session, Workspace};
use pty::PtyManager;
use uuid::Uuid;
use tui::event::EventHandler;

#[derive(Parser)]
#[command(name = "workbench")]
#[command(author = "Stefan Lenoach")]
#[command(version = "0.1.0")]
#[command(about = "TUI for managing AI agent workspaces and sessions")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Start with a specific workspace directory
    #[arg(short, long)]
    workspace: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a workspace directory
    Add {
        /// Path to the workspace directory
        path: PathBuf,
        /// Custom name for the workspace
        #[arg(short, long)]
        name: Option<String>,
    },
    /// List all workspaces
    List,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Add { path, name }) => {
            let abs_path = if path.is_absolute() {
                path
            } else {
                std::env::current_dir()?.join(path)
            };
            println!(
                "Added workspace: {} at {:?}",
                name.unwrap_or_else(|| abs_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string()),
                abs_path
            );
        }
        Some(Commands::List) => {
            println!("Workspaces: (in-memory only, no persistence)");
        }
        None => {
            run_tui(cli.workspace).await?;
        }
    }

    Ok(())
}

async fn run_tui(initial_workspace: Option<PathBuf>) -> Result<()> {
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
    loop {
        // Draw UI
        terminal.draw(|frame| tui::ui::draw(frame, state))?;

        // Handle events
        let action = events.next(state).await?;

        // Process action
        process_action(state, action, pty_manager, &action_tx)?;

        if state.should_quit {
            break;
        }
    }

    Ok(())
}

fn process_action(
    state: &mut AppState,
    action: Action,
    pty_manager: &PtyManager,
    action_tx: &mpsc::UnboundedSender<Action>,
) -> Result<()> {
    match action {
        Action::Quit => {
            state.should_quit = true;
        }
        Action::Tick => {
            state.tick_animation();
            state.update_idle_queue();
        }
        Action::Resize(w, h) => {
            state.terminal_size = (w, h);
            resize_ptys_to_panes(state);
        }

        // Mouse click - determine which panel was clicked, check for divider, or start selection
        Action::MouseClick(x, y) => {
            let (w, h) = state.terminal_size;
            let main_height = h.saturating_sub(1); // Subtract status bar
            let left_width = (w as f32 * state.left_panel_ratio) as u16;
            let workspace_height = (main_height as f32 * state.workspace_ratio) as u16;

            // Check if clicking on a divider (within 2 pixels)
            let divider_tolerance = 2u16;

            // Left-right divider
            if x >= left_width.saturating_sub(divider_tolerance) && x <= left_width + divider_tolerance && y < main_height {
                state.dragging_divider = Some(Divider::LeftRight);
                state.drag_start_pos = Some((x, y));
                state.drag_start_ratio = state.left_panel_ratio;
                return Ok(());
            }

            // Workspace-session divider (within left panel)
            if x < left_width && y >= workspace_height.saturating_sub(divider_tolerance) && y <= workspace_height + divider_tolerance {
                state.dragging_divider = Some(Divider::WorkspaceSession);
                state.drag_start_pos = Some((x, y));
                state.drag_start_ratio = state.workspace_ratio;
                return Ok(());
            }

            // Output-pinned divider (only when split view is active)
            if state.should_show_split() {
                if let Some((ox, _, ow, _)) = state.output_pane_area {
                    let divider_x = ox + ow;
                    if x >= divider_x.saturating_sub(divider_tolerance) && x <= divider_x + divider_tolerance && y < main_height {
                        state.dragging_divider = Some(Divider::OutputPinned);
                        state.drag_start_pos = Some((x, y));
                        state.drag_start_ratio = state.output_split_ratio;
                        return Ok(());
                    }
                }

                // Check for dividers between pinned panes (horizontal dividers)
                let pinned_count = state.pinned_count();
                if pinned_count > 1 {
                    for pane_idx in 0..(pinned_count - 1) {
                        if let Some((_, py, _, ph)) = state.pinned_pane_areas[pane_idx] {
                            let divider_y = py + ph;
                            if y >= divider_y.saturating_sub(divider_tolerance) && y <= divider_y + divider_tolerance {
                                // Check if x is within the pinned pane area
                                if let Some((px, _, pw, _)) = state.pinned_pane_areas[0] {
                                    if x >= px && x < px + pw {
                                        state.dragging_divider = Some(Divider::PinnedPanes(pane_idx));
                                        state.drag_start_pos = Some((x, y));
                                        state.drag_start_ratio = state.pinned_pane_ratios[pane_idx];
                                        return Ok(());
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Not on a divider - handle panel focus and selection
            if y < main_height {
                if x < left_width {
                    // Left panel - clear any selection
                    state.text_selection = TextSelection::default();
                    clear_all_pinned_selections(state);
                    if y < workspace_height {
                        state.focus = FocusPanel::WorkspaceList;
                    } else {
                        // Lower left is split between sessions and utilities
                        let lower_left_height = main_height.saturating_sub(workspace_height);
                        let session_height = (lower_left_height as f32 * state.session_ratio) as u16;
                        let session_end_y = workspace_height + session_height;

                        if y < session_end_y {
                            state.focus = FocusPanel::SessionList;
                        } else {
                            state.focus = FocusPanel::UtilitiesPane;
                        }
                    }
                } else {
                    // Right panel - check if clicking on pinned pane or output pane
                    // Clear all selections first
                    state.text_selection = TextSelection::default();
                    clear_all_pinned_selections(state);

                    // Check if click is in any pinned pane area
                    let mut clicked_pinned_idx: Option<usize> = None;
                    for (idx, area_opt) in state.pinned_pane_areas.iter().enumerate() {
                        if let Some((px, py, pw, ph)) = area_opt {
                            if x >= *px && x < px + pw && y >= *py && y < py + ph {
                                clicked_pinned_idx = Some(idx);
                                break;
                            }
                        }
                    }

                    if let Some(idx) = clicked_pinned_idx {
                        state.focus = FocusPanel::PinnedTerminalPane(idx);
                        state.focused_pinned_pane = idx;

                        // Start selection in pinned pane
                        if let Some((area_x, area_y, _, _)) = state.pinned_pane_areas[idx] {
                            let text_col = (x.saturating_sub(area_x).saturating_sub(1)) as usize;
                            let text_row = (y.saturating_sub(area_y).saturating_sub(1)) as usize;

                            if let Some(sel) = state.pinned_text_selections.get_mut(idx) {
                                *sel = TextSelection {
                                    start: Some((text_row, text_col)),
                                    end: Some((text_row, text_col)),
                                    is_dragging: true,
                                };
                            }
                        }
                    } else {
                        // Output pane - start selection
                        state.focus = FocusPanel::OutputPane;

                        // Convert screen coordinates to text position
                        if let Some((area_x, area_y, _, _)) = state.output_pane_area {
                            // Account for border (1 pixel)
                            let text_col = (x.saturating_sub(area_x).saturating_sub(1)) as usize;
                            let text_row = (y.saturating_sub(area_y).saturating_sub(1) + state.output_scroll_offset) as usize;

                            state.text_selection = TextSelection {
                                start: Some((text_row, text_col)),
                                end: Some((text_row, text_col)),
                                is_dragging: true,
                            };
                        }
                    }
                }
            } else {
                // Clicked on status bar - clear selection
                state.text_selection = TextSelection::default();
                clear_all_pinned_selections(state);
            }
        }

        // Mouse drag - update selection end or resize divider
        Action::MouseDrag(x, y) => {
            // Handle divider dragging
            if let Some(divider) = state.dragging_divider {
                let (w, h) = state.terminal_size;
                let main_height = h.saturating_sub(1);

                match divider {
                    Divider::LeftRight => {
                        // Calculate new ratio based on x position
                        let new_ratio = (x as f32 / w as f32).clamp(0.15, 0.50);
                        state.left_panel_ratio = new_ratio;
                    }
                    Divider::WorkspaceSession => {
                        // Calculate new ratio based on y position
                        let new_ratio = (y as f32 / main_height as f32).clamp(0.20, 0.80);
                        state.workspace_ratio = new_ratio;
                    }
                    Divider::OutputPinned => {
                        // Calculate new ratio within right panel
                        let left_width = (w as f32 * state.left_panel_ratio) as u16;
                        let right_panel_width = w.saturating_sub(left_width);
                        let x_in_right = x.saturating_sub(left_width);
                        let new_ratio = (x_in_right as f32 / right_panel_width as f32).clamp(0.20, 0.80);
                        state.output_split_ratio = new_ratio;
                    }
                    Divider::PinnedPanes(pane_idx) => {
                        // Adjust ratios between adjacent pinned panes
                        let count = state.pinned_count();
                        if count > 1 && pane_idx < count - 1 {
                            // Get total height and calculate new distribution
                            let mut ratios = state.pinned_pane_ratios;
                            let sum: f32 = ratios.iter().take(count).sum();

                            // Calculate y position relative to pinned area start
                            if let Some((_, py, _, ph)) = state.pinned_pane_areas[0] {
                                let pinned_total_height = state.pinned_pane_areas.iter()
                                    .take(count)
                                    .filter_map(|a| a.map(|(_, _, _, h)| h))
                                    .sum::<u16>();

                                let y_in_pinned = y.saturating_sub(py) as f32;
                                let new_split = y_in_pinned / pinned_total_height as f32;

                                // Adjust the ratios of the two adjacent panes
                                let combined_ratio = ratios[pane_idx] + ratios[pane_idx + 1];
                                let ratio_above: f32 = ratios.iter().take(pane_idx).sum();

                                // New ratio for pane at pane_idx
                                let new_upper_ratio = ((new_split - ratio_above / sum) * sum).clamp(0.1, combined_ratio - 0.1);
                                ratios[pane_idx] = new_upper_ratio;
                                ratios[pane_idx + 1] = combined_ratio - new_upper_ratio;

                                state.pinned_pane_ratios = ratios;
                            }
                        }
                    }
                }
                return Ok(());
            }

            // Handle text selection dragging in output pane
            if state.text_selection.is_dragging {
                if let Some((area_x, area_y, _, _)) = state.output_pane_area {
                    let text_col = (x.saturating_sub(area_x).saturating_sub(1)) as usize;
                    let text_row = (y.saturating_sub(area_y).saturating_sub(1) + state.output_scroll_offset) as usize;
                    state.text_selection.end = Some((text_row, text_col));
                }
            }

            // Handle text selection dragging in pinned panes
            for (idx, sel) in state.pinned_text_selections.iter_mut().enumerate() {
                if sel.is_dragging {
                    if let Some((area_x, area_y, _, _)) = state.pinned_pane_areas[idx] {
                        let text_col = (x.saturating_sub(area_x).saturating_sub(1)) as usize;
                        let text_row = (y.saturating_sub(area_y).saturating_sub(1)) as usize;
                        sel.end = Some((text_row, text_col));
                    }
                }
            }
        }

        // Mouse up - finalize selection or divider drag
        Action::MouseUp(x, y) => {
            // Finalize divider dragging
            if state.dragging_divider.is_some() {
                state.dragging_divider = None;
                state.drag_start_pos = None;
                // Resize PTYs to match new pane sizes
                resize_ptys_to_panes(state);
                return Ok(());
            }

            // Finalize text selection in output pane
            if state.text_selection.is_dragging {
                if let Some((area_x, area_y, _, _)) = state.output_pane_area {
                    let text_col = (x.saturating_sub(area_x).saturating_sub(1)) as usize;
                    let text_row = (y.saturating_sub(area_y).saturating_sub(1) + state.output_scroll_offset) as usize;
                    state.text_selection.end = Some((text_row, text_col));
                }
                state.text_selection.is_dragging = false;

                // If start and end are the same, clear selection (it was just a click)
                if state.text_selection.start == state.text_selection.end {
                    state.text_selection = TextSelection::default();
                }
            }

            // Finalize text selection in pinned panes
            for (idx, sel) in state.pinned_text_selections.iter_mut().enumerate() {
                if sel.is_dragging {
                    if let Some((area_x, area_y, _, _)) = state.pinned_pane_areas[idx] {
                        let text_col = (x.saturating_sub(area_x).saturating_sub(1)) as usize;
                        let text_row = (y.saturating_sub(area_y).saturating_sub(1)) as usize;
                        sel.end = Some((text_row, text_col));
                    }
                    sel.is_dragging = false;

                    // If start and end are the same, clear selection (it was just a click)
                    if sel.start == sel.end {
                        *sel = TextSelection::default();
                    }
                }
            }
        }

        // Copy selected text to clipboard
        Action::CopySelection => {
            // Check if copying from output pane
            if let (Some(parser), Some(start), Some(end)) = (
                state.active_output(),
                state.text_selection.start,
                state.text_selection.end,
            ) {
                let text = extract_selected_text(parser.screen(), start, end);
                copy_to_clipboard(&text);
            }
            // Check if copying from any pinned pane
            for idx in 0..state.pinned_count() {
                let sel = &state.pinned_text_selections[idx];
                if let (Some(parser), Some(start), Some(end)) = (
                    state.pinned_terminal_output_at(idx),
                    sel.start,
                    sel.end,
                ) {
                    let text = extract_selected_text(parser.screen(), start, end);
                    copy_to_clipboard(&text);
                    break; // Only copy from one pane
                }
            }
            state.text_selection = TextSelection::default();
            clear_all_pinned_selections(state);
        }

        // Clear selection
        Action::ClearSelection => {
            state.text_selection = TextSelection::default();
            clear_all_pinned_selections(state);
        }

        // Pin/unpin terminal and toggle split view
        Action::PinSession(session_id) => {
            // Pin to the currently selected workspace
            let ws_idx = state.selected_workspace_idx;
            if ws_idx < state.workspaces.len() {
                let pinned = state.workspaces[ws_idx].pin_terminal(session_id);
                if pinned {
                    // Auto-enable split view
                    state.split_view_enabled = true;
                    // Focus the newly pinned pane
                    let new_idx = state.workspaces[ws_idx].pinned_terminal_ids.len().saturating_sub(1);
                    state.focused_pinned_pane = new_idx;
                    // Resize PTYs to match new pane configuration
                    resize_ptys_to_panes(state);
                    // Auto-save
                    let _ = persistence::save(&state.workspaces, &state.sessions);
                }
            }
        }
        Action::UnpinSession(session_id) => {
            if let Some(ws) = state.workspaces.get_mut(state.selected_workspace_idx) {
                ws.unpin_terminal(session_id);
                // Adjust focused pinned pane if needed
                let count = ws.pinned_terminal_ids.len();
                if state.focused_pinned_pane >= count && count > 0 {
                    state.focused_pinned_pane = count - 1;
                }
                // Resize PTYs
                resize_ptys_to_panes(state);
                // Auto-save
                let _ = persistence::save(&state.workspaces, &state.sessions);
            }
        }
        Action::UnpinFocusedSession => {
            let session_id = state.pinned_terminal_id_at(state.focused_pinned_pane);
            if let (Some(ws), Some(sid)) = (state.workspaces.get_mut(state.selected_workspace_idx), session_id) {
                ws.unpin_terminal(sid);
                // Adjust focused pinned pane if needed
                let count = ws.pinned_terminal_ids.len();
                if state.focused_pinned_pane >= count && count > 0 {
                    state.focused_pinned_pane = count - 1;
                }
                // If no more pinned terminals, switch focus back to session list
                if count == 0 {
                    state.focus = FocusPanel::SessionList;
                }
                // Resize PTYs
                resize_ptys_to_panes(state);
                // Auto-save
                let _ = persistence::save(&state.workspaces, &state.sessions);
            }
        }
        Action::ToggleSplitView => {
            state.split_view_enabled = !state.split_view_enabled;
            // Resize PTYs to match new pane configuration
            resize_ptys_to_panes(state);
        }
        Action::FocusPinnedPane(idx) => {
            if idx < state.pinned_count() {
                state.focused_pinned_pane = idx;
                state.focus = FocusPanel::PinnedTerminalPane(idx);
            }
        }
        Action::NextPinnedPane => {
            let count = state.pinned_count();
            if count > 0 {
                state.focused_pinned_pane = (state.focused_pinned_pane + 1) % count;
                state.focus = FocusPanel::PinnedTerminalPane(state.focused_pinned_pane);
            }
        }
        Action::PrevPinnedPane => {
            let count = state.pinned_count();
            if count > 0 {
                state.focused_pinned_pane = if state.focused_pinned_pane == 0 {
                    count - 1
                } else {
                    state.focused_pinned_pane - 1
                };
                state.focus = FocusPanel::PinnedTerminalPane(state.focused_pinned_pane);
            }
        }

        // Navigation
        Action::MoveUp => match state.focus {
            FocusPanel::WorkspaceList => {
                let prev_idx = state.selected_workspace_idx;
                state.selected_workspace_idx = state.selected_workspace_idx.saturating_sub(1);
                state.selected_session_idx = 0;
                // Start all stopped sessions if workspace changed
                if state.selected_workspace_idx != prev_idx {
                    start_workspace_sessions(state, pty_manager, action_tx);
                }
            }
            FocusPanel::SessionList => {
                state.selected_session_idx = state.selected_session_idx.saturating_sub(1);
            }
            _ => {}
        },
        Action::MoveDown => match state.focus {
            FocusPanel::WorkspaceList => {
                let prev_idx = state.selected_workspace_idx;
                if state.selected_workspace_idx < state.workspaces.len().saturating_sub(1) {
                    state.selected_workspace_idx += 1;
                    state.selected_session_idx = 0;
                }
                // Start all stopped sessions if workspace changed
                if state.selected_workspace_idx != prev_idx {
                    start_workspace_sessions(state, pty_manager, action_tx);
                }
            }
            FocusPanel::SessionList => {
                let session_count = state.sessions_for_selected_workspace().len();
                if state.selected_session_idx < session_count.saturating_sub(1) {
                    state.selected_session_idx += 1;
                }
            }
            _ => {}
        },
        Action::FocusLeft => {
            state.focus = match state.focus {
                FocusPanel::OutputPane => FocusPanel::SessionList,
                FocusPanel::PinnedTerminalPane(_) => FocusPanel::SessionList,
                FocusPanel::SessionList => FocusPanel::WorkspaceList,
                FocusPanel::UtilitiesPane => FocusPanel::SessionList,
                _ => state.focus,
            };
        }
        Action::FocusRight => {
            let prev_focus = state.focus;
            state.focus = match state.focus {
                FocusPanel::WorkspaceList => FocusPanel::SessionList,
                FocusPanel::SessionList => FocusPanel::OutputPane,
                FocusPanel::UtilitiesPane => FocusPanel::OutputPane,
                _ => state.focus,
            };
            // Start all stopped sessions when entering SessionList from WorkspaceList
            if prev_focus == FocusPanel::WorkspaceList && state.focus == FocusPanel::SessionList {
                start_workspace_sessions(state, pty_manager, action_tx);
            }
        }
        Action::FocusUtilitiesPane => {
            state.focus = FocusPanel::UtilitiesPane;
        }
        Action::ScrollOutputUp => {
            // Scroll the focused pane
            if let FocusPanel::PinnedTerminalPane(idx) = state.focus {
                if let Some(offset) = state.pinned_scroll_offsets.get_mut(idx) {
                    *offset = offset.saturating_add(1);
                }
            } else {
                state.output_scroll_offset = state.output_scroll_offset.saturating_sub(1);
            }
        }
        Action::ScrollOutputDown => {
            // Scroll the focused pane
            if let FocusPanel::PinnedTerminalPane(idx) = state.focus {
                if let Some(offset) = state.pinned_scroll_offsets.get_mut(idx) {
                    *offset = offset.saturating_sub(1);
                }
            } else {
                state.output_scroll_offset = state.output_scroll_offset.saturating_add(1);
            }
        }
        Action::ScrollOutputToBottom => {
            if let FocusPanel::PinnedTerminalPane(idx) = state.focus {
                if let Some(offset) = state.pinned_scroll_offsets.get_mut(idx) {
                    *offset = 0;
                }
            } else {
                state.output_scroll_offset = 0;
            }
        }
        Action::JumpToNextIdle => {
            state.update_idle_queue();
            if let Some(session_id) = state.pop_next_idle() {
                // Find which workspace this session belongs to
                let workspace_info = state.sessions.iter()
                    .find_map(|(ws_id, sessions)| {
                        sessions.iter().position(|s| s.id == session_id)
                            .map(|session_idx| (*ws_id, session_idx))
                    });

                if let Some((workspace_id, session_idx)) = workspace_info {
                    // Find workspace index
                    if let Some(ws_idx) = state.workspaces.iter().position(|w| w.id == workspace_id) {
                        state.selected_workspace_idx = ws_idx;
                        state.selected_session_idx = session_idx;
                    }
                }

                // Activate the session
                state.active_session_id = Some(session_id);
                state.focus = FocusPanel::OutputPane;
                state.output_scroll_offset = 0;
            }
        }

        // Mode changes
        Action::EnterHelpMode => {
            state.input_mode = InputMode::Help;
        }
        Action::EnterCreateWorkspaceMode => {
            state.input_mode = InputMode::CreateWorkspace;
            state.input_buffer.clear();
            // Initialize file browser at home directory
            state.file_browser_path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
            state.refresh_file_browser();
        }
        Action::EnterCreateSessionMode => {
            if state.selected_workspace().is_some() {
                state.input_mode = InputMode::CreateSession;
            }
        }
        Action::EnterCreateTerminalMode => {
            if state.selected_workspace().is_some() {
                state.input_mode = InputMode::CreateTerminal;
                state.input_buffer.clear();
            }
        }
        Action::ExitMode => {
            state.input_mode = InputMode::Normal;
            state.input_buffer.clear();
            state.editing_session_id = None;
        }
        Action::EnterSetStartCommandMode => {
            // Get session info first to avoid borrow issues
            let session_info = state.selected_session()
                .filter(|s| s.agent_type.is_terminal())
                .map(|s| (s.id, s.start_command.clone()));

            if let Some((session_id, existing_cmd)) = session_info {
                state.editing_session_id = Some(session_id);
                // Pre-fill with existing command if any
                state.input_buffer = existing_cmd.unwrap_or_default();
                state.input_mode = InputMode::SetStartCommand;
            }
        }
        Action::SetStartCommand(session_id, command) => {
            if let Some(session) = state.get_session_mut(session_id) {
                session.start_command = if command.is_empty() {
                    None
                } else {
                    Some(command)
                };
            }
            state.input_mode = InputMode::Normal;
            state.input_buffer.clear();
            state.editing_session_id = None;
            // Save state
            let _ = persistence::save(&state.workspaces, &state.sessions);
        }

        // Input handling
        Action::InputChar(c) => {
            state.input_buffer.push(c);
        }
        Action::InputBackspace => {
            state.input_buffer.pop();
        }
        Action::InputSubmit => {
            // Legacy - kept for compatibility
        }

        // File browser actions
        Action::FileBrowserUp => {
            if state.file_browser_selected > 0 {
                state.file_browser_selected -= 1;
                // Adjust scroll if needed
                if state.file_browser_selected < state.file_browser_scroll {
                    state.file_browser_scroll = state.file_browser_selected;
                }
            }
        }
        Action::FileBrowserDown => {
            if state.file_browser_selected < state.file_browser_entries.len().saturating_sub(1) {
                state.file_browser_selected += 1;
                // Adjust scroll if needed (assuming ~15 visible items)
                let visible_height = 15;
                if state.file_browser_selected >= state.file_browser_scroll + visible_height {
                    state.file_browser_scroll = state.file_browser_selected - visible_height + 1;
                }
            }
        }
        Action::FileBrowserEnter => {
            state.file_browser_enter_selected();
        }
        Action::FileBrowserBack => {
            state.file_browser_go_up();
        }
        Action::FileBrowserSelect => {
            // Select highlighted directory as workspace (or current dir if nothing highlighted)
            let path = if let Some(selected) = state.file_browser_entries.get(state.file_browser_selected) {
                selected.clone()
            } else {
                state.file_browser_path.clone()
            };
            if path.exists() && path.is_dir() {
                let workspace = Workspace::from_path(path);
                state.add_workspace(workspace);
                state.input_mode = InputMode::Normal;
                // Auto-save
                let _ = persistence::save(&state.workspaces, &state.sessions);
            }
        }

        // Utilities pane actions
        Action::SelectNextUtility => {
            match state.utility_section {
                UtilitySection::Utilities => {
                    let utilities = UtilityItem::all();
                    let current_idx = utilities.iter().position(|u| *u == state.selected_utility).unwrap_or(0);
                    if current_idx < utilities.len() - 1 {
                        state.selected_utility = utilities[current_idx + 1];
                    }
                }
                UtilitySection::GlobalConfig => {
                    let configs = ConfigItem::all();
                    let current_idx = configs.iter().position(|c| *c == state.selected_config).unwrap_or(0);
                    if current_idx < configs.len() - 1 {
                        state.selected_config = configs[current_idx + 1];
                    }
                }
            }
        }
        Action::SelectPrevUtility => {
            match state.utility_section {
                UtilitySection::Utilities => {
                    let utilities = UtilityItem::all();
                    let current_idx = utilities.iter().position(|u| *u == state.selected_utility).unwrap_or(0);
                    if current_idx > 0 {
                        state.selected_utility = utilities[current_idx - 1];
                    }
                }
                UtilitySection::GlobalConfig => {
                    let configs = ConfigItem::all();
                    let current_idx = configs.iter().position(|c| *c == state.selected_config).unwrap_or(0);
                    if current_idx > 0 {
                        state.selected_config = configs[current_idx - 1];
                    }
                }
            }
        }
        Action::ActivateUtility => {
            // Load utility content based on selected utility
            load_utility_content(state);
            // Clear active session so utility content shows
            state.active_session_id = None;
            state.focus = FocusPanel::OutputPane;
        }
        Action::ToggleUtilitySection => {
            state.utility_section = state.utility_section.toggle();
        }
        Action::ToggleConfigItem => {
            match state.selected_config {
                ConfigItem::ToggleBanner => {
                    state.banner_visible = !state.banner_visible;
                }
            }
            // Save config changes
            let config = persistence::GlobalConfig {
                banner_visible: state.banner_visible,
            };
            let _ = persistence::save_config(&config);
        }

        // Workspace operations
        Action::CreateWorkspace(path) => {
            if path.exists() && path.is_dir() {
                let workspace = Workspace::from_path(path);
                state.add_workspace(workspace);
            }
        }
        Action::SelectWorkspace(idx) => {
            if idx < state.workspaces.len() {
                state.selected_workspace_idx = idx;
                state.selected_session_idx = 0;
            }
        }
        Action::DeleteWorkspace(_id) => {
            // TODO: Implement workspace deletion
        }
        Action::ToggleWorkspaceStatus => {
            if let Some(ws) = state.workspaces.get_mut(state.selected_workspace_idx) {
                ws.toggle_status();
                // Auto-save
                let _ = persistence::save(&state.workspaces, &state.sessions);
            }
        }

        // Session operations
        Action::CreateSession(agent_type) => {
            if let Some(workspace) = state.selected_workspace() {
                let session = Session::new(workspace.id, agent_type.clone());
                let session_id = session.id;
                let workspace_path = workspace.path.clone();
                let ws_idx = state.selected_workspace_idx;

                // Touch the workspace to update last_active_at
                if let Some(ws) = state.workspaces.get_mut(ws_idx) {
                    ws.touch();
                }

                // Calculate PTY size based on output pane (accounting for split view)
                let pty_rows = state.pane_rows();
                let cols = state.output_pane_cols();

                // Create vt100 parser with many more rows for scrollback
                let parser_rows = 500;
                let parser = vt100::Parser::new(parser_rows, cols, 0);
                state.output_buffers.insert(session_id, parser);

                // Spawn PTY with actual viewport size
                match pty_manager.spawn_session(
                    session_id,
                    agent_type,
                    &workspace_path,
                    pty_rows,
                    cols,
                    action_tx.clone(),
                ) {
                    Ok(handle) => {
                        state.pty_handles.insert(session_id, handle);
                        state.add_session(session);
                        state.active_session_id = Some(session_id);
                        state.focus = FocusPanel::OutputPane;
                        // Auto-save
                        let _ = persistence::save(&state.workspaces, &state.sessions);
                    }
                    Err(e) => {
                        eprintln!("Failed to spawn session: {}", e);
                        state.output_buffers.remove(&session_id);
                    }
                }

                state.input_mode = InputMode::Normal;
            }
        }
        Action::CreateTerminal(name) => {
            if let Some(workspace) = state.selected_workspace() {
                let agent_type = models::AgentType::Terminal(name);
                let session = Session::new(workspace.id, agent_type.clone());
                let session_id = session.id;
                let workspace_path = workspace.path.clone();
                let ws_idx = state.selected_workspace_idx;

                // Touch the workspace to update last_active_at
                if let Some(ws) = state.workspaces.get_mut(ws_idx) {
                    ws.touch();
                }

                // Calculate PTY size based on output pane (accounting for split view)
                let pty_rows = state.pane_rows();
                let cols = state.output_pane_cols();

                // Create vt100 parser with many more rows for scrollback
                let parser_rows = 500;
                let parser = vt100::Parser::new(parser_rows, cols, 0);
                state.output_buffers.insert(session_id, parser);

                // Spawn PTY (terminals don't resume)
                match pty_manager.spawn_session(
                    session_id,
                    agent_type,
                    &workspace_path,
                    pty_rows,
                    cols,
                    action_tx.clone(),
                ) {
                    Ok(handle) => {
                        state.pty_handles.insert(session_id, handle);
                        state.add_session(session);
                        state.active_session_id = Some(session_id);
                        state.focus = FocusPanel::OutputPane;
                        // Auto-save
                        let _ = persistence::save(&state.workspaces, &state.sessions);
                    }
                    Err(e) => {
                        eprintln!("Failed to spawn terminal: {}", e);
                        state.output_buffers.remove(&session_id);
                    }
                }

                state.input_mode = InputMode::Normal;
            }
        }
        Action::SelectSession(idx) => {
            let session_count = state.sessions_for_selected_workspace().len();
            if idx < session_count {
                state.selected_session_idx = idx;
            }
        }
        Action::ActivateSession(session_id) => {
            state.active_session_id = Some(session_id);
            state.focus = FocusPanel::OutputPane;
            state.output_scroll_offset = 0;
        }
        Action::RestartSession(session_id) => {
            // Find the session and its workspace, including start_command for terminals
            let session_info = state.sessions.values().flatten()
                .find(|s| s.id == session_id)
                .map(|s| (s.agent_type.clone(), s.workspace_id, s.start_command.clone()));

            if let Some((agent_type, workspace_id, start_command)) = session_info {
                // Find the workspace path
                let workspace_path = state.workspaces.iter()
                    .find(|w| w.id == workspace_id)
                    .map(|w| w.path.clone());

                if let Some(workspace_path) = workspace_path {
                    // Calculate PTY size based on output pane
                    let pty_rows = state.pane_rows();
                    let cols = state.output_pane_cols();

                    // Create new vt100 parser with large buffer for scrollback
                    let parser_rows = 500;
                    let parser = vt100::Parser::new(parser_rows, cols, 0);
                    state.output_buffers.insert(session_id, parser);

                    // For terminals, don't resume. For agents, resume.
                    let resume = agent_type.is_agent();

                    // Spawn new PTY with resume flag
                    match pty_manager.spawn_session_with_resume(
                        session_id,
                        agent_type.clone(),
                        &workspace_path,
                        pty_rows,
                        cols,
                        action_tx.clone(),
                        resume,
                    ) {
                        Ok(handle) => {
                            state.pty_handles.insert(session_id, handle);
                            // Mark session as running
                            if let Some(session) = state.get_session_mut(session_id) {
                                session.status = models::SessionStatus::Running;
                            }
                            state.active_session_id = Some(session_id);
                            state.focus = FocusPanel::OutputPane;

                            // Send start command for terminals after a short delay
                            if agent_type.is_terminal() {
                                if let Some(cmd) = start_command {
                                    if !cmd.is_empty() {
                                        // Queue the command to be sent
                                        let tx = action_tx.clone();
                                        let sid = session_id;
                                        tokio::spawn(async move {
                                            // Wait for shell to initialize
                                            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
                                            let mut input = cmd.into_bytes();
                                            input.push(b'\n'); // Add newline to execute
                                            let _ = tx.send(Action::SendInput(sid, input));
                                        });
                                    }
                                }
                            }

                            // Auto-save
                            let _ = persistence::save(&state.workspaces, &state.sessions);
                        }
                        Err(e) => {
                            eprintln!("Failed to restart session: {}", e);
                            state.output_buffers.remove(&session_id);
                        }
                    }
                }
            }
        }
        Action::StopSession(session_id) => {
            // Send Ctrl+C to the session
            if let Some(handle) = state.pty_handles.get_mut(&session_id) {
                let _ = handle.send_input(&[0x03]); // Ctrl+C
            }
        }
        Action::KillSession(session_id) => {
            // Kill the PTY process
            if let Some(mut handle) = state.pty_handles.remove(&session_id) {
                let _ = handle.kill();
            }
            if let Some(session) = state.get_session_mut(session_id) {
                session.mark_stopped();
            }
            if state.active_session_id == Some(session_id) {
                state.active_session_id = None;
            }
            // Auto-save
            let _ = persistence::save(&state.workspaces, &state.sessions);
        }
        Action::DeleteSession(session_id) => {
            // Kill PTY if running, then remove session entirely
            if let Some(mut handle) = state.pty_handles.remove(&session_id) {
                let _ = handle.kill();
            }
            state.delete_session(session_id);
            // Adjust selection if needed
            let session_count = state.sessions_for_selected_workspace().len();
            if state.selected_session_idx >= session_count && session_count > 0 {
                state.selected_session_idx = session_count - 1;
            }
            // Auto-save
            let _ = persistence::save(&state.workspaces, &state.sessions);
        }

        // PTY interaction
        Action::SendInput(session_id, data) => {
            if let Some(handle) = state.pty_handles.get_mut(&session_id) {
                let _ = handle.send_input(&data);
            }
            // Touch the workspace containing this session
            if let Some(workspace_id) = state.sessions.iter()
                .find_map(|(ws_id, sessions)| {
                    if sessions.iter().any(|s| s.id == session_id) {
                        Some(*ws_id)
                    } else {
                        None
                    }
                })
            {
                if let Some(ws) = state.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    ws.touch();
                }
            }
        }
        Action::PtyOutput(session_id, data) => {
            if let Some(parser) = state.output_buffers.get_mut(&session_id) {
                parser.process(&data);
            }
            // Track activity
            state.last_activity.insert(session_id, std::time::Instant::now());
        }
        Action::SessionExited(session_id, _exit_code) => {
            state.pty_handles.remove(&session_id);
            if let Some(session) = state.get_session_mut(session_id) {
                session.mark_stopped();
            }
            // Auto-save
            let _ = persistence::save(&state.workspaces, &state.sessions);
        }
    }

    Ok(())
}

/// Extract selected text from vt100 screen based on selection start and end positions
fn extract_selected_text(
    screen: &vt100::Screen,
    start: (usize, usize),
    end: (usize, usize),
) -> String {
    let (rows, cols) = screen.size();

    // Order the selection (start should be before end)
    let (start_row, start_col, end_row, end_col) = if start.0 < end.0
        || (start.0 == end.0 && start.1 <= end.1)
    {
        (start.0, start.1, end.0, end.1)
    } else {
        (end.0, end.1, start.0, start.1)
    };

    let mut result = String::new();

    for row in start_row..=end_row.min(rows as usize - 1) {
        let row_start = if row == start_row { start_col } else { 0 };
        let row_end = if row == end_row {
            end_col.min(cols as usize)
        } else {
            cols as usize
        };

        let mut line = String::new();
        for col in row_start..=row_end {
            if let Some(cell) = screen.cell(row as u16, col as u16) {
                line.push_str(&cell.contents());
            }
        }

        // Trim trailing whitespace from each line
        let trimmed = line.trim_end();
        result.push_str(trimmed);

        // Add newline between rows (but not at the very end)
        if row < end_row && row < rows as usize - 1 {
            result.push('\n');
        }
    }

    result
}

/// Resize all PTYs and vt100 parsers to match their respective pane sizes
/// This accounts for which pane each session is displayed in (output vs pinned)
fn resize_ptys_to_panes(state: &mut AppState) {
    let output_cols = state.output_pane_cols();
    let pinned_cols = state.pinned_pane_cols();
    let rows = state.pane_rows();
    let parser_rows = 500u16; // Keep large scrollback

    // Get all pinned terminal IDs for the current workspace
    let pinned_ids = state.pinned_terminal_ids();

    // Resize each PTY and parser based on which pane it belongs to
    for (session_id, handle) in state.pty_handles.iter() {
        let cols = if pinned_ids.contains(session_id) {
            pinned_cols
        } else {
            output_cols
        };

        // Resize the PTY
        let _ = handle.resize(rows, cols);

        // Resize the vt100 parser
        if let Some(parser) = state.output_buffers.get_mut(session_id) {
            parser.set_size(parser_rows, cols);
        }
    }
}

/// Clear all pinned text selections
fn clear_all_pinned_selections(state: &mut AppState) {
    for sel in state.pinned_text_selections.iter_mut() {
        *sel = TextSelection::default();
    }
}

/// Copy text to clipboard using pbcopy on macOS
fn copy_to_clipboard(text: &str) {
    if text.is_empty() {
        return;
    }
    if let Ok(mut child) = std::process::Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        if let Some(stdin) = child.stdin.as_mut() {
            use std::io::Write;
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}

/// Start all stopped sessions in the selected workspace
fn start_workspace_sessions(
    state: &mut AppState,
    pty_manager: &PtyManager,
    action_tx: &mpsc::UnboundedSender<Action>,
) {
    use crate::models::SessionStatus;

    // Get workspace info
    let workspace = match state.selected_workspace() {
        Some(ws) => ws,
        None => return,
    };
    let workspace_id = workspace.id;
    let workspace_path = workspace.path.clone();

    // Find all stopped sessions in this workspace
    let stopped_sessions: Vec<(Uuid, AgentType)> = state.sessions
        .get(&workspace_id)
        .map(|sessions| {
            sessions.iter()
                .filter(|s| s.status == SessionStatus::Stopped)
                .map(|s| (s.id, s.agent_type.clone()))
                .collect()
        })
        .unwrap_or_default();

    if stopped_sessions.is_empty() {
        return;
    }

    // Calculate PTY size
    let pty_rows = state.pane_rows();
    let cols = state.output_pane_cols();
    let parser_rows = 500u16;

    // Start each stopped session
    for (session_id, agent_type) in stopped_sessions {
        // Create vt100 parser
        let parser = vt100::Parser::new(parser_rows, cols, 0);
        state.output_buffers.insert(session_id, parser);

        // Spawn PTY with resume flag for agents (not terminals)
        let resume: bool = agent_type.is_agent();
        match pty_manager.spawn_session_with_resume(
            session_id,
            agent_type,
            &workspace_path,
            pty_rows,
            cols,
            action_tx.clone(),
            resume,
        ) {
            Ok(handle) => {
                state.pty_handles.insert(session_id, handle);
                // Mark session as running
                if let Some(session) = state.get_session_mut(session_id) {
                    session.status = SessionStatus::Running;
                }
            }
            Err(e) => {
                eprintln!("Failed to start session: {}", e);
                state.output_buffers.remove(&session_id);
            }
        }
    }

    // Touch workspace and save
    if let Some(ws) = state.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
        ws.touch();
    }
    let _ = persistence::save(&state.workspaces, &state.sessions);
}

/// Load utility content based on the selected utility
fn load_utility_content(state: &mut AppState) {
    let workspace_path = match state.selected_workspace() {
        Some(ws) => ws.path.clone(),
        None => {
            state.utility_content = vec!["No workspace selected".to_string()];
            return;
        }
    };

    state.utility_scroll_offset = 0;

    match state.selected_utility {
        UtilityItem::Calendar => {
            load_calendar_content(state);
        }
        UtilityItem::GitHistory => {
            load_git_history(&workspace_path, state);
        }
        UtilityItem::FileTree => {
            load_file_tree(&workspace_path, state);
        }
    }
}

/// Load calendar showing when workspace was worked on
fn load_calendar_content(state: &mut AppState) {
    let mut content = vec![
        "".to_string(),
        "  Work History".to_string(),
        "  ============".to_string(),
        "".to_string(),
    ];

    // Show last active for each workspace
    for ws in &state.workspaces {
        let status_icon = match ws.status {
            crate::models::WorkspaceStatus::Working => "[W]",
            crate::models::WorkspaceStatus::Paused => "[P]",
        };
        let last_active = ws.last_active_display();
        content.push(format!("  {} {} - {}", status_icon, ws.name, last_active));
    }

    if state.workspaces.is_empty() {
        content.push("  No workspaces yet".to_string());
    }

    content.push("".to_string());
    content.push("  [W] = Working, [P] = Paused".to_string());

    state.utility_content = content;
}

/// Load git history for the workspace
fn load_git_history(workspace_path: &PathBuf, state: &mut AppState) {
    let output = std::process::Command::new("git")
        .args(["log", "--oneline", "-30"])
        .current_dir(workspace_path)
        .output();

    let mut content = vec![
        "".to_string(),
        "  Git History (last 30 commits)".to_string(),
        "  =============================".to_string(),
        "".to_string(),
    ];

    match output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                content.push(format!("  {}", line));
            }
            if stdout.is_empty() {
                content.push("  No commits yet".to_string());
            }
        }
        Ok(_) => {
            content.push("  Not a git repository".to_string());
        }
        Err(e) => {
            content.push(format!("  Error: {}", e));
        }
    }

    state.utility_content = content;
}

/// Load file tree for the workspace using git ls-files (respects .gitignore)
fn load_file_tree(workspace_path: &PathBuf, state: &mut AppState) {
    use std::collections::BTreeMap;

    let mut content = vec![
        "".to_string(),
        "  File Tree".to_string(),
        "  =========".to_string(),
        "".to_string(),
    ];

    // Get workspace name for root
    let ws_name = workspace_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".");

    // Try git ls-files first (respects .gitignore)
    let output = std::process::Command::new("git")
        .args(["ls-files"])
        .current_dir(workspace_path)
        .output();

    let files: Vec<String> = match output {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|s| s.to_string())
                .collect()
        }
        _ => {
            // Fallback: manual directory walk (limited)
            content.push(format!("  {}/", ws_name));
            content.push("  (not a git repository)".to_string());
            state.utility_content = content;
            return;
        }
    };

    if files.is_empty() {
        content.push(format!("  {}/", ws_name));
        content.push("  (no tracked files)".to_string());
        state.utility_content = content;
        return;
    }

    // Build tree structure: path -> children (BTreeMap for sorted order)
    #[derive(Default)]
    struct TreeNode {
        children: BTreeMap<String, TreeNode>,
        is_file: bool,
    }

    let mut root = TreeNode::default();

    for file_path in &files {
        let parts: Vec<&str> = file_path.split('/').collect();
        let mut current = &mut root;

        for (i, part) in parts.iter().enumerate() {
            let is_last = i == parts.len() - 1;
            current = current.children.entry(part.to_string()).or_default();
            if is_last {
                current.is_file = true;
            }
        }
    }

    // Render tree with visual characters
    content.push(format!("  {}/", ws_name));

    fn render_tree(
        node: &TreeNode,
        prefix: &str,
        content: &mut Vec<String>,
    ) {
        let entries: Vec<_> = node.children.iter().collect();
        let count = entries.len();

        for (i, (name, child)) in entries.iter().enumerate() {
            let is_last = i == count - 1;
            let connector = if is_last { "└── " } else { "├── " };
            let child_prefix = if is_last { "    " } else { "│   " };

            let display_name = if child.is_file && child.children.is_empty() {
                name.to_string()
            } else {
                format!("{}/", name)
            };

            content.push(format!("  {}{}{}", prefix, connector, display_name));

            // Recursively render children (but limit depth to avoid huge trees)
            if !child.children.is_empty() && prefix.len() < 40 {
                render_tree(child, &format!("{}{}", prefix, child_prefix), content);
            }
        }
    }

    render_tree(&root, "", &mut content);

    // Add file count
    content.push("".to_string());
    content.push(format!("  {} files tracked", files.len()));

    state.utility_content = content;
}

/// Start all sessions in "Working" workspaces on startup
fn start_all_working_sessions(
    state: &mut AppState,
    pty_manager: &PtyManager,
    action_tx: &mpsc::UnboundedSender<Action>,
) {
    use crate::models::{SessionStatus, WorkspaceStatus};

    // Get all Working workspace IDs and their paths
    let working_workspaces: Vec<(Uuid, std::path::PathBuf)> = state.workspaces.iter()
        .filter(|ws| ws.status == WorkspaceStatus::Working)
        .map(|ws| (ws.id, ws.path.clone()))
        .collect();

    // For each working workspace, start all stopped sessions
    for (workspace_id, workspace_path) in working_workspaces {
        // Find all stopped sessions in this workspace (include start_command for terminals)
        let stopped_sessions: Vec<(Uuid, AgentType, Option<String>)> = state.sessions
            .get(&workspace_id)
            .map(|sessions| {
                sessions.iter()
                    .filter(|s| s.status == SessionStatus::Stopped)
                    .map(|s| (s.id, s.agent_type.clone(), s.start_command.clone()))
                    .collect()
            })
            .unwrap_or_default();

        if stopped_sessions.is_empty() {
            continue;
        }

        // Calculate PTY size
        let pty_rows = state.pane_rows();
        let cols = state.output_pane_cols();
        let parser_rows = 500u16;

        // Start each stopped session
        for (session_id, agent_type, start_command) in stopped_sessions {
            // Create vt100 parser
            let parser = vt100::Parser::new(parser_rows, cols, 0);
            state.output_buffers.insert(session_id, parser);

            // Spawn PTY with resume flag for agents (not terminals)
            let resume: bool = agent_type.is_agent();
            match pty_manager.spawn_session_with_resume(
                session_id,
                agent_type.clone(),
                &workspace_path,
                pty_rows,
                cols,
                action_tx.clone(),
                resume,
            ) {
                Ok(handle) => {
                    state.pty_handles.insert(session_id, handle);
                    // Mark session as running
                    if let Some(session) = state.get_session_mut(session_id) {
                        session.status = SessionStatus::Running;
                    }

                    // Send start command for terminals after a short delay
                    if agent_type.is_terminal() {
                        if let Some(cmd) = start_command {
                            if !cmd.is_empty() {
                                let tx = action_tx.clone();
                                let sid = session_id;
                                tokio::spawn(async move {
                                    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
                                    let mut input = cmd.into_bytes();
                                    input.push(b'\n');
                                    let _ = tx.send(Action::SendInput(sid, input));
                                });
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to auto-start session: {}", e);
                    state.output_buffers.remove(&session_id);
                }
            }
        }

        // Touch workspace
        if let Some(ws) = state.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
            ws.touch();
        }
    }

    // Save state after starting sessions
    let _ = persistence::save(&state.workspaces, &state.sessions);
}
