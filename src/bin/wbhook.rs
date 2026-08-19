//! The hook fast path: one line to workbench's control socket, then exit.
//!
//! Agents run a hook command on every lifecycle event — measured at ~1,450
//! invocations a minute across a busy set of them. Each one used to exec the
//! whole `workbench` binary, which links a TUI, an async runtime, SQLite and an
//! audio stack in order to write one small file, and which the kernel warned
//! about every time: loading it "increases system memory footprint almost
//! permanently".
//!
//! So this is deliberately its own program, and deliberately tiny: no
//! dependencies, no JSON library, no runtime. It reads three environment
//! variables workbench already injects into every pane, forwards the payload,
//! and gets out of the way. Interpreting the event stays in workbench, where it
//! was.
//!
//! Everything here fails silently. A hook is a side channel: if workbench is
//! not running, or the socket has gone, the agent must carry on regardless —
//! and it is the agent's own stderr that noise would land in.

#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;

/// Escape a string for a JSON scalar. Hand-rolled to keep this binary free of
/// serde: the payload is arbitrary agent output, so the control characters do
/// have to be handled, but that is twenty lines rather than a dependency tree.
fn escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 16);
    for c in raw.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Windows has no control socket to forward to — `control::start` says so
/// outright there — so the forwarder is a no-op rather than a build error.
/// It still exists as a program so the hook command generated for an agent
/// does not have to differ by platform.
#[cfg(not(unix))]
fn main() {}

#[cfg(unix)]
fn main() {
    let (Ok(socket), Ok(session), Ok(workspace)) = (
        std::env::var("WORKBENCH_CONTROL_SOCK"),
        std::env::var("WORKBENCH_SESSION"),
        std::env::var("WORKBENCH_WORKSPACE"),
    ) else {
        return; // not running inside a workbench pane
    };

    // Codex's hook command takes no arguments and names the event in the
    // payload instead; Claude passes it here. Both shapes are handled on the
    // other side, so an empty event is fine to send.
    let event = std::env::args().nth(1).unwrap_or_default();

    let mut payload = String::new();
    let _ = std::io::stdin().read_to_string(&mut payload);

    let line = format!(
        r#"{{"id":0,"method":"hook","params":{{"workspace":"{}","session":"{}","event":"{}","payload":"{}"}}}}"#,
        escape(&workspace),
        escape(&session),
        escape(&event),
        escape(payload.trim()),
    );

    if let Ok(mut stream) = UnixStream::connect(&socket) {
        let _ = stream.write_all(line.as_bytes());
        let _ = stream.write_all(b"\n");
        let _ = stream.flush();
        // Not waiting for the reply: nothing here acts on it, and a hook that
        // blocks is a hook that slows the agent down.
    }
}

#[cfg(test)]
mod tests {
    use super::escape;

    /// The one piece of hand-rolled parsing in the hot path. Agent payloads
    /// carry tool arguments and prose, so quotes, backslashes and newlines are
    /// routine — and a malformed line would be dropped by the server as bad
    /// JSON, losing the status silently rather than loudly.
    #[test]
    fn payloads_survive_being_escaped() {
        assert_eq!(escape(r#"say "hi""#), r#"say \"hi\""#);
        assert_eq!(escape(r"C:\path"), r"C:\\path");
        assert_eq!(escape("two\nlines\there"), "two\\nlines\\there");
        assert_eq!(escape("bell\u{7}"), "bell\\u0007");
        assert_eq!(escape("ünïcode ✓"), "ünïcode ✓");
        assert_eq!(escape(""), "");
    }

    /// What the server actually has to parse.
    #[test]
    fn an_escaped_payload_is_valid_json() {
        let nasty = "tool: \"grep\"\n\targs: C:\\x \u{1}";
        let line = format!(r#"{{"payload":"{}"}}"#, escape(nasty));
        let parsed: serde_json::Value = serde_json::from_str(&line).expect("valid JSON");
        assert_eq!(parsed["payload"], nasty);
    }
}
