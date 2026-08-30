use crate::app::{Action, AppState};
use crate::git;
use crate::pty::PtyManager;
use anyhow::Result;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use super::handlers::{config, input, navigation, parallel, session, tasks, workspace};
use super::pty_ops::request_pty_resize;

/// Send an action onto the dispatch channel, logging on failure instead of
/// silently swallowing the error like a bare `let _ = tx.send(...)` did.
/// A closed channel typically means the runtime is shutting down or has
/// crashed; in either case the user benefits from a log entry over silence.
fn dispatch_action(tx: &mpsc::UnboundedSender<Action>, action: Action) {
    if let Err(err) = tx.send(action) {
        crate::logger::warn(format!("action channel closed; dropped event: {err}"));
    }
}

pub fn process_action(
    state: &mut AppState,
    action: Action,
    pty_manager: &PtyManager,
    action_tx: &mpsc::UnboundedSender<Action>,
    pty_tx: &mpsc::Sender<Action>,
) -> Result<()> {
    match action {
        Action::Quit | Action::ConfirmQuit => {
            // Kill all active sessions before quitting
            let handles: Vec<_> = state.system.pty_handles.drain().collect();
            for (session_id, handle) in handles {
                // Check if this is a terminal session
                let is_terminal = state
                    .data
                    .sessions
                    .values()
                    .flat_map(|sessions| sessions.iter())
                    .find(|s| s.id == session_id)
                    .map(|s| s.agent_type.is_terminal())
                    .unwrap_or(false);

                session::terminate_session_handle(handle, is_terminal);
            }
            state.system.should_quit = true;
        }
        Action::Tick => {
            // Check for pending palette action
            if let Some(palette_action) = state.ui.palette.pending_action.take() {
                process_action(state, palette_action, pty_manager, action_tx, pty_tx)?;
            }

            // Remove expired toasts
            state.ui.toasts.retain(|t| !t.is_expired());

            state.tick_animation();
            navigation::handle_drag_auto_scroll(state);
            let newly_idle = state.update_idle_queue();

            // Agent-to-agent comms: transcript/roster export, consult
            // delivery and reply capture (see app::comms_tick).
            super::comms_tick::tick(state, action_tx, &newly_idle);

            // Process newly idle sessions
            for session_id in &newly_idle {
                if let Some(workspace_id) = state.workspace_id_for_session(*session_id) {
                    // Check if this is a parallel task session
                    let parallel_info = state.get_workspace(workspace_id).and_then(|ws| {
                        ws.parallel_tasks
                            .iter()
                            .find(|t| t.attempts.iter().any(|a| a.session_id == *session_id))
                            .and_then(|t| {
                                t.attempts
                                    .iter()
                                    .find(|a| a.session_id == *session_id)
                                    .map(|a| (t.full_prompt(), a.prompt_sent, a.status))
                            })
                    });

                    if let Some((full_prompt, prompt_sent, attempt_status)) = parallel_info {
                        use crate::models::AttemptStatus;

                        if !prompt_sent {
                            // Send the prompt to the agent
                            let text_bytes: Vec<u8> = full_prompt.bytes().collect();
                            dispatch_action(action_tx, Action::SendInput(*session_id, text_bytes));
                            dispatch_action(action_tx, Action::SendInput(*session_id, vec![b'\r']));

                            // Mark the prompt as sent
                            if let Some(ws) = state.get_workspace_mut(workspace_id) {
                                for task in ws.parallel_tasks.iter_mut() {
                                    if let Some(attempt) = task
                                        .attempts
                                        .iter_mut()
                                        .find(|a| a.session_id == *session_id)
                                    {
                                        attempt.prompt_sent = true;
                                    }
                                }
                            }
                        } else if attempt_status == AttemptStatus::Running {
                            // Agent already received prompt and is now idle again - it's done!
                            dispatch_action(
                                action_tx,
                                Action::ParallelAttemptCompleted(*session_id),
                            );
                        }
                    }
                }
            }

            refresh_agent_status(state);
            health_tick(state);
            remote_tick(state, action_tx);
            sync_repository_map(state);
            super::todo_dispatch::tick(state, action_tx);
            tasks::sync_selection(state);
            refresh_agent_tasks(state, action_tx);
            refresh_scrollback(state, action_tx);

            scan_ports(state, action_tx);

            // Refresh diff stats every 5 seconds
            if state.system.last_diff_refresh.elapsed() >= Duration::from_secs(5) {
                state.system.last_diff_refresh = std::time::Instant::now();

                // Collect unique (path, Option<base>) pairs
                let mut diff_requests: HashMap<std::path::PathBuf, Option<String>> = HashMap::new();

                for ws in &state.data.workspaces {
                    // Workspace path → diff vs HEAD (uncommitted changes)
                    diff_requests.entry(ws.path.clone()).or_insert(None);

                    // Parallel task attempts → diff vs source_branch
                    for task in &ws.parallel_tasks {
                        for attempt in &task.attempts {
                            diff_requests
                                .entry(attempt.worktree_path.clone())
                                .or_insert_with(|| Some(task.source_branch.clone()));
                        }
                    }

                    // Session worktrees → diff vs main workspace branch
                    if let Some(sessions) = state.data.sessions.get(&ws.id) {
                        for session in sessions {
                            if let Some(ref wt_path) = session.worktree_path {
                                if !diff_requests.contains_key(wt_path) {
                                    // Use the workspace's current branch as base
                                    let base = git::get_current_branch_fast(&ws.path);
                                    diff_requests.insert(wt_path.clone(), base);
                                }
                            }
                        }
                    }
                }

                if !diff_requests.is_empty() {
                    let tx = action_tx.clone();
                    tokio::task::spawn_blocking(move || {
                        let mut stats = HashMap::new();
                        for (path, base) in diff_requests {
                            if path.exists() {
                                let stat = git::get_diff_shortstat(&path, base.as_deref());
                                stats.insert(path, stat);
                            }
                        }
                        dispatch_action(&tx, Action::DiffStatsUpdated(stats));
                    });
                }
            }
        }
        Action::UtilityContentLoaded(payload) => {
            if payload.request_id == state.ui.utility_request_id {
                state.ui.utility_content = payload.content;
                state.ui.pie_chart_data = payload.pie_chart_data;
                state.ui.show_calendar = payload.show_calendar;
            }
        }
        Action::ScrollbackLoaded {
            session_id,
            lines,
            log_size,
            cols,
        } => {
            state.system.scrollback_inflight = false;
            state
                .system
                .scrollback_state
                .insert(session_id, (log_size, cols, state.ui.theme_mode));
            if let Some(transcript) = state.system.transcript_buffers.get_mut(&session_id) {
                transcript.set_log_history(Some(lines));
            }
        }
        Action::DiffStatsUpdated(stats) => {
            state.system.diff_stats = stats;
        }
        Action::PortsScanned(servers) => {
            state.system.dev_servers = servers;
            state.system.port_scan_inflight = false;
            expose_project_servers(state);
        }
        Action::PushEndpointGone(endpoint) => {
            // Said once, at the moment of dropping — not on every notification.
            if state.system.push.forget(&endpoint) {
                crate::logger::info(format!(
                    "dropping a push subscription the service says is gone: {}",
                    crate::remote::push_origin(&endpoint)
                ));
                if let Err(err) = state.system.push.save() {
                    crate::logger::warn(format!("could not store the push list: {err}"));
                }
            }
        }
        Action::OpenRepositoryMap => open_repository_map(state),
        Action::Resize(w, h) => {
            state.system.terminal_size = (w, h);
            request_pty_resize(state);
        }
        // Dispatch to specialized handlers
        _ => {
            // Try each handler in turn. They return Ok(()) if they handled it or ignored it.
            // Since Action is consumed, we need to clone it if we were chaining, but here we can just pattern match.
            // Actually, my handlers take Action by value. I need to dispatch based on action variant.
            // But implementing a huge match again here defeats the purpose.
            // The specialized handlers internally match on the actions they care about and ignore others.
            // So I should clone the action? No, Action might not be cloneable (it is derived Clone though).
            // Better: match here and call the right handler.

            // Creating a new project or opening an existing one appends a workspace.
            // Detect that below (after dispatch) to bootstrap its default sessions.
            let workspaces_before = state.data.workspaces.len();
            let opens_workspace = matches!(
                action,
                Action::CreateNewWorkspace(_) | Action::FileBrowserSelect
            );

            match action {
                // The provider picked for an unassigned proposal. Reuse an
                // idle agent of that kind already in the project — spawning a
                // second Claude to avoid queueing on the first is how a
                // machine drowns — and only spawn when there is none.
                Action::AssignProposalAgent(agent_type, skip_permissions, with_worktree) => {
                    let Some((workspace_id, proposal_id)) = state.ui.assign.take() else {
                        state.ui.input_mode = crate::app::InputMode::Normal;
                        return Ok(());
                    };
                    state.ui.input_mode = crate::app::InputMode::Normal;

                    let existing = state
                        .data
                        .sessions
                        .get(&workspace_id)
                        .into_iter()
                        .flatten()
                        .filter(|s| {
                            s.status == crate::models::SessionStatus::Running
                                && s.agent_type.is_directable()
                                && s.agent_type.command() == agent_type.command()
                        })
                        .map(|s| (s.id, state.activity(s.id).is_free()))
                        .max_by_key(|(_, free)| *free)
                        .map(|(id, _)| id);

                    let session_id = match existing {
                        Some(id) => Some(id),
                        None => super::handlers::session::create_session_in(
                            state,
                            workspace_id,
                            agent_type,
                            skip_permissions,
                            with_worktree,
                            pty_manager,
                            action_tx,
                            pty_tx,
                        ),
                    };
                    let Some(session_id) = session_id else {
                        state.ui.set_task_status("Could not start an agent for it");
                        return Ok(());
                    };
                    let short = crate::models::Session::short_id_of(session_id);
                    if let Some(stored) = state
                        .data
                        .workspaces
                        .iter_mut()
                        .find(|ws| ws.id == workspace_id)
                        .and_then(|ws| ws.proposals.iter_mut().find(|p| p.id == proposal_id))
                    {
                        stored.agent = Some(short);
                    }
                    match super::handlers::tasks::decide_proposal(
                        state,
                        workspace_id,
                        proposal_id,
                        true,
                        action_tx,
                    ) {
                        Ok(message) | Err(message) => state.ui.set_task_status(message),
                    }
                    return Ok(());
                }
                // Workspace actions
                Action::InitiateDeleteWorkspace(_, _) | Action::ConfirmDeleteWorkspace |
                Action::EnterWorkspaceActionMode |
                Action::NextWorkspaceChoice | Action::PrevWorkspaceChoice |
                Action::ConfirmWorkspaceChoice | Action::EnterWorkspaceNameMode |
                Action::CreateNewWorkspace(_) => {
                    workspace::handle_workspace_action(state, action, pty_manager, action_tx, pty_tx)?;
                }

                // Session actions
                Action::CreateSession(_, _, _) | Action::CreateSessionIn(_, _, _, _) | Action::CreateTerminal |
                Action::ActivateSession(_) | Action::RestartSession(_) | Action::StopSession(_) |
                Action::KillSession(_) | Action::InitiateDeleteSession(_, _) |
                Action::ConfirmDeleteSession | Action::CancelPendingDelete | Action::EnterCreateSessionMode | Action::EnterCreateManagerMode |
                Action::EnterSetStartCommandMode | Action::SetStartCommand(_, _) | Action::PinSession(_) |
                Action::UnpinSession(_) | Action::UnpinFocusedSession | Action::ToggleSplitView |
                Action::SessionExited(_, _) | Action::PtyOutput(_, _) | Action::SendInput(_, _) |
                Action::MergeSessionWorktree(_) | Action::SwitchToWorktree(_) |
                Action::ConfirmMergeWithCommit | Action::CancelMerge |
                Action::SessionWorktreeMergeChecked { .. } |
                Action::SessionWorktreeMergeFinished { .. } |
                Action::SessionWorktreeCreated { .. } => {
                    session::handle_session_action(state, action, pty_manager, action_tx, pty_tx)?;
                }

                // Tasks pane actions
                Action::SelectNextTask | Action::SelectPrevTask | Action::ToggleTasksTab |
                Action::FocusSelectedTaskAgent |
                Action::EnterTaskEditMode(_) | Action::SendTaskMessage(_) |
                Action::DeleteSelectedTodo | Action::MoveSelectedTodo(_) |
                Action::ToggleTodoQueuePaused | Action::ClearCompletedTodos |
                Action::EditObjective(_) | Action::DeleteObjective |
                Action::CycleObjectiveState | Action::MoveObjective(_) |
                Action::ApproveProposal | Action::DeclineProposal |
                Action::DeskDecide(_) | Action::DeskOpen |
                Action::OpenDetail | Action::CloseDetail | Action::DeskDecideDetail(_) |
                Action::VerificationFinished { .. } |
                Action::AgentTasksRefreshed(_) |
                Action::ActivateUtility => {
                    tasks::handle_task_action(state, action, action_tx)?;
                }

                // Navigation actions
                Action::MoveUp | Action::MoveDown | Action::FocusLeft | Action::FocusRight |
                Action::NextPinnedPane | Action::PrevPinnedPane | Action::ScrollOutputUp |
                Action::ScrollOutputDown | Action::MouseScrollUp(_, _) |
                Action::MouseScrollDown(_, _) | Action::CycleNextWorkspace | Action::CyclePrevWorkspace | Action::CycleNextSession | Action::CyclePrevSession |
                Action::MouseClick(_, _) |
                Action::MouseDrag(_, _) | Action::MouseUp(_, _) | Action::CopySelection |
                Action::Paste(_) | Action::ClearSelection | Action::SelectNextUtility |
                Action::SelectPrevUtility | Action::ToggleUtilitySection |
                Action::ToggleBrownNoise | Action::ToggleClassicalRadio |
                Action::ToggleOceanWaves | Action::ToggleWindChimes | Action::ToggleRainforestRain => {
                    navigation::handle_navigation_action(state, action, pty_manager, pty_tx)?;
                }

                // Command palette actions
                Action::EnterCommandPalette | Action::ExitCommandPalette |
                Action::CommandPaletteExecute | Action::CommandPaletteDown |
                Action::CommandPaletteUp | Action::CommandPaletteInput(_) |
                Action::CommandPaletteBackspace |
                // Input actions
                Action::ExitMode | Action::InputChar(_) |
                Action::InputBackspace | Action::NotepadInput(_) |
                Action::FileBrowserUp | Action::FileBrowserDown | Action::FileBrowserEnter |
                Action::FileBrowserBack | Action::FileBrowserSelect |
                // Parallel task modal input actions
                Action::EnterParallelTaskMode | Action::ToggleParallelAgent(_) |
                Action::NextParallelAgent | Action::PrevParallelAgent |
                // Quit confirmation actions
                Action::InitiateQuit | Action::CancelQuit => {
                    input::handle_input_action(state, action)?;
                }

                // Parallel task execution actions
                Action::StartParallelTask | Action::CancelParallelTask(_) |
                Action::ParallelAttemptCompleted(_) |
                Action::ParallelWorktreesReady { .. } | Action::ParallelWorktreesFailed { .. } |
                Action::ParallelMergeFinished { .. } |
                Action::SelectNextReport | Action::SelectPrevReport |
                Action::ViewReport | Action::MergeSelectedReport |
                Action::ConfirmParallelMerge | Action::CancelParallelMerge => {
                    parallel::handle_parallel_action(state, action, pty_manager, action_tx, pty_tx)?;
                }

                // Toast notifications
                Action::ShowToast(msg, level) => {
                    use crate::app::Toast;
                    let duration = match level {
                        crate::app::ToastLevel::Error => std::time::Duration::from_secs(5),
                        _ => std::time::Duration::from_secs(3),
                    };
                    state.ui.toasts.push_back(Toast::new(msg, level, duration));
                    // Keep at most 5 toasts visible
                    while state.ui.toasts.len() > 5 {
                        state.ui.toasts.pop_front();
                    }
                }
                Action::TestToast => {
                    use crate::app::{Toast, ToastLevel};
                    let messages: [(&str, ToastLevel); 4] = [
                        ("Session started successfully", ToastLevel::Success),
                        ("Workspace has uncommitted changes", ToastLevel::Warning),
                        ("Failed to create worktree", ToastLevel::Error),
                        ("Merge complete", ToastLevel::Info),
                    ];
                    let idx = state.system.animation_frame % messages.len();
                    let (msg, level) = messages[idx];
                    state.ui.toasts.push_back(Toast::new(
                        msg.to_string(),
                        level,
                        std::time::Duration::from_secs(3),
                    ));
                    while state.ui.toasts.len() > 5 {
                        state.ui.toasts.pop_front();
                    }
                }

                // Debug overlay toggle
                Action::ToggleDebugOverlay => {
                    state.ui.show_debug_overlay = !state.ui.show_debug_overlay;
                }

                // Config window actions
                Action::EnterConfigWindow => {
                    state.ui.input_mode = crate::app::InputMode::ConfigWindow;
                    state.ui.config.tab = crate::app::ConfigTab::Agents;
                    state.ui.config.selected_row = 0;
                    state.ui.config.selected_col = 0;
                    state.ui.config.editing = false;
                    state.ui.config.rebinding = false;
                }
                Action::ExitConfigWindow => {
                    state.ui.input_mode = crate::app::InputMode::Normal;
                    state.ui.config.editing = false;
                    state.ui.config.rebinding = false;
                }
                Action::ConfigSwitchTab(_) | Action::ConfigMoveUp | Action::ConfigMoveDown |
                Action::ConfigMoveLeft | Action::ConfigMoveRight | Action::ConfigStartEdit |
                Action::ConfigFinishEdit | Action::ConfigCancelEdit | Action::ConfigAddAgent |
                Action::ConfigDeleteAgent | Action::ConfigReorderUp | Action::ConfigReorderDown |
                Action::ConfigResetDefault | Action::ConfigInputChar(_) |
                Action::ConfigInputBackspace | Action::ConfigRebindKey(_) => {
                    config::handle_config_action(state, action);
                }

                // Global already handled
                Action::Quit | Action::ConfirmQuit | Action::Tick | Action::Resize(_, _) |
                Action::ForceRedraw | Action::OpenRepositoryMap |
                Action::UtilityContentLoaded(_) | Action::DiffStatsUpdated(_) |
                Action::PortsScanned(_) | Action::PushEndpointGone(_) |
                Action::ScrollbackLoaded { .. } => {}
            }

            // A new project was just created or opened: select it and start its
            // default sessions (one Claude agent + two pinned terminals).
            if opens_workspace && state.data.workspaces.len() > workspaces_before {
                state.ui.selected_workspace_idx = state.data.workspaces.len() - 1;
                state.set_active_session_id(None);
                state.set_selected_session_idx(0);
                session::start_default_workspace_sessions(state, pty_manager, action_tx, pty_tx);
            }
        }
    }

    Ok(())
}

