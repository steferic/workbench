pub mod config;
pub mod input;
pub mod navigation;
pub mod parallel;
pub mod session;
mod session_worktree;
pub mod tasks;
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

/// Mark state as needing a save. The actual write is debounced and performed
/// off the event loop by [`flush_dirty_state`]; every flush includes notepad
/// content, so the two save entry points below are equivalent.
pub(crate) fn save_state(state: &mut AppState, _context: &str) {
    state.system.state_dirty = true;
}

pub(crate) fn save_state_with_notepad(state: &mut AppState, context: &str) {
    save_state(state, context);
}

const STATE_SAVE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(400);

/// Flush pending state changes to disk. Serialization happens inline (cheap,
/// in-memory); the file write runs on a blocking thread so the event loop
/// never waits on disk. `force` bypasses the debounce and writes
/// synchronously — used on shutdown.
pub(crate) fn flush_dirty_state(
    state: &mut AppState,
    action_tx: &tokio::sync::mpsc::UnboundedSender<crate::app::Action>,
    force: bool,
) {
    if !state.system.state_dirty {
        return;
    }
    if !force && state.system.last_state_save.elapsed() < STATE_SAVE_DEBOUNCE {
        return;
    }

    let notepad_contents = state.notepad_content_for_persistence();
    let json = match persistence::serialize_state(
        &state.data.workspaces,
        &state.data.sessions,
        &notepad_contents,
    ) {
        Ok(json) => json,
        Err(err) => {
            // Clear the flag so a persistent serialization failure doesn't
            // re-toast every frame.
            state.system.state_dirty = false;
            report_persistence_error(state, "failed to serialize state", err);
            return;
        }
    };
    state.system.state_dirty = false;
    state.system.last_state_save = std::time::Instant::now();

    if force {
        if let Err(err) = persistence::write_state_file(&json) {
            crate::logger::warn(format!("failed to save state on shutdown: {err}"));
        }
    } else {
        let tx = action_tx.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(err) = persistence::write_state_file(&json) {
                report_background_error("failed to save state", err);
                let _ = tx.send(crate::app::Action::ShowToast(
                    "Failed to save changes".to_string(),
                    ToastLevel::Error,
                ));
            }
        });
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
