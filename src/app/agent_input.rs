//! Typing into an agent on its behalf.
//!
//! Sending text and its Enter back-to-back does not work: the two writes land
//! in one `read()` on the far side, and a full-screen agent (Claude's Ink UI,
//! Codex's TUI) treats a newline inside one chunk as *part of the paste*. The
//! text appears in the composer and simply sits there, which looks exactly
//! like workbench doing nothing.
//!
//! So the Enter goes as its own write, a beat later. Every path that submits
//! on your behalf — the TODO queue, the phone's reply box — goes through here.

use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::app::Action;

/// Long enough for the agent's input handling to settle after the paste,
/// short enough to feel immediate. Claude and Codex both accept the Enter
/// reliably at this spacing.
pub const SUBMIT_DELAY: Duration = Duration::from_millis(150);

/// Type `text` into a session and submit it.
pub fn submit_text(action_tx: &mpsc::UnboundedSender<Action>, session_id: Uuid, text: &str) {
    if action_tx
        .send(Action::SendInput(session_id, text.as_bytes().to_vec()))
        .is_err()
    {
        return;
    }
    submit(action_tx, session_id);
}

/// Press Enter on a session, after the pause that makes it register.
pub fn submit(action_tx: &mpsc::UnboundedSender<Action>, session_id: Uuid) {
    press_after(action_tx, session_id, vec![b'\r'], SUBMIT_DELAY);
}

/// Send a key on its own. Used by the phone's Approve (Enter) and Deny (Esc),
/// where the pause matters just as much: an approval that arrives glued to
/// something else is read as text.
pub fn press_after(
    action_tx: &mpsc::UnboundedSender<Action>,
    session_id: Uuid,
    bytes: Vec<u8>,
    delay: Duration,
) {
    let tx = action_tx.clone();
    // Outside a runtime (tests) there is nothing to schedule on, and no
    // reason to wait — send it straight away.
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        let _ = tx.send(Action::SendInput(session_id, bytes));
        return;
    };
    handle.spawn(async move {
        tokio::time::sleep(delay).await;
        if let Err(err) = tx.send(Action::SendInput(session_id, bytes)) {
            crate::logger::warn(format!("could not submit to {session_id}: {err}"));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_its_enter_are_separate_writes() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        submit_text(&tx, Uuid::new_v4(), "fix the redirect");

        // The bug this guards: one write of "text\r" leaves the text sitting
        // in the composer, because the agent reads it as a pasted newline.
        match rx.try_recv().unwrap() {
            Action::SendInput(_, bytes) => {
                assert_eq!(String::from_utf8(bytes).unwrap(), "fix the redirect");
            }
            other => panic!("expected the text first, got {other:?}"),
        }
        match rx.try_recv().unwrap() {
            Action::SendInput(_, bytes) => assert_eq!(bytes, vec![b'\r']),
            other => panic!("expected Enter on its own, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn inside_a_runtime_the_enter_is_delayed_but_still_arrives() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let session = Uuid::new_v4();
        submit_text(&tx, session, "hello");

        assert!(matches!(rx.try_recv(), Ok(Action::SendInput(_, _))), "text goes immediately");
        assert!(
            rx.try_recv().is_err(),
            "the Enter must not ride along with the text"
        );

        tokio::time::sleep(SUBMIT_DELAY * 2).await;
        match rx.try_recv().unwrap() {
            Action::SendInput(id, bytes) => {
                assert_eq!(id, session);
                assert_eq!(bytes, vec![b'\r']);
            }
            other => panic!("expected the delayed Enter, got {other:?}"),
        }
    }
}