fn canvas_workspaces(state: &AppState) -> Vec<crate::canvas::CanvasWorkspace> {
    state
        .data
        .workspaces
        .iter()
        .map(|workspace| {
            crate::canvas::CanvasWorkspace::new(
                workspace.id.to_string(),
                workspace.name.clone(),
                workspace.path.clone(),
            )
        })
        .collect()
}

/// Keep the server's allowed roots aligned with the projects visible in the
/// TUI. The server never accepts a path from the browser, only one of these
/// opaque ids, so this list is also the traversal security boundary.
fn sync_repository_map(state: &mut AppState) {
    if state.system.canvas.is_none() {
        return;
    }
    let workspaces = canvas_workspaces(state);
    if let Some(canvas) = state.system.canvas.as_ref() {
        canvas.replace_workspaces(workspaces);
    }

    let commands = state
        .system
        .canvas
        .as_ref()
        .map(crate::canvas::CanvasServer::take_commands)
        .unwrap_or_default();
    for command in commands {
        dispatch_canvas_command(state, command);
    }
}

fn dispatch_canvas_command(state: &mut AppState, command: crate::canvas::CanvasCommand) {
    let workspace_path = state
        .data
        .workspaces
        .iter()
        .find(|workspace| workspace.id.to_string() == command.workspace)
        .map(|workspace| workspace.path.clone());
    let Some(workspace_path) = workspace_path else {
        if let Some(canvas) = state.system.canvas.as_ref() {
            canvas.fail(&command.request_id, "This workspace is no longer available.");
        }
        return;
    };
    if let Some(canvas) = state.system.canvas.as_ref() {
        canvas.launch_agent(command, workspace_path);
    }
}

fn open_repository_map(state: &mut AppState) {
    use crate::app::{Toast, ToastLevel};

    let Some(selected) = state
        .selected_workspace()
        .map(|workspace| workspace.id.to_string())
    else {
        state.ui.toasts.push_back(Toast::new(
            "Open a workspace before launching the repository map".into(),
            ToastLevel::Warning,
            Duration::from_secs(4),
        ));
        return;
    };
    let workspaces = canvas_workspaces(state);
    if state.system.canvas.is_none() {
        match crate::canvas::CanvasServer::start(workspaces.clone()) {
            Ok(canvas) => state.system.canvas = Some(canvas),
            Err(err) => {
                state.ui.toasts.push_back(Toast::new(
                    err.to_string(),
                    ToastLevel::Error,
                    Duration::from_secs(5),
                ));
                return;
            }
        }
    }

    let Some(canvas) = state.system.canvas.as_ref() else {
        return;
    };
    canvas.replace_workspaces(workspaces);
    let url = canvas.url(Some(&selected));
    match crate::canvas::open_browser(&url) {
        Ok(()) => state.ui.toasts.push_back(Toast::new(
            "Repository map opened in your browser".into(),
            ToastLevel::Success,
            Duration::from_secs(3),
        )),
        Err(err) => state.ui.toasts.push_back(Toast::new(
            format!("{err}; open {url}"),
            ToastLevel::Error,
            Duration::from_secs(7),
        )),
    }
}

/// How often the agent session logs are re-read for the tasks pane. Each pass
/// is usually a stat per agent (nothing new to parse), so this is cheap; it
/// still runs off-thread because locating a log can touch many directories.
const TASK_REFRESH_INTERVAL: Duration = Duration::from_millis(1000);
/// How often to look for growth in the agents' session logs. A polling rate,
/// not a correctness knob — the parse itself is exact.
const SCROLLBACK_REFRESH_INTERVAL: Duration = Duration::from_millis(1500);

/// How often agent hook reports are re-read. Fast enough that a permission
/// prompt surfaces while you are still looking at the pane, cheap enough to
/// run inline: a handful of small files per workspace with a live agent.
const STATUS_REFRESH_INTERVAL: Duration = Duration::from_millis(300);

/// Pull in what agents reported about themselves since the last pass.
///
/// The hook wrote one file per session keyed by the short session id it
/// inherited through `WORKBENCH_SESSION`; this resolves those back to
/// sessions and drops reports for sessions that no longer exist.
fn refresh_agent_status(state: &mut AppState) {
    if state.system.last_status_refresh.elapsed() < STATUS_REFRESH_INTERVAL {
        return;
    }
    state.system.last_status_refresh = std::time::Instant::now();

    // Only workspaces with a running agent can have anything new to say.
    let live: Vec<(uuid::Uuid, HashMap<String, uuid::Uuid>)> = state
        .data
        .workspaces
        .iter()
        .filter_map(|workspace| {
            let sessions = state.data.sessions.get(&workspace.id)?;
            let by_short: HashMap<String, uuid::Uuid> = sessions
                .iter()
                .filter(|s| {
                    s.agent_type.is_agent() && s.status == crate::models::SessionStatus::Running
                })
                .map(|s| (s.short_id(), s.id))
                .collect();
            (!by_short.is_empty()).then_some((workspace.id, by_short))
        })
        .collect();

    for (workspace_id, by_short) in live {
        for (short_id, status) in crate::agent_status::load_all(&workspace_id.to_string()) {
            let Some(session_id) = by_short.get(&short_id) else {
                continue;
            };
            // A report older than the current process describes the previous
            // one: restarting a session reuses its short id, so the file left
            // by the run before it must not be read as this run's state.
            if let Some(spawned_at) = state.system.session_spawned_at.get(session_id) {
                if status.at < *spawned_at {
                    continue;
                }
            }
            state.system.agent_status.insert(*session_id, status);
        }
    }
}

