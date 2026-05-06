pub mod config;
pub mod input;
pub mod navigation;
pub mod parallel;
pub mod session;
mod session_worktree;
pub mod todo;
pub mod workspace;

use crate::app::{AppState, Toast, ToastLevel};
use crate::persistence;
use std::fmt::Display;

fn push_error_toast(state: &mut AppState, message: impl Into<String>) {
    state.ui.toasts.push_back(Toast::new(
        message.into(),
        ToastLevel::Error,
        std::time::Duration::from_secs(5),
    ));
    while state.ui.toasts.len() > 5 {
        state.ui.toasts.pop_front();
    }
}

pub(crate) fn report_persistence_error(state: &mut AppState, context: &str, err: anyhow::Error) {
    crate::logger::warn(format!("{context}: {err}"));
    push_error_toast(state, "Failed to save changes");
}

pub(crate) fn report_runtime_error(
    state: &mut AppState,
    context: &str,
    err: impl Display,
    message: &str,
) {
    crate::logger::warn(format!("{context}: {err}"));
    push_error_toast(state, message);
}

pub(crate) fn report_background_error(context: &str, err: impl Display) {
    crate::logger::warn(format!("{context}: {err}"));
}

pub(crate) fn save_state(state: &mut AppState, context: &str) {
    if let Err(err) = persistence::save(&state.data.workspaces, &state.data.sessions) {
        report_persistence_error(state, context, err);
    }
}

pub(crate) fn save_state_with_notepad(state: &mut AppState, context: &str) {
    let notepad_contents = state.notepad_content_for_persistence();
    if let Err(err) = persistence::save_with_notepad(
        &state.data.workspaces,
        &state.data.sessions,
        &notepad_contents,
    ) {
        report_persistence_error(state, context, err);
    }
}

pub(crate) fn save_config(state: &mut AppState, config: &persistence::GlobalConfig, context: &str) {
    if let Err(err) = persistence::save_config(config) {
        report_persistence_error(state, context, err);
    }
}

#[cfg(test)]
mod tests {
    use super::report_persistence_error;
    use crate::app::{AppState, Toast, ToastLevel};
    use std::time::Duration;

    #[test]
    fn persistence_error_adds_error_toast_and_caps_queue() {
        let mut state = AppState::default();
        for idx in 0..5 {
            state.ui.toasts.push_back(Toast::new(
                format!("old {idx}"),
                ToastLevel::Info,
                Duration::from_secs(3),
            ));
        }

        report_persistence_error(
            &mut state,
            "failed to save test state",
            anyhow::anyhow!("disk full"),
        );

        assert_eq!(state.ui.toasts.len(), 5);
        assert_eq!(state.ui.toasts.front().unwrap().message, "old 1");
        let newest = state.ui.toasts.back().unwrap();
        assert_eq!(newest.message, "Failed to save changes");
        assert_eq!(newest.level, ToastLevel::Error);
    }
}
