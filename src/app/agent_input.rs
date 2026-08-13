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
///
/// Sent as a bracketed paste, which is the terminal's way of saying "this is
/// content, not keys". Without it a message is fed to the agent one keystroke
/// at a time, and a leading character the agent has bound to something else
/// never reaches the composer at all: a bare `?` opens Claude's shortcuts
/// overlay and is swallowed, so the Enter that follows submits nothing. `/`,
/// `!` and `#` are the same story. Verified against Claude 2.1.220 and Codex
/// 0.146 — `?` typed raw disappears, `?` pasted is answered.
pub fn submit_text(action_tx: &mpsc::UnboundedSender<Action>, session_id: Uuid, text: &str) {
    if action_tx
        .send(Action::SendInput(session_id, bracketed(text)))
        .is_err()
    {
        return;
    }
    submit(action_tx, session_id);
}

/// Wrap text in the paste markers, with any end-marker inside it removed —
/// otherwise a message could close its own paste and have the rest read as
/// keystrokes. Used for every paste workbench forwards, including the user's
/// own clipboard.
pub(crate) fn bracketed(text: &str) -> Vec<u8> {
    const START: &[u8] = b"\x1b[200~";
    const END: &[u8] = b"\x1b[201~";

    let cleaned = text.replace("\x1b[201~", "");
    let mut out = Vec::with_capacity(cleaned.len() + START.len() + END.len());
    out.extend_from_slice(START);
    out.extend_from_slice(cleaned.as_bytes());
    out.extend_from_slice(END);
    out
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
                assert_eq!(
                    String::from_utf8(bytes).unwrap(),
                    "\x1b[200~fix the redirect\x1b[201~"
                );
            }
            other => panic!("expected the text first, got {other:?}"),
        }
        match rx.try_recv().unwrap() {
            Action::SendInput(_, bytes) => assert_eq!(bytes, vec![b'\r']),
            other => panic!("expected Enter on its own, got {other:?}"),
        }
    }

    /// The failure this guards: typed one key at a time, a leading `?` is
    /// eaten by Claude's shortcuts overlay and never reaches the composer, so
    /// the Enter after it submits an empty prompt and the message is simply
    /// lost. A paste is content, and arrives whole.
    #[test]
    fn a_message_that_starts_with_a_hotkey_still_arrives() {
        for text in ["?", "/status", "!ls", "#remember this"] {
            let wrapped = String::from_utf8(bracketed(text)).unwrap();
            assert_eq!(wrapped, format!("\x1b[200~{text}\x1b[201~"));
        }
    }

    #[test]
    fn a_message_cannot_close_its_own_paste() {
        let sneaky = "innocent\x1b[201~?then keys";
        let wrapped = String::from_utf8(bracketed(sneaky)).unwrap();
        assert_eq!(wrapped, "\x1b[200~innocent?then keys\x1b[201~");
        assert_eq!(wrapped.matches("\x1b[201~").count(), 1, "one end marker only");
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