/// Start the tailnet server if we can, publish what the phone should see, and
/// apply whatever it asked for.
///
/// Commands are applied here rather than in the server thread so they take the
/// same path as the TUI's own keys: no locks, no racing the event loop.
fn remote_tick(state: &mut AppState, action_tx: &mpsc::UnboundedSender<Action>) {
    use crate::remote::Remote;

    if !state.system.remote_tried {
        state.system.remote_tried = true;
        let port = state.system.user_config.remote_port;
        if port != 0 {
            // Kept in the config so the phone's bookmark survives a restart.
            if state.system.user_config.remote_token.is_empty() {
                state.system.user_config.remote_token = crate::remote::new_token();
                if let Err(err) =
                    crate::config::user_config::save_user_config(&state.system.user_config)
                {
                    crate::logger::warn(format!("could not save the phone token: {err}"));
                }
            }
            let token = state.system.user_config.remote_token.clone();
            state.system.push = crate::remote::Push::load();
            let push_key = state.system.push.public_key();
            let (tx, rx) = mpsc::unbounded_channel();
            match Remote::start(
                port,
                token,
                push_key,
                state.system.remote_state.clone(),
                tx,
                action_tx.clone(),
            ) {
                Ok(remote) => {
                    crate::logger::info(format!("phone view on {}", remote.config.url()));
                    state.system.remote = Some(remote);
                    state.system.remote_commands = Some(rx);
                }
                // No tailnet, or the port is taken: the TUI carries on without
                // it rather than refusing to start.
                Err(err) => crate::logger::warn(format!("phone view unavailable: {err}")),
            }
        }
    }

    crate::remote::publish(state, &state.system.remote_state.clone());
    notify_phone(state, action_tx);
    control_tick(state);

    let mut pending = Vec::new();
    if let Some(rx) = state.system.remote_commands.as_mut() {
        while let Ok(command) = rx.try_recv() {
            pending.push(command);
        }
    }
    // The control socket speaks the same command vocabulary, so it lands in
    // the same place and takes the same path as a tap on the phone.
    if let Some(rx) = state.system.control_commands.as_mut() {
        while let Ok(command) = rx.try_recv() {
            pending.push(command);
        }
    }
    for command in pending {
        apply_remote(state, command, action_tx);
    }
}

/// Start the control socket once, then push whatever moved since last tick.
///
/// Runs after `publish`, so subscribers are told about the snapshot callers
/// can actually read — an event that arrives before the state backing it is
/// readable is worse than no event.
fn control_tick(state: &mut AppState) {
    if !state.system.control_tried {
        state.system.control_tried = true;
        let (tx, rx) = mpsc::unbounded_channel();
        match crate::control::start(state.system.remote_state.clone(), tx) {
            Ok(server) => {
                crate::logger::info(format!("control socket on {}", server.path().display()));
                state.system.control = Some(server);
                state.system.control_commands = Some(rx);
            }
            // Another workbench owns the socket, or the directory is not
            // writable. The TUI does not need it to work.
            Err(err) => crate::logger::warn(format!("control socket unavailable: {err}")),
        }
    }

    let Some(server) = state.system.control.as_ref() else {
        return;
    };
    let hub = server.hub.clone();
    let snapshot = match state.system.remote_state.lock() {
        Ok(snapshot) => snapshot.clone(),
        Err(_) => return,
    };
    crate::control::publish_events(&hub, &mut state.system.control_events, &snapshot);
}

/// How often to write a line describing the machine we are living on.
fn health_every() -> Duration {
    // Overridable so a test can watch several heartbeats without waiting
    // minutes; nothing sets it in normal use.
    std::env::var("WORKBENCH_HEALTH_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(60))
}

/// Leave a trail the process cannot leave for itself.
///
/// SIGKILL is uncatchable: no panic hook, no Drop, no last words. Three times
/// now workbench has simply ended, and the only honest answer to "why" was
/// that the log stopped. Nothing in-process can report its own killing — but
/// it can describe the conditions on the way there, so the minute before the
/// silence says whether memory was the story.
///
/// The PID is here for the same reason. Attribution lives in the system log
/// (`log show`), which identifies processes by number, and matching one up
/// after the fact previously meant guessing from open ports.
fn health_tick(state: &mut AppState) {
    let due = state
        .system
        .last_health_log
        .map(|at| at.elapsed() >= health_every())
        .unwrap_or(true);
    if !due {
        return;
    }
    state.system.last_health_log = Some(std::time::Instant::now());

    // Counted apart, because they cost differently: an agent is a
    // 300–600 MB claude or codex process, a terminal is a shell. "39 running
    // agents" once sent a memory investigation toward a fleet two-thirds of
    // which was fish prompts.
    let running: Vec<_> = state
        .data
        .sessions
        .values()
        .flatten()
        .filter(|s| s.status == crate::models::SessionStatus::Running)
        .collect();
    let agents = running.iter().filter(|s| s.agent_type.is_agent()).count();
    let terminals = running.len() - agents;
    let swap = match state.system.perf.system_swap() {
        Some((used, total)) if total > 0 => format!(
            "swap {:.1}/{:.1}GB ({:.0}%)",
            used as f64 / 1e9,
            total as f64 / 1e9,
            used as f64 / total as f64 * 100.0
        ),
        _ => "swap unknown".to_string(),
    };
    crate::logger::info(format!(
        "health: pid {} rss {:.0}MB, {agents} agents + {terminals} terminals, {swap}",
        std::process::id(),
        state.system.perf.memory_mb(),
    ));
}

/// Record how a manager thinks an objective could be checked.
///
/// Stored with `proposed` set, which is the whole point: it appears in the
/// pane for you to agree with, and until you do, nothing is held to it and no
/// work runs unattended against that objective.
fn apply_proposed_check(state: &mut AppState, manager: &str, objective: &str, command: &str) {
    let command = command.trim();
    if command.is_empty() {
        return;
    }
    let Some(session_id) = crate::remote::session_for(state, manager) else {
        return;
    };
    let Some(workspace_id) = state.workspace_id_for_session(session_id) else {
        return;
    };
    let Ok(objective_id) = uuid::Uuid::parse_str(objective) else {
        return;
    };
    let Some(workspace) = state
        .data
        .workspaces
        .iter_mut()
        .find(|ws| ws.id == workspace_id)
    else {
        return;
    };
    let Some(objective) = workspace
        .objectives
        .iter_mut()
        .find(|o| o.id == objective_id)
    else {
        return;
    };
    // A check you already approved is not something a manager may replace.
    if objective.done_when.as_ref().is_some_and(|c| !c.proposed) {
        return;
    }
    objective.done_when = Some(crate::models::Verification::proposed(command));
    crate::logger::info(format!("manager {manager} proposed a check: {command}"));
    super::handlers::save_state(state, "failed to save a proposed check");
}

/// Apply a manager's review outcome to the proposal it reviewed.
///
/// `accept` resolves the job — the job, never its objective. /// `request_changes` sends the findings back to the same agent, within the
/// bounded loop the original approval authorized. `needs_user` parks it for a
/// person, as does anything malformed enough to distrust.
fn apply_review(
    state: &mut AppState,
    manager: &str,
    proposal: &str,
    outcome: &str,
    findings: &str,
    action_tx: &mpsc::UnboundedSender<Action>,
) {
    let _ = action_tx;
    let found = state.data.workspaces.iter().find_map(|ws| {
        ws.proposals
            .iter()
            .find(|p| p.id.to_string() == *proposal)
            .map(|p| (ws.id, p.clone()))
    });
    let Some((workspace_id, snapshot)) = found else {
        crate::logger::warn(format!("review of an unknown proposal {proposal}"));
        return;
    };
    if !snapshot.manager.eq_ignore_ascii_case(manager) {
        crate::logger::warn(format!(
            "manager {manager} tried to review a proposal that belongs to {}",
            snapshot.manager
        ));
        return;
    }
    if !snapshot.awaiting_review() {
        crate::logger::warn(format!(
            "manager {manager} reviewed proposal {proposal}, which is not awaiting review"
        ));
        return;
    }

    fn stored<'a>(
        state: &'a mut AppState,
        workspace_id: uuid::Uuid,
        proposal: &str,
    ) -> Option<&'a mut crate::models::Proposal> {
        state
            .data
            .workspaces
            .iter_mut()
            .find(|ws| ws.id == workspace_id)
            .and_then(|ws| {
                ws.proposals
                    .iter_mut()
                    .find(|p| p.id.to_string() == proposal)
            })
    }

    match outcome {
        "accept" => {
            if let Some(p) = stored(state, workspace_id, proposal) {
                p.accept();
            }
            let line = format!(
                "manager {manager} accepted after round {}: {}",
                snapshot.review_rounds,
                first_line_of(&snapshot.instruction)
            );
            crate::logger::info(&line);
            state.ui.set_task_status(line);
        }
        "request_changes" => {
            if findings.trim().is_empty() {
                if let Some(p) = stored(state, workspace_id, proposal) {
                    p.needs_user("manager requested changes but named none".into());
                }
                crate::logger::warn("request_changes with no findings; parked for the user");
                super::handlers::save_state(state, "failed to save a review");
                return;
            }
            // Corrections go to the agent the approved proposal named — the
            // same authorization, one more lap, if any laps remain.
            let Some(agent) = snapshot.agent.clone() else {
                if let Some(p) = stored(state, workspace_id, proposal) {
                    p.needs_user("no agent to send corrections to".into());
                }
                super::handlers::save_state(state, "failed to save a review");
                return;
            };
            let Some(session_id) = crate::remote::session_for(state, &agent) else {
                if let Some(p) = stored(state, workspace_id, proposal) {
                    p.needs_user(format!("agent {agent} is no longer here"));
                }
                super::handlers::save_state(state, "failed to save a review");
                return;
            };
            if snapshot.review_rounds >= crate::models::MAX_REVIEW_ROUNDS {
                if let Some(p) = stored(state, workspace_id, proposal) {
                    p.needs_user(format!(
                        "correction rounds exhausted; last findings: {findings}"
                    ));
                }
                let line = format!(
                    "proposal used its {} correction rounds — over to you",
                    crate::models::MAX_REVIEW_ROUNDS
                );
                crate::logger::warn(&line);
                state.ui.set_task_status(line);
                super::handlers::save_state(state, "failed to save a review");
                return;
            }
            let text = format!(
                "Manager review (round {} of {}) requested changes on the work you just did for: {}

Findings to address, and nothing beyond them:
{}",
                snapshot.review_rounds + 1,
                crate::models::MAX_REVIEW_ROUNDS,
                first_line_of(&snapshot.instruction),
                findings.trim()
            );
            let Some(session) = state.get_session_mut(session_id) else {
                return;
            };
            let todo = session.todo_queue.add(text);
            if let Some(p) = stored(state, workspace_id, proposal) {
                p.request_changes(findings.trim().to_string(), todo);
            }
            let line = format!("manager {manager} sent corrections back to {agent}");
            crate::logger::info(&line);
            state.ui.set_task_status(line);
        }
        "needs_user" => {
            if let Some(p) = stored(state, workspace_id, proposal) {
                p.needs_user(if findings.trim().is_empty() {
                    "the manager asked for your eyes".into()
                } else {
                    findings.trim().to_string()
                });
            }
            let line = format!(
                "manager {manager} needs you on: {}",
                first_line_of(&snapshot.instruction)
            );
            crate::logger::warn(&line);
            state.ui.set_task_status(line);
        }
        other => {
            crate::logger::warn(format!("unknown review outcome {other:?}; ignored"));
            return;
        }
    }
    super::handlers::save_state(state, "failed to save a review");
}

