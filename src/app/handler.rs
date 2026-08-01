use crate::app::{Action, AppState};
use crate::git;
use crate::pty::PtyManager;
use anyhow::Result;
use std::collections::HashMap;
use std::time::Duration;
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
            super::todo_dispatch::tick(state, action_tx);
            tasks::sync_selection(state);
            refresh_agent_tasks(state, action_tx);

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
        Action::DiffStatsUpdated(stats) => {
            state.system.diff_stats = stats;
        }
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
                // Workspace actions
                Action::ToggleWorkspaceStatus | Action::InitiateDeleteWorkspace(_, _) |
                Action::ConfirmDeleteWorkspace | Action::EnterWorkspaceActionMode |
                Action::NextWorkspaceChoice | Action::PrevWorkspaceChoice |
                Action::ConfirmWorkspaceChoice | Action::EnterWorkspaceNameMode |
                Action::CreateNewWorkspace(_) => {
                    workspace::handle_workspace_action(state, action, pty_manager, action_tx, pty_tx)?;
                }

                // Session actions
                Action::CreateSession(_, _, _) | Action::CreateTerminal |
                Action::ActivateSession(_) | Action::RestartSession(_) | Action::StopSession(_) |
                Action::KillSession(_) | Action::InitiateDeleteSession(_, _) |
                Action::ConfirmDeleteSession | Action::CancelPendingDelete | Action::EnterCreateSessionMode |
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
                Action::UtilityContentLoaded(_) | Action::DiffStatsUpdated(_) => {}
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

/// How often the agent session logs are re-read for the tasks pane. Each pass
/// is usually a stat per agent (nothing new to parse), so this is cheap; it
/// still runs off-thread because locating a log can touch many directories.
const TASK_REFRESH_INTERVAL: Duration = Duration::from_millis(1000);

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
    use super::{plan_task_refresh, process_action};
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

    fn state_with_workspace() -> (AppState, uuid::Uuid) {
        let mut state = AppState::default();
        let workspace = Workspace::new("w".into(), std::path::PathBuf::from("/tmp/w"));
        let id = workspace.id;
        state.data.workspaces.push(workspace);
        (state, id)
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
}
