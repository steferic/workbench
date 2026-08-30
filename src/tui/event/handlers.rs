use crate::app::{Action, AppState, FocusPanel, PendingDelete, TasksTab};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::key_modes::handle_input_mode_key;
use super::shortcuts::{agent_shortcut, check_global_keys};
use super::EventHandler;

impl EventHandler {
    pub(super) fn handle_key_event(&self, key: KeyEvent, state: &AppState) -> Action {
        // Handle input mode first
        if let Some(action) = handle_input_mode_key(&key, state) {
            return action;
        }

        // Handle pending delete confirmation
        if state.ui.pending_delete.is_some() {
            return match key.code {
                KeyCode::Char('d') => match &state.ui.pending_delete {
                    Some(PendingDelete::Session(_, _)) => Action::ConfirmDeleteSession,
                    Some(PendingDelete::Workspace(_, _)) => Action::ConfirmDeleteWorkspace,
                    None => Action::Tick,
                },
                KeyCode::Esc => Action::CancelPendingDelete,
                _ => Action::CancelPendingDelete,
            };
        }

        // Handle pending quit confirmation.
        //
        // Esc cancels. It used to confirm, alongside `q` and `y`, which made
        // the key you reach for to back out of an accidental quit the key that
        // completed it. Confirming takes `y`, Enter, or Ctrl+Q again — the
        // same chord that asked the question.
        if state.ui.pending_quit {
            let confirmed = matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y'))
                || key.code == KeyCode::Enter
                || (key.code == KeyCode::Char('q')
                    && key.modifiers.contains(KeyModifiers::CONTROL));
            return if confirmed {
                Action::ConfirmQuit
            } else {
                Action::CancelQuit
            };
        }

        // Global Ctrl+P - command palette
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('p') {
            return Action::EnterCommandPalette;
        }

        // Global window navigation with Shift+Left/Right arrows
        if key.modifiers.contains(KeyModifiers::SHIFT) {
            match key.code {
                KeyCode::Left => return Action::FocusLeft,
                KeyCode::Right => return Action::FocusRight,
                _ => {}
            }
        }

        // Normal mode key handling based on focused panel
        match state.ui.focus {
            FocusPanel::WorkspaceList => self.handle_workspace_list_keys(key, state),
            FocusPanel::SessionList => self.handle_session_list_keys(key, state),
            FocusPanel::TasksPane => self.handle_tasks_pane_keys(key, state),
            FocusPanel::UtilitiesPane => self.handle_utilities_pane_keys(key, state),
            FocusPanel::OutputPane => self.handle_output_pane_keys(key, state),
            FocusPanel::PinnedTerminalPane(idx) => {
                self.handle_pinned_terminal_keys(key, state, idx)
            }
        }
    }