fn first_line_of(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    let mut out: String = line.chars().take(70).collect();
    if line.chars().count() > 70 {
        out.push('…');
    }
    out
}

/// Record a manager's suggestion against the project it belongs to.
///
/// Recorded and nothing else. No queue is touched and no agent is told
/// anything — that step is the user's, and keeping it separate is what makes
/// a manager's reasoning reviewable before it can act on any of it.
fn apply_proposal(
    state: &mut AppState,
    manager: String,
    objective: Option<String>,
    agent: Option<String>,
    instruction: String,
    rationale: String,
) {
    let instruction = instruction.trim().to_string();
    if instruction.is_empty() {
        return;
    }
    // A manager only ever proposes within its own project, which is the one
    // its session lives in — not whichever happens to be selected.
    let Some(session_id) = crate::remote::session_for(state, &manager) else {
        crate::logger::warn(format!("proposal from unknown manager {manager}"));
        return;
    };
    let Some(workspace_id) = state.workspace_id_for_session(session_id) else {
        return;
    };
    let objective_id = objective.and_then(|id| uuid::Uuid::parse_str(&id).ok());
    let Some(workspace) = state
        .data
        .workspaces
        .iter_mut()
        .find(|ws| ws.id == workspace_id)
    else {
        return;
    };
    // An id for an objective that is not there would render as an orphan, so
    // it is dropped rather than kept: the proposal still stands on its own.
    let objective_id = objective_id.filter(|id| workspace.objectives.iter().any(|o| o.id == *id));

    let mut proposal = crate::models::Proposal::new(manager.clone(), instruction);
    proposal.objective_id = objective_id;
    proposal.agent = agent.filter(|a| !a.trim().is_empty());
    proposal.rationale = rationale.trim().to_string();
    workspace.proposals.push(proposal);

    crate::logger::info(format!("manager {manager} proposed work"));
    super::handlers::save_state(state, "failed to save a proposal");
}

/// How often to look for dev servers. Two `lsof` calls, so not every tick —
/// and a dev server you have just started is worth waiting a moment for.
const PORT_SCAN_EVERY: Duration = Duration::from_secs(5);

/// Look for listening dev servers, off the event loop.
fn scan_ports(state: &mut AppState, action_tx: &mpsc::UnboundedSender<Action>) {
    if !state.system.user_config.expose_dev_servers || state.system.port_scan_inflight {
        return;
    }
    // Everything the scan feeds is a phone-view feature — the dev-server
    // list and the tailnet forwarders — and both attribute servers to a
    // workspace. Without a running remote or any workspace to own a port,
    // the two lsof forks every 5 seconds buy nothing.
    if state.system.remote.is_none() || state.data.workspaces.is_empty() {
        return;
    }
    let due = state
        .system
        .last_port_scan
        .map(|at| at.elapsed() >= PORT_SCAN_EVERY)
        .unwrap_or(true);
    if !due {
        return;
    }
    state.system.last_port_scan = Some(std::time::Instant::now());
    state.system.port_scan_inflight = true;

    let tx = action_tx.clone();
    tokio::task::spawn_blocking(move || {
        dispatch_action(&tx, Action::PortsScanned(crate::ports::scan()));
    });
}

/// Splice each project's dev servers onto the tailnet address.
///
/// Only what runs inside a project, and only what binds loopback — a server
/// already on every interface is reachable as it is. Forwarders are additive:
/// see `SystemState::forwarded` for why none is ever taken down.
fn expose_project_servers(state: &mut AppState) {
    let Some(tailnet) = state.system.remote.as_ref().map(|r| r.config.addr.ip()) else {
        return;
    };
    let phone_port = state.system.user_config.remote_port;

    let mut roots: Vec<(std::path::PathBuf, uuid::Uuid)> = Vec::new();
    for workspace in &state.data.workspaces {
        roots.push((workspace.path.clone(), workspace.id));
        for session in state.data.sessions.get(&workspace.id).into_iter().flatten() {
            if let Some(worktree) = &session.worktree_path {
                roots.push((worktree.clone(), workspace.id));
            }
        }
    }

    // Reap forwarders that were shot. Port-freeing scripts kill by number
    // and hit ours too; the whole design is that the death lands on a
    // disposable child. Removing it here lets the loop below respawn it.
    state.system.forwarded.retain(|port, forwarder| {
        let standing = forwarder.alive();
        if !standing {
            crate::logger::info(format!(
                "forwarder for port {port} was killed; respawning if still wanted"
            ));
        }
        standing
    });

    let wanted: Vec<u16> = crate::ports::owned_by(&state.system.dev_servers, &roots)
        .into_iter()
        .filter(|(server, _)| server.loopback_only && server.port != phone_port)
        .map(|(server, _)| server.port)
        .collect();

    for port in wanted {
        if state.system.forwarded.contains_key(&port) {
            continue;
        }
        let bind = std::net::SocketAddr::new(tailnet, port);
        let upstream =
            std::net::SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), port);
        match crate::ports::expose(bind, upstream) {
            Ok(forwarder) => {
                crate::logger::info(format!(
                    "dev server on {port} is now reachable from the phone"
                ));
                state.system.forwarded.insert(port, forwarder);
            }
            // Almost always "address in use" — something else already has that
            // port on the tailnet address. A Thread entry is the "do not try
            // again every five seconds" marker: it always reports alive.
            Err(err) => {
                crate::logger::warn(format!("could not forward port {port}: {err}"));
                state
                    .system
                    .forwarded
                    .insert(port, crate::ports::Forwarder::unretryable(bind));
            }
        }
    }
}

/// How long an *inferred* spell of work has to last before its end counts as
/// a turn ending.
///
/// Only inferred spells need a floor. Where a hook is talking, the agent says
/// when it stopped and that is the end of it; where one is not — a provider
/// without hooks, or a report gone stale after thirty minutes — all there is
/// to go on is that bytes arrived recently, and a screen that merely repaints
/// satisfies that as well as a screen that is thinking.
///
/// That is not hypothetical. Anything making every pane redraw at once, a
/// terminal resize being the obvious one, did it to every agent at once, and
/// the log shows the result: sixteen agents "finishing" in the same instant,
/// then the same sixteen again five seconds later.
///
/// Such a blip lasts the output-timing window plus a tick, so about three
/// seconds. Ten is clear of that and under any turn worth being told about.
const TURN_FLOOR: Duration = Duration::from_secs(10);

/// Poke subscribed devices when an agent stops for you.
///
/// Read off the snapshot that was published a moment ago, so the phone is told
/// exactly what it would see if it looked.
///
/// One poke per stop, whatever the stop turns out to be. An agent that halts
/// at a permission prompt is often idle for a tick or two before the question
/// is parsed off its screen, and Claude's own "waiting for input" hook fires a
/// full minute after that — so a single stop used to arrive as "finished" and
/// then "needs you", twice, up to a minute apart. Idle and blocked are both
/// stopped; only the crossing into stopped is news, and the phone reads the
/// live state to decide which of the two words to use.
///
/// Returns what it told the phone, which is what the tests read. The state is
/// tracked even with nobody subscribed, so turning notifications on does not
/// immediately fire for everything already in progress.
fn notify_phone(state: &mut AppState, action_tx: &mpsc::UnboundedSender<Action>) -> Vec<String> {
    let statuses: Vec<(String, String)> = match state.system.remote_state.lock() {
        Ok(snapshot) => snapshot
            .agents
            .iter()
            .map(|agent| (agent.id.clone(), agent.status.clone()))
            .collect(),
        Err(_) => return Vec::new(),
    };

    // Idle and blocked are the same thing to a notification: the agent has
    // stopped and the next move is yours.
    let stopped = |status: &str| status == "idle" || status == "blocked";

    // The snapshot addresses agents by short id; asking whether a status was
    // reported or inferred needs the session behind it.
    let session_ids: HashMap<String, uuid::Uuid> = state
        .data
        .sessions
        .values()
        .flatten()
        .map(|session| (session.short_id(), session.id))
        .collect();

    let mut news: Vec<String> = Vec::new();
    for (id, status) in &statuses {
        let was = state.system.remote_seen.get(id).cloned();
        // The first sighting of an agent says nothing, so turning
        // notifications on does not fire for everything already running.
        if let Some(was) = was.as_deref() {
            if status == "working" && was != "working" {
                state
                    .system
                    .remote_working_since
                    .insert(id.clone(), Instant::now());
            }
            if stopped(status) && !stopped(was) {
                // Either the agent said it stopped, or it was working for long
                // enough that something other than a repaint was going on.
                let worked = state
                    .system
                    .remote_working_since
                    .get(id)
                    .map(Instant::elapsed)
                    .unwrap_or(Duration::ZERO);
                let said_so = session_ids
                    .get(id)
                    .is_some_and(|id| state.activity_is_reported(*id));
                if said_so || worked >= TURN_FLOOR {
                    state
                        .system
                        .remote_finished
                        .insert(id.clone(), chrono::Utc::now());
                    news.push(format!("{id} stopped"));
                }
                state.system.remote_working_since.remove(id);
            }
        }
        state.system.remote_seen.insert(id.clone(), status.clone());
    }

    // Agents that have gone away entirely.
    state
        .system
        .remote_seen
        .retain(|id, _| statuses.iter().any(|(seen, _)| seen == id));
    state
        .system
        .remote_working_since
        .retain(|id, _| statuses.iter().any(|(seen, _)| seen == id));

    if !news.is_empty() && !state.system.push.is_empty() {
        crate::logger::info(format!("telling the phone: {}", news.join(", ")));
        state.system.push.notify(action_tx);
    }
    news
}

