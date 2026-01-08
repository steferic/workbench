use crate::app::{Action, AppState};
use crate::pty::PtyManager;
use anyhow::Result;
use tokio::sync::mpsc;

use super::handlers::{input, navigation, session, todo, workspace};
use super::pty_ops::resize_ptys_to_panes;

pub fn process_action(
    state: &mut AppState,
    action: Action,
    pty_manager: &PtyManager,
    action_tx: &mpsc::UnboundedSender<Action>,
) -> Result<()> {
    match action {
        Action::Quit => {
            state.system.should_quit = true;
        }
        Action::Tick => {
            state.tick_animation();
            let newly_idle = state.update_idle_queue();

            // Check if analyzer session went idle
            if let Some(analyzer_id) = state.ui.analyzer_session_id {
                if newly_idle.contains(&analyzer_id) {
                    if let Some(parser) = state.system.output_buffers.get(&analyzer_id) {
                        let screen = parser.screen();
                        let contents = screen.contents();
                        for line in contents.lines() {
                            if let Some(idx) = line.find("TODO: ") {
                                let todo_text = line[idx + 6..].trim();
                                if !todo_text.is_empty() && todo_text.len() > 5 {
                                    let clean_text: String = todo_text.chars().filter(|c| !c.is_control()).collect();
                                    let _ = action_tx.send(Action::AddSuggestedTodo(clean_text));
                                }
                            }
                        }
                    }
                    state.ui.analyzer_session_id = None;
                }
            }

            // Process newly idle sessions
            for session_id in &newly_idle {
                if let Some(workspace_id) = state.workspace_id_for_session(*session_id) {
                    let has_in_progress = state.get_workspace(workspace_id)
                        .and_then(|ws| ws.todo_for_session(*session_id))
                        .map(|t| t.is_in_progress())
                        .unwrap_or(false);

                    if has_in_progress {
                        if let Some(ws) = state.get_workspace_mut(workspace_id) {
                            if let Some(todo) = ws.todo_for_session_mut(*session_id) {
                                let _ = action_tx.send(Action::MarkTodoReadyForReview(todo.id));
                            }
                        }
                    }
                }
            }

            // Autorun dispatch
            if state.ui.todo_pane_mode == crate::app::TodoPaneMode::Autorun {
                let idle_sessions = state.data.idle_queue.clone();
                for session_id in idle_sessions {
                    if let Some(workspace_id) = state.workspace_id_for_session(session_id) {
                        let has_in_progress = state.get_workspace(workspace_id)
                            .and_then(|ws| ws.todo_for_session(session_id))
                            .map(|t| t.is_in_progress())
                            .unwrap_or(false);

                        if !has_in_progress {
                            let pending = state.get_workspace(workspace_id)
                                .and_then(|ws| ws.next_pending_todo())
                                .map(|t| (t.id, t.description.clone()));
                            
                            if let Some((id, desc)) = pending {
                                let _ = action_tx.send(Action::DispatchTodoToSession(session_id, id, desc));
                                break;
                            }
                        }
                    }
                }
            }
        }
        Action::Resize(w, h) => {
            state.system.terminal_size = (w, h);
            resize_ptys_to_panes(state);
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
            
            match action {
                // Workspace actions
                Action::ToggleWorkspaceStatus | Action::InitiateDeleteWorkspace(_, _) |
                Action::ConfirmDeleteWorkspace | Action::EnterWorkspaceActionMode |
                Action::SelectNextWorkspaceAction | Action::SelectPrevWorkspaceAction |
                Action::ConfirmWorkspaceAction | Action::EnterWorkspaceNameMode |
                Action::CreateNewWorkspace(_) => {
                    workspace::handle_workspace_action(state, action)?;
                }

                // Session actions
                Action::CreateSession(_, _) | Action::CreateTerminal |
                Action::ActivateSession(_) | Action::RestartSession(_) | Action::StopSession(_) |
                Action::KillSession(_) | Action::InitiateDeleteSession(_, _) |
                Action::ConfirmDeleteSession | Action::CancelPendingDelete | Action::EnterCreateSessionMode |
                Action::EnterSetStartCommandMode | Action::SetStartCommand(_, _) | Action::PinSession(_) |
                Action::UnpinSession(_) | Action::UnpinFocusedSession | Action::ToggleSplitView |
                Action::SessionExited(_, _) | Action::PtyOutput(_, _) | Action::SendInput(_, _) => {
                    session::handle_session_action(state, action, pty_manager, action_tx)?;
                }

                // Todo actions
                Action::SelectNextTodo | Action::SelectPrevTodo | Action::EnterCreateTodoMode |
                Action::CreateTodo(_) | Action::MarkTodoDone | Action::RunSelectedTodo |
                Action::ToggleTodoPaneMode | Action::InitiateDeleteTodo(_, _) | Action::ConfirmDeleteTodo |
                Action::DispatchTodoToSession(_, _, _) | Action::MarkTodoReadyForReview(_) |
                Action::AddSuggestedTodo(_) | Action::ApproveSuggestedTodo(_) |
                Action::ApproveAllSuggestedTodos | Action::ArchiveTodo(_) | Action::ToggleTodosTab |
                Action::ActivateUtility => {
                    todo::handle_todo_action(state, action, action_tx)?;
                }

                // Navigation actions
                Action::MoveUp | Action::MoveDown | Action::FocusLeft | Action::FocusRight |
                Action::NextPinnedPane | Action::PrevPinnedPane | Action::ScrollOutputUp |
                Action::ScrollOutputDown | Action::JumpToNextIdle | Action::MouseClick(_, _) |
                Action::MouseDrag(_, _) | Action::MouseUp(_, _) | Action::CopySelection |
                Action::Paste(_) | Action::ClearSelection | Action::SelectNextUtility |
                Action::SelectPrevUtility | Action::ToggleUtilitySection | Action::ToggleConfigItem |
                Action::ToggleBrownNoise => {
                    navigation::handle_navigation_action(state, action, pty_manager, action_tx)?;
                }

                // Input actions
                Action::EnterHelpMode | Action::ExitMode | Action::InputChar(_) |
                Action::InputBackspace | Action::NotepadInput(_) |
                Action::FileBrowserUp | Action::FileBrowserDown | Action::FileBrowserEnter |
                Action::FileBrowserBack | Action::FileBrowserSelect => {
                    input::handle_input_action(state, action)?;
                }

                // Global already handled
                Action::Quit | Action::Tick | Action::Resize(_, _) => {}
            }
        }
    }

    Ok(())
}