    fn handle_workspace_list_keys(&self, key: KeyEvent, state: &AppState) -> Action {
        if let Some(action) = check_global_keys(&key, &state.system.user_config) {
            return action;
        }

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => Action::MoveDown,
            KeyCode::Char('k') | KeyCode::Up => Action::MoveUp,
            KeyCode::Char('l') => Action::FocusRight,
            KeyCode::Char('n') => Action::EnterWorkspaceActionMode,
            KeyCode::Char('g') => Action::OpenRepositoryMap,
            KeyCode::Enter => Action::FocusRight,
            KeyCode::Char('d') => {
                if let Some(workspace) = state.selected_workspace() {
                    Action::InitiateDeleteWorkspace(workspace.id, workspace.name.clone())
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('h') => Action::EnterConfigWindow,
            KeyCode::Char('?') => Action::EnterConfigWindow,
            _ => Action::Tick,
        }
    }

    fn handle_session_list_keys(&self, key: KeyEvent, state: &AppState) -> Action {
        if let Some(action) = check_global_keys(&key, &state.system.user_config) {
            return action;
        }

        if let Some((agent_type, dangerously_skip_permissions, with_worktree)) =
            agent_shortcut(&key, &state.system.user_config.agents)
        {
            return Action::CreateSession(agent_type, dangerously_skip_permissions, with_worktree);
        }

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => Action::MoveDown,
            KeyCode::Char('k') | KeyCode::Up => Action::MoveUp,
            KeyCode::Char('l') => Action::FocusRight,
            KeyCode::Char('n') => Action::EnterCreateSessionMode,
            KeyCode::Enter => {
                if let Some(session) = state.selected_session() {
                    if matches!(
                        session.status,
                        crate::models::SessionStatus::Stopped
                            | crate::models::SessionStatus::Errored
                    ) {
                        Action::RestartSession(session.id)
                    } else {
                        Action::ActivateSession(session.id)
                    }
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('r') => {
                if let Some(session) = state.selected_session() {
                    if matches!(
                        session.status,
                        crate::models::SessionStatus::Stopped
                            | crate::models::SessionStatus::Errored
                    ) {
                        Action::RestartSession(session.id)
                    } else {
                        Action::Tick
                    }
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('s') => {
                if let Some(session) = state.selected_session() {
                    Action::StopSession(session.id)
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('x') => {
                if let Some(session) = state.selected_session() {
                    Action::KillSession(session.id)
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('d') => {
                if let Some(session) = state.selected_session() {
                    Action::InitiateDeleteSession(session.id, session.display_name())
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('t') => Action::CreateTerminal,
            KeyCode::Char('c') => {
                if let Some(session) = state.selected_session() {
                    if session.agent_type.is_terminal() {
                        Action::EnterSetStartCommandMode
                    } else {
                        Action::Tick
                    }
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('p') => {
                if let Some(session) = state.selected_session() {
                    if state.pinned_terminal_ids().contains(&session.id) {
                        Action::UnpinSession(session.id)
                    } else {
                        Action::PinSession(session.id)
                    }
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('u') => {
                if let Some(session) = state.selected_session() {
                    if state.pinned_terminal_ids().contains(&session.id) {
                        Action::UnpinSession(session.id)
                    } else {
                        Action::Tick
                    }
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('\\') | KeyCode::Char('/') => Action::ToggleSplitView,
            KeyCode::Char('P') => Action::EnterParallelTaskMode,
            KeyCode::Char('X') => {
                if let Some(session) = state.selected_session() {
                    if let Some(task_id) = state.selected_workspace().and_then(|ws| {
                        ws.parallel_tasks
                            .iter()
                            .find(|t| t.attempts.iter().any(|a| a.session_id == session.id))
                            .map(|t| t.id)
                    }) {
                        Action::CancelParallelTask(task_id)
                    } else {
                        Action::Tick
                    }
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('m') => {
                if let Some(session) = state.selected_session() {
                    if session.has_worktree() {
                        Action::MergeSessionWorktree(session.id)
                    } else {
                        Action::Tick
                    }
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('w') => {
                if let Some(session) = state.selected_session() {
                    if session.has_worktree() {
                        let is_active = state
                            .selected_workspace()
                            .and_then(|ws| ws.active_worktree_session_id)
                            .map(|id| id == session.id)
                            .unwrap_or(false);

                        if is_active {
                            Action::SwitchToWorktree(None)
                        } else {
                            Action::SwitchToWorktree(Some(session.id))
                        }
                    } else {
                        Action::Tick
                    }
                } else {
                    Action::Tick
                }
            }
            KeyCode::Char('h') => Action::EnterConfigWindow,
            KeyCode::Char('?') => Action::EnterConfigWindow,
            _ => Action::Tick,
        }
    }

    fn handle_tasks_pane_keys(&self, key: KeyEvent, state: &AppState) -> Action {
        if let Some(action) = check_global_keys(&key, &state.system.user_config) {
            return action;
        }

        // The detail overlay swallows the pane's keys while open: the row it
        // shows is the row a/x act on, and everything else just closes it.
        if state.ui.detail.is_some() {
            return match key.code {
                KeyCode::Char('a') => Action::DeskDecideDetail(true),
                KeyCode::Char('x') => Action::DeskDecideDetail(false),
                _ => Action::CloseDetail,
            };
        }

        let managers = state.ui.selected_tasks_tab == TasksTab::Managers;
        let desk = state.ui.selected_tasks_tab == TasksTab::Desk;

        // A manager is started exactly the way an agent is: the provider
        // number, with the same Shift and Alt meanings. One key, no dialog —
        // the only difference from Sessions is what the session is for.
        if managers {
            if let Some((agent_type, skip_permissions, with_worktree)) =
                agent_shortcut(&key, &state.system.user_config.agents)
            {
                return Action::CreateSession(
                    agent_type.as_manager(),
                    skip_permissions,
                    with_worktree,
                );
            }
        }

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => Action::SelectNextTask,
            KeyCode::Char('k') | KeyCode::Up => Action::SelectPrevTask,
            KeyCode::Char('l') => Action::FocusRight,
            KeyCode::Tab => Action::ToggleTasksTab,

            // -- Desk (everything waiting on you) --
            KeyCode::Char('a') if desk => Action::DeskDecide(true),
            KeyCode::Char('x') if desk => Action::DeskDecide(false),
            KeyCode::Enter if desk => Action::DeskOpen,

            // -- Managers tab --
            KeyCode::Enter if managers => Action::FocusSelectedTaskAgent,
            KeyCode::Char('d') if managers => {
                match crate::app::managers_view::selected(state) {
                    Some(row) => Action::InitiateDeleteSession(row.session_id, row.name),
                    None => Action::Tick,
                }
            }

            // -- Objectives tab (the project's standing priorities) --
            KeyCode::Enter => Action::OpenDetail,
            KeyCode::Char('n') => Action::EditObjective(false),
            KeyCode::Char('e') => Action::EditObjective(true),
            KeyCode::Char('d') => Action::DeleteObjective,
            KeyCode::Char(' ') => Action::CycleObjectiveState,
            KeyCode::Char('K') => Action::MoveObjective(-1),
            KeyCode::Char('J') => Action::MoveObjective(1),
            // On a proposal row: turn it into work, or say no. Approving is
            // the only way a manager's suggestion reaches an agent.
            KeyCode::Char('a') => Action::ApproveProposal,
            KeyCode::Char('x') => Action::DeclineProposal,

            KeyCode::Char('h') => Action::EnterConfigWindow,
            KeyCode::Char('?') => Action::EnterConfigWindow,
            _ => Action::Tick,
        }
    }

    fn handle_utilities_pane_keys(&self, key: KeyEvent, state: &AppState) -> Action {
        use crate::app::{UtilityItem, UtilitySection};

        if let Some(action) = check_global_keys(&key, &state.system.user_config) {
            return action;
        }

        if state.ui.utility_section == UtilitySection::Notepad {
            if key.code == KeyCode::Tab {
                return Action::ToggleUtilitySection;
            }
            if key.code == KeyCode::Esc {
                return Action::ToggleUtilitySection;
            }
            return Action::NotepadInput(key);
        }

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => Action::SelectNextUtility,
            KeyCode::Char('k') | KeyCode::Up => Action::SelectPrevUtility,
            KeyCode::Char('l') | KeyCode::Enter => match state.ui.utility_section {
                UtilitySection::Utilities => Action::ActivateUtility,
                UtilitySection::Themes => Action::ActivateUtility,
                UtilitySection::Sounds => match state.ui.selected_sound {
                    UtilityItem::BrownNoise => Action::ToggleBrownNoise,
                    UtilityItem::ClassicalRadio => Action::ToggleClassicalRadio,
                    UtilityItem::OceanWaves => Action::ToggleOceanWaves,
                    UtilityItem::WindChimes => Action::ToggleWindChimes,
                    UtilityItem::RainforestRain => Action::ToggleRainforestRain,
                    _ => Action::Tick,
                },
                UtilitySection::Notepad => Action::Tick,
            },
            KeyCode::Tab => Action::ToggleUtilitySection,
            KeyCode::Char('h') => Action::EnterConfigWindow,
            KeyCode::Char('?') => Action::EnterConfigWindow,
            _ => Action::Tick,
        }
    }

    fn handle_output_pane_keys(&self, key: KeyEvent, state: &AppState) -> Action {
        if let Some(action) = check_global_keys(&key, &state.system.user_config) {
            return action;
        }

        if state.text_selection().start.is_some() {
            match key.code {
                // Ctrl+C copies when a selection is active. Without a selection
                // it falls through and is sent to the PTY as SIGINT (0x03).
                // Cmd+C and Ctrl+Shift+C also work on terminals that report
                // those modifiers (Ghostty intercepts Cmd+C itself, so Ctrl+C
                // is the dependable shortcut on macOS).
                KeyCode::Char('c') | KeyCode::Char('C')
                    if key.modifiers.contains(KeyModifiers::SUPER)
                        || key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    return Action::CopySelection;
                }
                KeyCode::Esc => return Action::ClearSelection,
                _ => {}
            }
        }

        if let Some(session_id) = state.active_session_id() {
            match key.code {
                KeyCode::Esc => Action::SendInput(session_id, vec![0x1b]),
                KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    Action::ScrollOutputUp
                }
                KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    Action::ScrollOutputDown
                }
                KeyCode::PageUp => Action::ScrollOutputUp,
                KeyCode::PageDown => Action::ScrollOutputDown,
                KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Action::FocusLeft
                }
                KeyCode::BackTab => Action::SendInput(session_id, b"\x1b[Z".to_vec()),
                KeyCode::Char(c) => {
                    let data = if key.modifiers.contains(KeyModifiers::CONTROL) {
                        vec![(c as u8) & 0x1f]
                    } else if key.modifiers.contains(KeyModifiers::ALT) {
                        vec![0x1b, c as u8]
                    } else {
                        c.to_string().into_bytes()
                    };
                    Action::SendInput(session_id, data)
                }
                KeyCode::Enter => Action::SendInput(session_id, vec![b'\r']),
                KeyCode::Backspace => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, vec![0x1b, 0x7f])
                    } else if key.modifiers.contains(KeyModifiers::SUPER) {
                        Action::SendInput(session_id, vec![0x15])
                    } else {
                        Action::SendInput(session_id, vec![0x7f])
                    }
                }
                KeyCode::Delete => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, vec![0x1b, b'd'])
                    } else {
                        Action::SendInput(session_id, b"\x1b[3~".to_vec())
                    }
                }
                KeyCode::Tab => Action::SendInput(session_id, vec![b'\t']),
                KeyCode::Up => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, b"\x1b[1;3A".to_vec())
                    } else {
                        Action::SendInput(session_id, b"\x1b[A".to_vec())
                    }
                }
                KeyCode::Down => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, b"\x1b[1;3B".to_vec())
                    } else {
                        Action::SendInput(session_id, b"\x1b[B".to_vec())
                    }
                }
                KeyCode::Right => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, vec![0x1b, b'f'])
                    } else if key.modifiers.contains(KeyModifiers::SUPER) {
                        Action::SendInput(session_id, vec![0x05])
                    } else {
                        Action::SendInput(session_id, b"\x1b[C".to_vec())
                    }
                }
                KeyCode::Left => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, vec![0x1b, b'b'])
                    } else if key.modifiers.contains(KeyModifiers::SUPER) {
                        Action::SendInput(session_id, vec![0x01])
                    } else {
                        Action::SendInput(session_id, b"\x1b[D".to_vec())
                    }
                }
                KeyCode::Home => Action::SendInput(session_id, vec![0x01]),
                KeyCode::End => Action::SendInput(session_id, vec![0x05]),
                KeyCode::F(n) => {
                    let seq = match n {
                        1 => b"\x1bOP".to_vec(),
                        2 => b"\x1bOQ".to_vec(),
                        3 => b"\x1bOR".to_vec(),
                        4 => b"\x1bOS".to_vec(),
                        5 => b"\x1b[15~".to_vec(),
                        6 => b"\x1b[17~".to_vec(),
                        7 => b"\x1b[18~".to_vec(),
                        8 => b"\x1b[19~".to_vec(),
                        9 => b"\x1b[20~".to_vec(),
                        10 => b"\x1b[21~".to_vec(),
                        11 => b"\x1b[23~".to_vec(),
                        12 => b"\x1b[24~".to_vec(),
                        _ => vec![],
                    };
                    if seq.is_empty() {
                        Action::Tick
                    } else {
                        Action::SendInput(session_id, seq)
                    }
                }
                KeyCode::Insert => Action::SendInput(session_id, b"\x1b[2~".to_vec()),
                _ => Action::Tick,
            }
        } else {
            match key.code {
                KeyCode::Char('h') | KeyCode::Esc => Action::FocusLeft,
                KeyCode::Char('?') => Action::EnterConfigWindow,
                    _ => Action::Tick,
            }
        }
    }

    fn handle_pinned_terminal_keys(
        &self,
        key: KeyEvent,
        state: &AppState,
        pane_idx: usize,
    ) -> Action {
        if let Some(action) = check_global_keys(&key, &state.system.user_config) {
            return action;
        }

        if state.pinned_text_selection(pane_idx).start.is_some() {
            match key.code {
                // Ctrl+C copies when a selection is active; without a selection
                // it falls through and is sent to the PTY as SIGINT.
                KeyCode::Char('c') | KeyCode::Char('C')
                    if key.modifiers.contains(KeyModifiers::SUPER)
                        || key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    return Action::CopySelection;
                }
                KeyCode::Esc => return Action::ClearSelection,
                _ => {}
            }
        }

        if let Some(session_id) = state.pinned_terminal_id_at(pane_idx) {
            match key.code {
                KeyCode::Esc => Action::SendInput(session_id, vec![0x1b]),
                KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Action::FocusLeft
                }
                KeyCode::BackTab => Action::SendInput(session_id, b"\x1b[Z".to_vec()),
                KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Action::NextPinnedPane
                }
                KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Action::PrevPinnedPane
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Action::UnpinFocusedSession
                }
                KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    Action::ScrollOutputUp
                }
                KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    Action::ScrollOutputDown
                }
                KeyCode::PageUp => Action::ScrollOutputUp,
                KeyCode::PageDown => Action::ScrollOutputDown,
                KeyCode::Char(c) => {
                    let data = if key.modifiers.contains(KeyModifiers::CONTROL) {
                        vec![(c as u8) & 0x1f]
                    } else if key.modifiers.contains(KeyModifiers::ALT) {
                        vec![0x1b, c as u8]
                    } else {
                        c.to_string().into_bytes()
                    };
                    Action::SendInput(session_id, data)
                }
                KeyCode::Enter => Action::SendInput(session_id, vec![b'\r']),
                KeyCode::Backspace => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, vec![0x1b, 0x7f])
                    } else if key.modifiers.contains(KeyModifiers::SUPER) {
                        Action::SendInput(session_id, vec![0x15])
                    } else {
                        Action::SendInput(session_id, vec![0x7f])
                    }
                }
                KeyCode::Delete => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, vec![0x1b, b'd'])
                    } else {
                        Action::SendInput(session_id, b"\x1b[3~".to_vec())
                    }
                }
                KeyCode::Tab => Action::SendInput(session_id, vec![b'\t']),
                KeyCode::Up => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, b"\x1b[1;3A".to_vec())
                    } else {
                        Action::SendInput(session_id, b"\x1b[A".to_vec())
                    }
                }
                KeyCode::Down => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, b"\x1b[1;3B".to_vec())
                    } else {
                        Action::SendInput(session_id, b"\x1b[B".to_vec())
                    }
                }
                KeyCode::Right => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, vec![0x1b, b'f'])
                    } else if key.modifiers.contains(KeyModifiers::SUPER) {
                        Action::SendInput(session_id, vec![0x05])
                    } else {
                        Action::SendInput(session_id, b"\x1b[C".to_vec())
                    }
                }
                KeyCode::Left => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        Action::SendInput(session_id, vec![0x1b, b'b'])
                    } else if key.modifiers.contains(KeyModifiers::SUPER) {
                        Action::SendInput(session_id, vec![0x01])
                    } else {
                        Action::SendInput(session_id, b"\x1b[D".to_vec())
                    }
                }
                KeyCode::Home => Action::SendInput(session_id, vec![0x01]),
                KeyCode::End => Action::SendInput(session_id, vec![0x05]),
                KeyCode::F(n) => {
                    let seq = match n {
                        1 => b"\x1bOP".to_vec(),
                        2 => b"\x1bOQ".to_vec(),
                        3 => b"\x1bOR".to_vec(),
                        4 => b"\x1bOS".to_vec(),
                        5 => b"\x1b[15~".to_vec(),
                        6 => b"\x1b[17~".to_vec(),
                        7 => b"\x1b[18~".to_vec(),
                        8 => b"\x1b[19~".to_vec(),
                        9 => b"\x1b[20~".to_vec(),
                        10 => b"\x1b[21~".to_vec(),
                        11 => b"\x1b[23~".to_vec(),
                        12 => b"\x1b[24~".to_vec(),
                        _ => vec![],
                    };
                    if seq.is_empty() {
                        Action::Tick
                    } else {
                        Action::SendInput(session_id, seq)
                    }
                }
                KeyCode::Insert => Action::SendInput(session_id, b"\x1b[2~".to_vec()),
                _ => Action::Tick,
            }
        } else {
            match key.code {
                KeyCode::Esc | KeyCode::Char('h') => Action::FocusLeft,
                _ => Action::Tick,
            }
        }
    }
}