/// Everything the phone can ask for, in terms of what the keyboard could do.
fn apply_remote(
    state: &mut AppState,
    command: crate::remote::RemoteCommand,
    action_tx: &mpsc::UnboundedSender<Action>,
) {
    use crate::remote::RemoteCommand;

    // A manager answering its review turn. Validated hard: only the manager
    // that proposed the job may close its review, and corrections can only
    // go to the agent the approved proposal already named.
    if let RemoteCommand::Review {
        manager,
        proposal,
        outcome,
        findings,
    } = &command
    {
        apply_review(state, manager, proposal, outcome, findings, action_tx);
        return;
    }

    // A decision names a proposal, not a session. Routed to the same core
    // the TUI's `a` and `x` use, found in whichever workspace holds it.
    if let RemoteCommand::Decide { proposal, approve } = &command {
        let found = state.data.workspaces.iter().find_map(|ws| {
            ws.proposals
                .iter()
                .find(|p| p.id.to_string() == *proposal)
                .map(|p| (ws.id, p.id))
        });
        let Some((workspace_id, proposal_id)) = found else {
            crate::logger::warn(format!("phone decided an unknown proposal {proposal}"));
            return;
        };
        // A proposal parked on the user is not pending, so `decide_proposal`
        // would answer "Already decided" and leave the row on the desk. The
        // decision this phase actually takes lives in `decide_needs_user`,
        // the same one the desk's own no reaches.
        if crate::app::handlers::tasks::is_parked_on_user(state, workspace_id, proposal_id) {
            match crate::app::handlers::tasks::decide_needs_user(
                state,
                workspace_id,
                proposal_id,
                *approve,
                action_tx,
            ) {
                Ok(outcome) => crate::logger::info(format!("phone decided a review: {outcome}")),
                Err(err) => crate::logger::warn(format!("phone's review decision failed: {err}")),
            }
            return;
        }
        // The phone has no picker, so an unassigned approval takes the best
        // default instead of failing: an idle directable agent already in
        // that project. If there is none at all, it stays pending with a log
        // line — spawning agents is a decision the desk's picker owns.
        if *approve {
            let unassigned = state
                .data
                .workspaces
                .iter()
                .find(|ws| ws.id == workspace_id)
                .and_then(|ws| ws.proposals.iter().find(|p| p.id == proposal_id))
                .is_some_and(|p| p.agent.is_none());
            if unassigned {
                let pick = state
                    .data
                    .sessions
                    .get(&workspace_id)
                    .into_iter()
                    .flatten()
                    .filter(|s| {
                        s.status == crate::models::SessionStatus::Running
                            && s.agent_type.is_directable()
                    })
                    .map(|s| (s.id, state.activity(s.id).is_free()))
                    .max_by_key(|(_, free)| *free)
                    .map(|(id, _)| id);
                match pick {
                    Some(id) => {
                        let short = crate::models::Session::short_id_of(id);
                        crate::logger::info(format!(
                            "phone approval auto-assigned {short} to an unassigned proposal"
                        ));
                        if let Some(stored) = state
                            .data
                            .workspaces
                            .iter_mut()
                            .find(|ws| ws.id == workspace_id)
                            .and_then(|ws| {
                                ws.proposals.iter_mut().find(|p| p.id == proposal_id)
                            })
                        {
                            stored.agent = Some(short);
                        }
                    }
                    None => {
                        crate::logger::warn(
                            "phone approved an unassigned proposal in a project with no                              agents; left pending — assign one from the desk",
                        );
                        return;
                    }
                }
            }
        }
        match crate::app::handlers::tasks::decide_proposal(
            state,
            workspace_id,
            proposal_id,
            *approve,
            action_tx,
        ) {
            Ok(outcome) => crate::logger::info(format!("phone decided a proposal: {outcome}")),
            Err(err) => crate::logger::warn(format!("phone's decision failed: {err}")),
        }
        return;
    }

    // A check belongs to the objective it would prove, not to an agent.
    // Routed to the same helper the desk's `a`/`x` reaches, so approving from
    // the phone and approving at the desk are one act with one code path.
    if let RemoteCommand::DecideCheck { objective, approve } = &command {
        let found = state.data.workspaces.iter().find_map(|ws| {
            ws.objectives
                .iter()
                .find(|o| o.id.to_string() == *objective)
                .map(|o| (ws.id, o.id))
        });
        let Some((workspace_id, objective_id)) = found else {
            crate::logger::warn(format!("phone decided an unknown check on {objective}"));
            return;
        };
        crate::app::handlers::tasks::decide_check(state, workspace_id, objective_id, *approve);
        crate::logger::info(format!(
            "phone {} a proposed check",
            if *approve { "approved" } else { "dropped" }
        ));
        return;
    }

    // Re-arming names a proposal. Same helper as the desk's yes on a
    // "needs you" row: a fresh set of rounds, the findings carried across.
    if let RemoteCommand::RearmReview { proposal } = &command {
        let found = state.data.workspaces.iter().find_map(|ws| {
            ws.proposals
                .iter()
                .find(|p| p.id.to_string() == *proposal)
                .map(|p| (ws.id, p.id))
        });
        let Some((workspace_id, proposal_id)) = found else {
            crate::logger::warn(format!("phone re-armed an unknown proposal {proposal}"));
            return;
        };
        match crate::app::handlers::tasks::rearm_review(
            state,
            workspace_id,
            proposal_id,
            action_tx,
        ) {
            Ok(outcome) => crate::logger::info(format!("phone re-armed a review: {outcome}")),
            Err(err) => crate::logger::warn(format!("phone's re-arm was refused: {err}")),
        }
        return;
    }

    // A subscription names a device, not a session.
    if let RemoteCommand::Subscribe { endpoint } = &command {
        if state.system.push.subscribe(endpoint.clone()) {
            crate::logger::info("a device asked to be told when an agent needs you".to_string());
            if let Err(err) = state.system.push.save() {
                crate::logger::warn(format!("could not store the subscription: {err}"));
            }
        }
        return;
    }

    // Creating an agent names a project, not a session.
    if let RemoteCommand::NewAgent { project, provider } = &command {
        let Ok(workspace_id) = project.parse::<uuid::Uuid>() else {
            return;
        };
        let agent_type = match provider.as_str() {
            "codex" => crate::models::AgentType::Codex,
            "claude" => crate::models::AgentType::Claude,
            other => {
                crate::logger::warn(format!("phone asked for an unknown agent: {other}"));
                return;
            }
        };
        if state.get_workspace(workspace_id).is_none() {
            crate::logger::warn(format!(
                "phone asked for an agent in unknown project {project}"
            ));
            return;
        }
        crate::logger::info(format!("phone started a {provider} in {project}"));
        // Permissions stay on: a prompt is answerable from the phone now, so
        // there is no reason to hand a remote-started agent a free pass.
        dispatch_action(
            action_tx,
            Action::CreateSessionIn(workspace_id, agent_type, false, false),
        );
        return;
    }

    if let RemoteCommand::ProposeCheck {
        manager,
        objective,
        command,
    } = &command
    {
        apply_proposed_check(state, manager, objective, command);
        return;
    }

    if let RemoteCommand::Propose {
        manager,
        objective,
        agent,
        instruction,
        rationale,
    } = command
    {
        apply_proposal(state, manager, objective, agent, instruction, rationale);
        return;
    }

    let agent = match &command {
        RemoteCommand::Todo { agent, .. }
        | RemoteCommand::Reply { agent, .. }
        | RemoteCommand::Answer { agent, .. }
        | RemoteCommand::Focus { agent } => agent.clone(),
        // Handled above.
        RemoteCommand::NewAgent { .. } | RemoteCommand::Subscribe { .. } => return,
        // Applied above; they name a project, a proposal or an objective,
        // not an agent.
        RemoteCommand::Propose { .. }
        | RemoteCommand::ProposeCheck { .. }
        | RemoteCommand::Decide { .. }
        | RemoteCommand::DecideCheck { .. }
        | RemoteCommand::RearmReview { .. }
        | RemoteCommand::Review { .. } => return,
    };
    let Some(session_id) = crate::remote::session_for(state, &agent) else {
        crate::logger::warn(format!("phone asked for unknown agent {agent}"));
        return;
    };

    match command {
        // Applied before the agent lookup above; unreachable here.
        RemoteCommand::Propose { .. }
        | RemoteCommand::ProposeCheck { .. }
        | RemoteCommand::Decide { .. }
        | RemoteCommand::DecideCheck { .. }
        | RemoteCommand::RearmReview { .. }
        | RemoteCommand::Review { .. } => {}
        RemoteCommand::Todo { text, .. } => {
            if let Some(session) = state.get_session_mut(session_id) {
                session.todo_queue.add(text.clone());
            }
            crate::logger::info(format!("phone queued for {agent}: {text}"));
            super::handlers::save_state(state, "failed to save a queued todo");
        }
        RemoteCommand::Reply { text, .. } => {
            let running = state
                .get_session(session_id)
                .map(|s| s.status == crate::models::SessionStatus::Running)
                .unwrap_or(false);
            if running {
                crate::logger::info(format!("phone replied to {agent}: {text}"));
                super::agent_input::submit_text(action_tx, session_id, &text);
            } else {
                // Talking to a stopped agent means you want it back. Start it,
                // and let the queue deliver as soon as it is ready — the agent
                // takes seconds to boot, and its own hooks say when it is.
                crate::logger::info(format!("phone woke {agent} to say: {text}"));
                if let Some(session) = state.get_session_mut(session_id) {
                    session.todo_queue.add_next(text);
                }
                dispatch_action(action_tx, Action::RestartSession(session_id));
                super::handlers::save_state(state, "failed to save a woken message");
            }
        }
        // Both providers take a bare digit for a numbered choice — verified by
        // driving each in a pty until it blocked, then answering. That beats
        // the Enter this used to send, which took whichever option happened to
        // be highlighted.
        RemoteCommand::Answer { key, .. } => {
            let offered = crate::remote::prompt_on_screen(state, session_id);
            let bytes = match (key.as_str(), &offered) {
                ("esc", _) => Some(vec![0x1b]),
                (key, Some(prompt)) if prompt.options.iter().any(|o| o.key == key) => {
                    Some(key.as_bytes().to_vec())
                }
                // The prompt was answered at the desk while the tap was in
                // flight. Typing the digit now would put it in the composer.
                _ => None,
            };
            match bytes {
                Some(bytes) => {
                    crate::logger::info(format!("phone answered {agent} with {key}"));
                    dispatch_action(action_tx, Action::SendInput(session_id, bytes));
                }
                None => crate::logger::info(format!(
                    "phone answered {agent} with {key}, but that choice is no longer on screen"
                )),
            }
        }
        RemoteCommand::Focus { .. } => {
            state.system.remote_focus = Some(session_id);
            // A different conversation means the cached one is of no use.
            state.system.remote_thread = None;
        }
        // Handled before the session lookup, which they do not need.
        RemoteCommand::NewAgent { .. } | RemoteCommand::Subscribe { .. } => {}
    }
}

/// Which sessions a refresh pass reads, and in what order.
struct TaskRefreshPlan {
    /// Where to look, for the sessions worth looking at.
    sources: HashMap<uuid::Uuid, crate::agent_tasks::TaskSource>,
    /// Every agent session that still exists, running or not — trackers for
    /// anything else are dropped.
    known: std::collections::HashSet<uuid::Uuid>,
    /// `sources` keys in spawn order.
    order: Vec<uuid::Uuid>,
}

/// Only *running* agents are read. A stopped session's list stays on screen
/// (its tracker is kept), but it neither rescans the stores every second nor
/// claims a conversation the live agent should get.
///
/// The order matters: a codex rollout is recognised as "the first one opened
/// after this process started", so the session that spawned first has to pick
/// first — iterating a HashMap would hand two agents in one project each
/// other's conversation.
fn plan_task_refresh(state: &AppState) -> TaskRefreshPlan {
    use crate::agent_tasks::{Provider, TaskSource};

    let mut sources: HashMap<uuid::Uuid, TaskSource> = HashMap::new();
    let mut known: std::collections::HashSet<uuid::Uuid> = std::collections::HashSet::new();

    for workspace in &state.data.workspaces {
        let Some(sessions) = state.data.sessions.get(&workspace.id) else {
            continue;
        };
        for session in sessions {
            let Some(provider) = Provider::for_agent(&session.agent_type) else {
                continue;
            };
            known.insert(session.id);
            if session.status != crate::models::SessionStatus::Running {
                continue;
            }
            let cwd = session
                .worktree_path
                .clone()
                .unwrap_or_else(|| workspace.path.clone());
            sources.insert(
                session.id,
                TaskSource {
                    provider,
                    session_uuid: session.id.to_string(),
                    cwd,
                    started_at: session.started_at,
                    conversation: session.provider_session_id.clone(),
                    spawned_at: state.system.session_spawned_at.get(&session.id).copied(),
                    // Not gated on freshness: the last file the agent named is
                    // the last file it wrote, however long ago that was.
                    reported: state
                        .system
                        .agent_status
                        .get(&session.id)
                        .and_then(|status| status.transcript.as_deref())
                        .map(std::path::PathBuf::from),
                },
            );
        }
    }

    let mut order: Vec<uuid::Uuid> = sources.keys().copied().collect();
    order.sort_by_key(|id| (sources[id].spawned_at, *id));

    TaskRefreshPlan {
        sources,
        known,
        order,
    }
}