#[cfg(test)]
mod quit_tests {
    use super::*;
    use crate::models::Workspace;

    fn state() -> AppState {
        let mut state = AppState::default();
        let workspace = Workspace::new("w".into(), std::path::PathBuf::from("/tmp/w"));
        state.data.workspaces.push(workspace);
        state
    }

    fn press(events: &EventHandler, state: &AppState, code: KeyCode, mods: KeyModifiers) -> Action {
        events.handle_key_event(KeyEvent::new(code, mods), state)
    }

    /// Quitting kills every agent on the machine, so it takes the one chord
    /// nobody presses by accident.
    #[test]
    fn ctrl_q_is_the_way_out() {
        let events = EventHandler::new();
        let mut state = state();
        for panel in [
            FocusPanel::WorkspaceList,
            FocusPanel::SessionList,
            FocusPanel::TasksPane,
            FocusPanel::UtilitiesPane,
            FocusPanel::OutputPane,
        ] {
            state.ui.focus = panel;
            assert!(
                matches!(
                    press(&events, &state, KeyCode::Char('q'), KeyModifiers::CONTROL),
                    Action::InitiateQuit
                ),
                "Ctrl+Q should ask to quit from {panel:?}"
            );
        }
    }

    /// The bug this guards: `q` ended the session outright from four panes,
    /// and `Esc` did it from the workspace list. Both are one finger away from
    /// keys used constantly for navigation.
    #[test]
    fn a_bare_q_or_esc_never_quits() {
        let events = EventHandler::new();
        let mut state = state();
        for panel in [
            FocusPanel::WorkspaceList,
            FocusPanel::SessionList,
            FocusPanel::TasksPane,
            FocusPanel::UtilitiesPane,
            FocusPanel::OutputPane,
        ] {
            state.ui.focus = panel;
            for code in [KeyCode::Char('q'), KeyCode::Esc] {
                let action = press(&events, &state, code, KeyModifiers::NONE);
                assert!(
                    !matches!(action, Action::Quit | Action::InitiateQuit | Action::ConfirmQuit),
                    "{code:?} must not quit from {panel:?}, got {action:?}"
                );
            }
        }
    }