/// Re-read every running agent's task list off the UI thread.
///
/// Trackers are cloned out, refreshed, and sent back whole: they carry their
/// own file offsets, so a pass only parses bytes appended since the last one.
/// Reload durable scrollback for any agent whose session log has grown (or
/// whose pane changed width, since the log text is wrapped to fit).
fn refresh_scrollback(state: &mut AppState, action_tx: &mpsc::UnboundedSender<Action>) {
    use crate::scrollback::{log_path, LogFormat};

    if state.system.scrollback_inflight
        || state.system.last_scrollback_refresh.elapsed() < SCROLLBACK_REFRESH_INTERVAL
    {
        return;
    }
    state.system.last_scrollback_refresh = std::time::Instant::now();

    let cols = state.output_pane_cols();
    let theme_mode = state.ui.theme_mode;
    let live: std::collections::HashSet<uuid::Uuid> = state
        .data
        .sessions
        .values()
        .flatten()
        .map(|session| session.id)
        .collect();
    state
        .system
        .scrollback_state
        .retain(|session_id, _| live.contains(session_id));

    // A session whose parsed history is out of date; one per tick keeps a big
    // log from stalling the others behind it.
    //
    // The one you are looking at goes first. One per tick at this interval
    // means a session waits behind every other stale session before its turn,
    // so with several agents all answering at once the history under your
    // scroll could be many seconds behind the screen — and the gap shows,
    // because anything that scrolled off the live viewport in that window is
    // in neither the last parse nor the current frame. Whichever session is
    // open is the only one whose staleness anybody can see.
    let open = state.active_session_id();
    let stale: Vec<_> = state
        .data
        .sessions
        .values()
        .flatten()
        .filter_map(|session| {
            let format = LogFormat::for_agent(&session.agent_type)?;
            // The task tracker already resolves which log belongs to which
            // session (it handles claiming and spawn order); fall back to a
            // lookup by conversation id only if it has not got there yet.
            let path = match session.journal_path.clone() {
                Some(path) => path,
                None => log_path(format, session.provider_session_id.as_deref()?)?,
            };
            let size = std::fs::metadata(&path).ok()?.len();
            let current = state.system.scrollback_state.get(&session.id);
            (current != Some(&(size, cols, theme_mode))).then_some((session.id, format, path, size))
        })
        .collect();

    let Some((session_id, format, path, size)) = open
        .and_then(|open| stale.iter().find(|(id, ..)| *id == open))
        .or_else(|| stale.first())
        .cloned()
    else {
        return;
    };

    // `theme::current()` is thread-local, so it has to be read here on the
    // event loop — a worker thread would silently get the dark default.
    let theme = crate::theme::current();
    state.system.scrollback_inflight = true;
    let tx = action_tx.clone();
    tokio::task::spawn_blocking(move || {
        let lines = crate::scrollback::history(format, &path, cols, theme);
        dispatch_action(
            &tx,
            Action::ScrollbackLoaded {
                session_id,
                lines,
                log_size: size,
                cols,
            },
        );
    });
}

fn refresh_agent_tasks(state: &mut AppState, action_tx: &mpsc::UnboundedSender<Action>) {
    use crate::agent_tasks::TaskTracker;

    if state.system.task_refresh_inflight
        || state.system.last_task_refresh.elapsed() < TASK_REFRESH_INTERVAL
    {
        return;
    }
    state.system.last_task_refresh = std::time::Instant::now();

    let TaskRefreshPlan {
        sources,
        known,
        order,
    } = plan_task_refresh(state);

    // Drop trackers for sessions that no longer exist.
    state
        .system
        .agent_tasks
        .retain(|session_id, _| known.contains(session_id));
    if sources.is_empty() {
        return;
    }

    let mut trackers = state.system.agent_tasks.clone();
    for (session_id, source) in &sources {
        trackers
            .entry(*session_id)
            .or_insert_with(|| TaskTracker::new(source.provider));
    }

    state.system.task_refresh_inflight = true;
    let tx = action_tx.clone();
    tokio::task::spawn_blocking(move || {
        // Logs already spoken for — including stopped sessions', which still
        // own their conversation.
        let mut claimed: std::collections::HashSet<String> = trackers
            .values()
            .filter_map(|tracker| tracker.source().map(|source| source.key()))
            .collect();

        for session_id in order {
            let (Some(tracker), Some(source)) =
                (trackers.get_mut(&session_id), sources.get(&session_id))
            else {
                continue;
            };
            tracker.refresh(source, &claimed);
            if let Some(source) = tracker.source() {
                claimed.insert(source.key());
            }
        }
        dispatch_action(&tx, Action::AgentTasksRefreshed(trackers));
    });
}

#[cfg(test)]
mod tests {
    /// `notify_phone` with a throwaway action channel: these tests are about
    /// what counts as news, not about the push delivery behind it.
    fn poke(state: &mut super::AppState) -> Vec<String> {
        let (tx, _rx) = super::mpsc::unbounded_channel();
        super::notify_phone(state, &tx)
    }

    use super::{apply_remote, apply_review, notify_phone, plan_task_refresh, process_action};
    use crate::models::{AgentType, Session, SessionStatus, Workspace};
    use chrono::{Duration as ChronoDuration, Utc};

    /// Add an agent session to `state`, optionally already spawned.
    fn add_agent(
        state: &mut AppState,
        workspace_id: uuid::Uuid,
        status: SessionStatus,
        spawned_ago_secs: Option<i64>,
    ) -> uuid::Uuid {
        let mut session = Session::new(workspace_id, AgentType::Codex, false);
        session.status = status;
        let id = session.id;
        state
            .data
            .sessions
            .entry(workspace_id)
            .or_default()
            .push(session);
        if let Some(secs) = spawned_ago_secs {
            state
                .system
                .session_spawned_at
                .insert(id, Utc::now() - ChronoDuration::seconds(secs));
        }
        id
    }

    /// The failure this fixes: notifications only ever fired on the edge into
    /// "blocked", but a ⚡ session skips permission prompts and so is almost
    /// never blocked — meaning nothing ever fired. Finishing is the event you
    /// left the desk for.
    #[test]
    fn the_phone_is_told_every_time_a_turn_finishes() {
        let mut state = AppState::default();
        let workspace = Workspace::new("zeta".into(), std::path::PathBuf::from("/tmp/z"));
        let workspace_id = workspace.id;
        state.data.workspaces.push(workspace);
        let agent = add_agent(&mut state, workspace_id, SessionStatus::Running, Some(30));
        let short = crate::models::Session::short_id_of(agent);

        let set = |state: &mut AppState, activity| {
            state.system.agent_status.insert(
                agent,
                crate::agent_status::AgentStatus {
                    activity,
                    reason: String::new(),
                    at: Utc::now(),
                    event: "Stop".into(),
                    transcript: None,
                    model: None,
                },
            );
            crate::remote::publish(state, &state.system.remote_state.clone());
        };

        set(&mut state, crate::agent_status::Activity::Working);
        assert!(poke(&mut state).is_empty(), "starting is not news");

        set(&mut state, crate::agent_status::Activity::Idle);
        assert_eq!(
            poke(&mut state),
            vec![format!("{short} stopped")],
            "every finished turn is worth saying, however short"
        );

        // And again next time round, not only the first.
        set(&mut state, crate::agent_status::Activity::Working);
        poke(&mut state);
        set(&mut state, crate::agent_status::Activity::Idle);
        assert_eq!(poke(&mut state), vec![format!("{short} stopped")]);

        // And the phone can tell which kind of news it was.
        let shared = state.system.remote_state.clone();
        crate::remote::publish(&mut state, &shared);
        let snapshot = shared.lock().unwrap();
        let view = snapshot.agents.iter().find(|a| a.id == short).unwrap();
        assert!(view.finished_ago.is_some_and(|ago| ago < 5));

        // Staying idle is not news again.
        drop(snapshot);
        assert!(poke(&mut state).is_empty());
    }

    /// A blocked agent stays blocked until you answer it. The phone should
    /// hear about that once, on the edge — not once a second for as long as
    /// it takes you to get to your desk.
    #[test]
    fn the_phone_is_told_when_an_agent_becomes_blocked_and_not_again() {
        let mut state = AppState::default();
        let workspace = Workspace::new("zeta".into(), std::path::PathBuf::from("/tmp/z"));
        let workspace_id = workspace.id;
        state.data.workspaces.push(workspace);
        let agent = add_agent(&mut state, workspace_id, SessionStatus::Running, Some(30));
        let short = crate::models::Session::short_id_of(agent);

        let block = |state: &mut AppState, blocked: bool| {
            state.system.agent_status.insert(
                agent,
                crate::agent_status::AgentStatus {
                    activity: if blocked {
                        crate::agent_status::Activity::NeedsAttention(
                            crate::agent_status::Attention::Permission,
                        )
                    } else {
                        crate::agent_status::Activity::Working
                    },
                    reason: "wants to run shell".into(),
                    at: Utc::now(),
                    event: "PermissionRequest".into(),
                    transcript: None,
                    model: None,
                },
            );
            crate::remote::publish(state, &state.system.remote_state.clone());
        };

        block(&mut state, false);
        assert!(poke(&mut state).is_empty(), "working is not news");

        block(&mut state, true);
        assert_eq!(poke(&mut state), vec![format!("{short} stopped")]);
        block(&mut state, true);
        assert!(
            poke(&mut state).is_empty(),
            "still blocked is not a new thing to say"
        );

        // Answered, then blocked again: that is news a second time.
        block(&mut state, false);
        assert!(poke(&mut state).is_empty());
        block(&mut state, true);
        assert_eq!(poke(&mut state), vec![format!("{short} stopped")]);
    }

    /// The bug: one stop arriving as two notifications.
    ///
    /// An agent that halts at a permission prompt is idle for a tick or two
    /// before the question can be parsed off its screen, and Claude's own
    /// "waiting for input" hook fires a full minute later. Both used to be
    /// news, so a single stop buzzed the phone as "finished" and then again as
    /// "needs you" — which is what the log showed, five to sixty seconds apart.
    #[test]
    fn one_stop_is_one_notification_however_it_settles() {
        let mut state = AppState::default();
        let workspace = Workspace::new("zeta".into(), std::path::PathBuf::from("/tmp/z"));
        let workspace_id = workspace.id;
        state.data.workspaces.push(workspace);
        let agent = add_agent(&mut state, workspace_id, SessionStatus::Running, Some(30));
        let short = crate::models::Session::short_id_of(agent);

        let set = |state: &mut AppState, activity| {
            state.system.agent_status.insert(
                agent,
                crate::agent_status::AgentStatus {
                    activity,
                    reason: String::new(),
                    at: Utc::now(),
                    event: "Stop".into(),
                    transcript: None,
                    model: None,
                },
            );
            crate::remote::publish(state, &state.system.remote_state.clone());
        };

        set(&mut state, crate::agent_status::Activity::Working);
        assert!(poke(&mut state).is_empty());

        // It stops. One poke.
        set(&mut state, crate::agent_status::Activity::Idle);
        assert_eq!(poke(&mut state), vec![format!("{short} stopped")]);

        // The question on its screen is read a tick later, and a minute after
        // that the harness says it is waiting. Same stop; nothing new to say.
        set(
            &mut state,
            crate::agent_status::Activity::NeedsAttention(
                crate::agent_status::Attention::Permission,
            ),
        );
        assert!(
            poke(&mut state).is_empty(),
            "the phone was already told this agent stopped"
        );

        // Answer it, and the next stop is news again.
        set(&mut state, crate::agent_status::Activity::Working);
        assert!(poke(&mut state).is_empty());
        set(&mut state, crate::agent_status::Activity::Idle);
        assert_eq!(poke(&mut state), vec![format!("{short} stopped")]);
    }