    /// Esc used to *confirm* the quit it is the natural key to escape from.
    #[test]
    fn esc_backs_out_of_the_confirmation() {
        let events = EventHandler::new();
        let mut state = state();
        state.ui.pending_quit = true;

        assert!(matches!(
            press(&events, &state, KeyCode::Esc, KeyModifiers::NONE),
            Action::CancelQuit
        ));
        // And so does anything else that is not a yes.
        assert!(matches!(
            press(&events, &state, KeyCode::Char('n'), KeyModifiers::NONE),
            Action::CancelQuit
        ));
        assert!(matches!(
            press(&events, &state, KeyCode::Char('q'), KeyModifiers::NONE),
            Action::CancelQuit
        ));
    }

    #[test]
    fn yes_enter_or_the_same_chord_confirms() {
        let events = EventHandler::new();
        let mut state = state();
        state.ui.pending_quit = true;

        for (code, mods) in [
            (KeyCode::Char('y'), KeyModifiers::NONE),
            (KeyCode::Char('Y'), KeyModifiers::NONE),
            (KeyCode::Enter, KeyModifiers::NONE),
            (KeyCode::Char('q'), KeyModifiers::CONTROL),
        ] {
            assert!(
                matches!(press(&events, &state, code, mods), Action::ConfirmQuit),
                "{code:?}+{mods:?} should confirm"
            );
        }
    }
}