    /// The other half of the bug, and the louder one: an idle agent whose
    /// screen merely repaints looks like it worked for two seconds and then
    /// finished. A terminal resize does that to every agent at once — the log
    /// showed sixteen "finishing" in the same instant, then again five seconds
    /// later.
    ///
    /// Nothing reported it, and nothing worked for long enough to have been a
    /// turn, so there is nothing to say.
    #[test]
    fn a_screen_that_merely_repainted_did_not_finish_a_turn() {
        let mut state = AppState::default();
        let workspace = Workspace::new("zeta".into(), std::path::PathBuf::from("/tmp/z"));
        let workspace_id = workspace.id;
        state.data.workspaces.push(workspace);
        let agent = add_agent(&mut state, workspace_id, SessionStatus::Running, Some(30));

        // No hook has spoken for over half an hour, so activity is inferred
        // from output timing alone — which is the only way this happens.
        state.system.agent_status.insert(
            agent,
            crate::agent_status::AgentStatus {
                activity: crate::agent_status::Activity::Idle,
                reason: String::new(),
                at: Utc::now() - ChronoDuration::minutes(45),
                event: "Stop".into(),
                transcript: None,
                model: None,
            },
        );
        let publish = |state: &mut AppState| {
            crate::remote::publish(state, &state.system.remote_state.clone());
        };

        publish(&mut state);
        assert!(poke(&mut state).is_empty());

        // A repaint: output lands, so it reads as working for a moment.
        state
            .data
            .last_activity
            .insert(agent, std::time::Instant::now());
        publish(&mut state);
        assert!(poke(&mut state).is_empty(), "starting is not news");

        // The window passes with no more output and it reads as idle again.
        state.data.last_activity.insert(
            agent,
            std::time::Instant::now() - std::time::Duration::from_secs(5),
        );
        publish(&mut state);
        assert!(
            poke(&mut state).is_empty(),
            "two seconds of redraw is not a turn"
        );
    }

    fn state_with_workspace() -> (AppState, uuid::Uuid) {
        let mut state = AppState::default();
        let workspace = Workspace::new("w".into(), std::path::PathBuf::from("/tmp/w"));
        let id = workspace.id;
        state.data.workspaces.push(workspace);
        (state, id)
    }

    /// Talking to an agent you stopped means you want it back — the message
    /// should wake it, not vanish.
    #[test]
    fn messaging_a_stopped_agent_starts_it_and_keeps_the_message() {
        let (mut state, workspace_id) = state_with_workspace();
        let id = add_agent(&mut state, workspace_id, SessionStatus::Stopped, None);
        let short = state.get_session(id).unwrap().short_id();
        let (tx, mut rx) = mpsc::unbounded_channel();

        apply_remote(
            &mut state,
            crate::remote::RemoteCommand::Reply {
                agent: short,
                text: "pick this back up please".into(),
            },
            &tx,
        );

        // The message waits in the queue, to be delivered when it is ready.
        let queue = &state.get_session(id).unwrap().todo_queue;
        assert_eq!(queue.items.len(), 1);
        assert_eq!(queue.items[0].text, "pick this back up please");

        // And the session was asked to start.
        let restarted = std::iter::from_fn(|| rx.try_recv().ok())
            .any(|action| matches!(action, Action::RestartSession(target) if target == id));
        assert!(restarted, "a stopped agent must be started, not written to");
    }

    #[test]
    fn the_phone_can_start_an_agent_in_a_named_project() {
        let (mut state, workspace_id) = state_with_workspace();
        let (tx, mut rx) = mpsc::unbounded_channel();

        apply_remote(
            &mut state,
            crate::remote::RemoteCommand::NewAgent {
                project: workspace_id.to_string(),
                provider: "codex".into(),
            },
            &tx,
        );

        match rx.try_recv() {
            Ok(Action::CreateSessionIn(target, agent, dangerous, worktree)) => {
                assert_eq!(target, workspace_id);
                assert_eq!(agent, AgentType::Codex);
                // A prompt is answerable from the phone, so a remotely started
                // agent keeps its permission gates.
                assert!(!dangerous);
                assert!(!worktree);
            }
            other => panic!("expected a session to be created, got {other:?}"),
        }
    }

    #[test]
    fn a_nonsense_project_or_provider_starts_nothing() {
        let (mut state, workspace_id) = state_with_workspace();
        let (tx, mut rx) = mpsc::unbounded_channel();

        for command in [
            crate::remote::RemoteCommand::NewAgent {
                project: workspace_id.to_string(),
                provider: "definitely-not-an-agent".into(),
            },
            crate::remote::RemoteCommand::NewAgent {
                project: uuid::Uuid::new_v4().to_string(),
                provider: "claude".into(),
            },
            crate::remote::RemoteCommand::NewAgent {
                project: "not-a-uuid".into(),
                provider: "claude".into(),
            },
        ] {
            apply_remote(&mut state, command, &tx);
        }
        assert!(rx.try_recv().is_err(), "nothing should have been started");
    }

    /// A running agent is typed to directly — no queue, no delay.
    #[test]
    fn messaging_a_running_agent_goes_straight_to_its_terminal() {
        let (mut state, workspace_id) = state_with_workspace();
        let id = add_agent(&mut state, workspace_id, SessionStatus::Running, Some(5));
        let short = state.get_session(id).unwrap().short_id();
        let (tx, mut rx) = mpsc::unbounded_channel();

        apply_remote(
            &mut state,
            crate::remote::RemoteCommand::Reply {
                agent: short,
                text: "hello".into(),
            },
            &tx,
        );

        assert!(state.get_session(id).unwrap().todo_queue.is_empty());
        assert!(matches!(
            rx.try_recv(),
            // Bracketed, so a message that starts with one of the agent's own
            // hotkeys still reaches the composer (see `agent_input`).
            Ok(Action::SendInput(target, bytes))
                if target == id && bytes == b"\x1b[200~hello\x1b[201~".to_vec()
        ));
    }

    #[test]
    fn only_running_agents_are_read_but_stopped_ones_keep_their_tracker() {
        let (mut state, workspace_id) = state_with_workspace();
        let running = add_agent(&mut state, workspace_id, SessionStatus::Running, Some(10));
        let stopped = add_agent(&mut state, workspace_id, SessionStatus::Stopped, None);
        // Terminals have no task list at all.
        let mut terminal = Session::new(workspace_id, AgentType::Terminal("sh".into()), false);
        terminal.status = SessionStatus::Running;
        let terminal_id = terminal.id;
        state
            .data
            .sessions
            .get_mut(&workspace_id)
            .unwrap()
            .push(terminal);

        let plan = plan_task_refresh(&state);

        assert!(plan.sources.contains_key(&running));
        assert!(
            !plan.sources.contains_key(&stopped),
            "a stopped agent must not rescan the stores or claim a conversation"
        );
        assert!(
            plan.known.contains(&stopped),
            "its task list should stay on screen, so keep its tracker"
        );
        assert!(!plan.known.contains(&terminal_id));
    }

    #[test]
    fn sessions_are_resolved_in_spawn_order() {
        let (mut state, workspace_id) = state_with_workspace();
        // Deliberately added newest-first; the plan must still put the
        // earliest spawn first, whatever order the map iterates in.
        let newest = add_agent(&mut state, workspace_id, SessionStatus::Running, Some(5));
        let oldest = add_agent(&mut state, workspace_id, SessionStatus::Running, Some(500));
        let middle = add_agent(&mut state, workspace_id, SessionStatus::Running, Some(50));

        let plan = plan_task_refresh(&state);

        assert_eq!(plan.order, vec![oldest, middle, newest]);
    }

    #[test]
    fn a_session_that_never_spawned_sorts_before_ones_that_did() {
        let (mut state, workspace_id) = state_with_workspace();
        let spawned = add_agent(&mut state, workspace_id, SessionStatus::Running, Some(30));
        let unspawned = add_agent(&mut state, workspace_id, SessionStatus::Running, None);

        let plan = plan_task_refresh(&state);

        // `None` first is deliberate: a session with no spawn anchor falls back
        // to the loose "newest log for this cwd" rule, so it must not get to
        // pick after an anchored session has already taken its own.
        assert_eq!(plan.order, vec![unspawned, spawned]);
    }

    use crate::app::{Action, AppState, UtilityContentPayload};
    use crate::pty::PtyManager;
    use ratatui::style::Color;
    use tokio::sync::mpsc;

    #[test]
    fn utility_content_loaded_ignores_stale_request() {
        let mut state = AppState::default();
        state.ui.utility_request_id = 2;
        state.ui.utility_content = vec!["old".to_string()];
        state.ui.pie_chart_data = vec![("old".to_string(), 1.0, Color::Blue)];
        state.ui.show_calendar = true;

        let payload = UtilityContentPayload {
            request_id: 1,
            content: vec!["new".to_string()],
            pie_chart_data: vec![("new".to_string(), 2.0, Color::Red)],
            show_calendar: false,
        };

        let pty_manager = PtyManager::new();
        let (action_tx, _) = mpsc::unbounded_channel();
        let (pty_tx, _) = mpsc::channel(1);

        process_action(
            &mut state,
            Action::UtilityContentLoaded(payload),
            &pty_manager,
            &action_tx,
            &pty_tx,
        )
        .unwrap();

        assert_eq!(state.ui.utility_content, vec!["old".to_string()]);
        assert_eq!(state.ui.pie_chart_data.len(), 1);
        assert_eq!(state.ui.pie_chart_data[0].0, "old");
        assert!(state.ui.show_calendar);
    }

    #[test]
    fn utility_content_loaded_updates_current_request() {
        let mut state = AppState::default();
        state.ui.utility_request_id = 3;
        state.ui.utility_content = vec!["old".to_string()];
        state.ui.pie_chart_data = vec![("old".to_string(), 1.0, Color::Blue)];
        state.ui.show_calendar = false;

        let payload = UtilityContentPayload {
            request_id: 3,
            content: vec!["new".to_string()],
            pie_chart_data: vec![("new".to_string(), 2.0, Color::Red)],
            show_calendar: true,
        };

        let pty_manager = PtyManager::new();
        let (action_tx, _) = mpsc::unbounded_channel();
        let (pty_tx, _) = mpsc::channel(1);

        process_action(
            &mut state,
            Action::UtilityContentLoaded(payload),
            &pty_manager,
            &action_tx,
            &pty_tx,
        )
        .unwrap();

        assert_eq!(state.ui.utility_content, vec!["new".to_string()]);
        assert_eq!(state.ui.pie_chart_data.len(), 1);
        assert_eq!(state.ui.pie_chart_data[0].0, "new");
        assert!(state.ui.show_calendar);
    }
    /// A world with one manager, one worker, and one approved proposal that
    /// has just finished — the moment the review loop begins.
    fn review_world() -> (AppState, uuid::Uuid, uuid::Uuid, uuid::Uuid, uuid::Uuid) {
        let mut state = AppState::default();
        let workspace = Workspace::new("w".into(), std::path::PathBuf::from("/tmp/w"));
        let workspace_id = workspace.id;
        state.data.workspaces.push(workspace);

        let mut worker = Session::new(workspace_id, AgentType::Claude, false);
        worker.status = SessionStatus::Running;
        let worker_id = worker.id;
        let mut manager = Session::new(workspace_id, AgentType::Claude.as_manager(), false);
        manager.status = SessionStatus::Running;
        let manager_id = manager.id;

        let mut proposal = crate::models::Proposal::new(
            Session::short_id_of(manager_id),
            "tighten the recorder supervision",
        );
        proposal.agent = Some(Session::short_id_of(worker_id));
        let todo = worker.todo_queue.add("tighten the recorder supervision");
        proposal.approve(todo);
        let proposal_id = proposal.id;
        state.data.workspaces[0].proposals.push(proposal);
        state
            .data
            .sessions
            .insert(workspace_id, vec![worker, manager]);
        (state, workspace_id, proposal_id, worker_id, manager_id)
    }

    fn the_proposal(state: &AppState, id: uuid::Uuid) -> crate::models::Proposal {
        state.data.workspaces[0]
            .proposals
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .unwrap()
    }

    /// Piece one and two of the lifecycle: finishing always enters review —
    /// even with no approved check — and the manager receives a turn that
    /// says how to answer.
    #[test]
    fn manager_review_turn_follows_every_finish() {
        let (mut state, ws, proposal_id, _worker, manager_id) = review_world();
        let (tx, _rx) = mpsc::unbounded_channel();

        crate::app::handlers::tasks::finish_into_review(&mut state, ws, proposal_id, &tx);

        let after = the_proposal(&state, proposal_id);
        assert!(after.awaiting_review(), "no check, yet review still begins");

        let queue = &state.get_session(manager_id).unwrap().todo_queue;
        let turn = queue.items.last().expect("the manager got a turn");
        assert!(turn.text.contains("REVIEW TURN"), "{}", turn.text);
        assert!(turn.text.contains(&proposal_id.to_string()));
        assert!(turn.text.contains("manager.review"), "says how to answer");
        assert!(
            turn.text.contains("round 1/3"),
            "the bound is stated up front: {}",
            turn.text
        );
    }

    /// A finished proposal whose manager has left runs to the user, not to
    /// nobody: work reviewed by nobody must not read as done.
    #[test]
    fn manager_review_without_a_manager_goes_to_the_user() {
        let (mut state, ws, proposal_id, _worker, manager_id) = review_world();
        let (tx, _rx) = mpsc::unbounded_channel();
        state
            .data
            .sessions
            .get_mut(&ws)
            .unwrap()
            .retain(|s| s.id != manager_id);

        crate::app::handlers::tasks::finish_into_review(&mut state, ws, proposal_id, &tx);

        let after = the_proposal(&state, proposal_id);
        assert_eq!(after.review, Some(crate::models::ReviewPhase::NeedsUser));
    }

    /// The full loop: request_changes re-queues the findings on the same
    /// agent and burns a round; the next finish reviews again; accept
    /// resolves. And only the proposing manager's word counts.
    #[test]
    fn proposal_review_cycle_corrects_then_resolves() {
        let (mut state, ws, proposal_id, worker_id, _manager) = review_world();
        let (tx, _rx) = mpsc::unbounded_channel();
        let manager_short = the_proposal(&state, proposal_id).manager.clone();
        let pid = proposal_id.to_string();

        crate::app::handlers::tasks::finish_into_review(&mut state, ws, proposal_id, &tx);

        // A stranger's review is refused outright.
        apply_review(&mut state, "deadbeef", &pid, "accept", "", &tx);
        assert!(the_proposal(&state, proposal_id).awaiting_review());

        // Corrections go back to the worker, verbatim, and cost a round.
        apply_review(
            &mut state,
            &manager_short,
            &pid,
            "request_changes",
            "the restart counter never resets",
            &tx,
        );
        let after = the_proposal(&state, proposal_id);
        assert_eq!(after.review, Some(crate::models::ReviewPhase::Working));
        assert_eq!(after.review_rounds, 1);
        let worker_queue = &state.get_session(worker_id).unwrap().todo_queue;
        let correction = worker_queue.items.last().unwrap();
        assert!(
            correction.text.contains("the restart counter never resets"),
            "{}",
            correction.text
        );
        assert_eq!(after.todo_id, Some(correction.id), "the loop tracks the new item");

        // Round two ends in acceptance.
        crate::app::handlers::tasks::finish_into_review(&mut state, ws, proposal_id, &tx);
        apply_review(&mut state, &manager_short, &pid, "accept", "", &tx);
        assert_eq!(
            the_proposal(&state, proposal_id).review,
            Some(crate::models::ReviewPhase::Resolved)
        );
    }

    /// The loop is bounded: past its rounds, one more request_changes parks
    /// the proposal on the user instead of buying another lap.
    #[test]
    fn proposal_review_cycle_is_bounded() {
        let (mut state, ws, proposal_id, _worker, _manager) = review_world();
        let (tx, _rx) = mpsc::unbounded_channel();
        let manager_short = the_proposal(&state, proposal_id).manager.clone();
        let pid = proposal_id.to_string();

        state.data.workspaces[0]
            .proposals
            .iter_mut()
            .find(|p| p.id == proposal_id)
            .unwrap()
            .review_rounds = crate::models::MAX_REVIEW_ROUNDS;
        crate::app::handlers::tasks::finish_into_review(&mut state, ws, proposal_id, &tx);

        apply_review(
            &mut state,
            &manager_short,
            &pid,
            "request_changes",
            "still broken",
            &tx,
        );
        let after = the_proposal(&state, proposal_id);
        assert_eq!(after.review, Some(crate::models::ReviewPhase::NeedsUser));
        assert!(after.findings.unwrap().contains("still broken"));
    }

    // ---- the phone's desk -------------------------------------------------
    //
    // The two decisions the phone could not make before. Both go through the
    // helper the TUI's keys reach, so what is asserted here is the wiring:
    // that a tap lands on the same code an `a` at the desk would.

    /// Approving keeps the command and makes it real; dropping removes the
    /// check outright. Same helper as `a`/`x` on a desk check row.
    #[test]
    fn phone_desk_decides_a_proposed_check() {
        use crate::models::{Objective, Verification};
        use crate::remote::RemoteCommand;

        let world = || {
            let mut state = AppState::default();
            let mut ws = Workspace::new("zeta".into(), std::path::PathBuf::from("/tmp/z"));
            let mut objective = Objective::new("keep it green");
            objective.done_when = Some(Verification::proposed("cargo test"));
            let objective_id = objective.id;
            ws.objectives.push(objective);
            state.data.workspaces.push(ws);
            (state, objective_id)
        };
        let check = |state: &AppState| {
            state.data.workspaces[0].objectives[0].done_when.clone()
        };

        let (mut state, objective_id) = world();
        let (tx, _rx) = super::mpsc::unbounded_channel();
        apply_remote(
            &mut state,
            RemoteCommand::DecideCheck {
                objective: objective_id.to_string(),
                approve: true,
            },
            &tx,
        );
        let approved = check(&state).expect("approving keeps the command");
        assert!(!approved.proposed, "approved means no longer merely proposed");
        assert_eq!(approved.command, "cargo test");

        let (mut state, objective_id) = world();
        apply_remote(
            &mut state,
            RemoteCommand::DecideCheck {
                objective: objective_id.to_string(),
                approve: false,
            },
            &tx,
        );
        assert!(check(&state).is_none(), "dropping removes the check");

        // An id that names nothing is a log line, not a panic: the desk may
        // have decided it while the tap was in flight.
        let (mut state, _) = world();
        apply_remote(
            &mut state,
            RemoteCommand::DecideCheck {
                objective: uuid::Uuid::new_v4().to_string(),
                approve: true,
            },
            &tx,
        );
        assert!(check(&state).is_some_and(|v| v.proposed), "untouched");
    }

    /// The phone's decline on a "needs you" row. It posted to `/api/proposal`
    /// like any other decline, which reached `decide_proposal` — and that
    /// refuses anything not pending, so the tap did nothing at all and the
    /// row came back with the next snapshot.
    #[test]
    fn phone_desk_declines_a_needs_user_review() {
        use crate::models::{Proposal, ProposalState};
        use crate::remote::RemoteCommand;

        let mut state = AppState::default();
        let workspace = Workspace::new("zeta".into(), std::path::PathBuf::from("/tmp/z"));
        let workspace_id = workspace.id;
        state.data.workspaces.push(workspace);
        let agent = add_agent(&mut state, workspace_id, SessionStatus::Running, Some(30));

        let mut parked = Proposal::new("m1", "tidy the parser");
        parked.state = ProposalState::Approved;
        parked.agent = Some(crate::models::Session::short_id_of(agent));
        parked.needs_user("could not tell if the migration ran".into());
        let proposal_id = parked.id;
        state.data.workspaces[0].proposals.push(parked);
        assert_eq!(crate::remote::phone_desk_rows(&state).len(), 1);

        let (tx, _rx) = super::mpsc::unbounded_channel();
        apply_remote(
            &mut state,
            RemoteCommand::Decide {
                proposal: proposal_id.to_string(),
                approve: false,
            },
            &tx,
        );

        assert!(
            crate::remote::phone_desk_rows(&state).is_empty(),
            "the declined row has to leave the phone's desk too"
        );
        assert!(
            state.get_session(agent).unwrap().todo_queue.items.is_empty(),
            "declining queues nothing"
        );

        // The phone must not go on calling it approved either. `phase` used
        // to come out absent, which the page renders as the bare state —
        // "approved · tidy the parser", for a job just stopped.
        let closed = &state.data.workspaces[0].proposals[0];
        assert_eq!(closed.review, Some(crate::models::ReviewPhase::Closed));
        let shared: crate::remote::Shared = Default::default();
        crate::remote::publish(&mut state, &shared);
        let json = serde_json::to_value(&*shared.lock().unwrap()).unwrap();
        let published = &json["projects"][0]["proposals"][0];
        assert_eq!(published["phase"], "closed");
        assert_ne!(published["phase"], "working");
    }

    /// A phone that retries a post it never saw answered used to queue the
    /// job a second time: nothing checked the proposal was still parked.
    #[test]
    fn phone_desk_rearming_twice_queues_the_work_once() {
        use crate::models::{Proposal, ProposalState};
        use crate::remote::RemoteCommand;

        let mut state = AppState::default();
        let workspace = Workspace::new("zeta".into(), std::path::PathBuf::from("/tmp/z"));
        let workspace_id = workspace.id;
        state.data.workspaces.push(workspace);
        let agent = add_agent(&mut state, workspace_id, SessionStatus::Running, Some(30));

        let mut parked = Proposal::new("m1", "tidy the parser");
        parked.state = ProposalState::Approved;
        parked.agent = Some(crate::models::Session::short_id_of(agent));
        parked.needs_user("could not tell if the migration ran".into());
        let proposal_id = parked.id;
        state.data.workspaces[0].proposals.push(parked);

        let (tx, _rx) = super::mpsc::unbounded_channel();
        let rearm = || RemoteCommand::RearmReview {
            proposal: proposal_id.to_string(),
        };
        apply_remote(&mut state, rearm(), &tx);
        assert_eq!(state.get_session(agent).unwrap().todo_queue.items.len(), 1);

        apply_remote(&mut state, rearm(), &tx);
        assert_eq!(
            state.get_session(agent).unwrap().todo_queue.items.len(),
            1,
            "a repeated post must not hand the agent the same job twice"
        );
    }

    /// Re-arming a review the manager punted: the work goes back to the agent
    /// it named, with a fresh set of rounds and the findings carried across,
    /// exactly as the desk's yes on a "needs you" row does it.
    #[test]
    fn phone_desk_rearms_a_needs_user_proposal() {
        use crate::models::{Proposal, ProposalState};
        use crate::remote::RemoteCommand;

        let mut state = AppState::default();
        let workspace = Workspace::new("zeta".into(), std::path::PathBuf::from("/tmp/z"));
        let workspace_id = workspace.id;
        state.data.workspaces.push(workspace);
        let agent = add_agent(&mut state, workspace_id, SessionStatus::Running, Some(30));
        let short = crate::models::Session::short_id_of(agent);

        let mut parked = Proposal::new("m1", "tidy the parser");
        parked.state = ProposalState::Approved;
        parked.agent = Some(short.clone());
        parked.review_rounds = 3;
        parked.needs_user("could not tell if the migration ran".into());
        let proposal_id = parked.id;
        state.data.workspaces[0].proposals.push(parked);

        let (tx, _rx) = super::mpsc::unbounded_channel();
        apply_remote(
            &mut state,
            RemoteCommand::RearmReview {
                proposal: proposal_id.to_string(),
            },
            &tx,
        );

        let after = state.data.workspaces[0].proposals[0].clone();
        assert_eq!(
            after.review,
            Some(crate::models::ReviewPhase::Working),
            "re-armed work is working again, not still parked on the user"
        );
        assert_eq!(after.review_rounds, 0, "the user's approval buys fresh rounds");
        let todo_id = after.todo_id.expect("re-arming queues the work");

        let queued = state
            .get_session(agent)
            .unwrap()
            .todo_queue
            .items
            .iter()
            .find(|item| item.id == todo_id)
            .expect("the queued item reached the agent it named")
            .text
            .clone();
        assert!(
            queued.contains("could not tell if the migration ran"),
            "the findings travel with it, or the agent redoes the same thing: {queued}"
        );
        assert!(queued.contains("tidy the parser"), "{queued}");
    }
}
