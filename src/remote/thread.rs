//! The conversation, read from the agent's own journal.
//!
//! The phone's first version showed the tail of the terminal: box-drawing
//! characters, spinners, token counters, a status line — all shrunk to fit a
//! phone. It was legible in the way a screenshot of a terminal is legible.
//!
//! Both Claude and Codex journal every turn to JSONL, and workbench already
//! knows which file belongs to which session (see `agent_tasks::locate`). So
//! the phone can have what it actually wants: your messages, the agent's
//! replies, and a one-line trace of the tools in between.
//!
//! Providers with no readable journal keep the terminal tail — worse, but
//! better than nothing.

use serde::Serialize;
use serde_json::Value;
use std::path::Path;

use crate::agent_tasks::Provider;

/// Who said it. `tool` is not speech — it is the compact "▸ Bash · cargo test"
/// line that stops a conversation from looking like it skipped a beat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    You,
    Agent,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Message {
    pub role: Role,
    pub text: String,
    /// RFC3339 as the provider wrote it, for the time separators.
    pub at: Option<String>,
}

/// How far back a first read goes. Turns are appended, so the end of the file
/// is the recent conversation; a phone has no use for the start of a long
/// session and no wish to wait for it to be parsed. Generous because it is
/// paid once — after that only new bytes are read.
///
/// Codex needs the room: it journals encrypted reasoning blobs between turns,
/// so a megabyte of rollout can hold only a handful of spoken messages.
const TAIL_BYTES: u64 = 4 * 1024 * 1024;

/// Where a reader has got to, so the next pass costs only what was appended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cursor(pub u64);

/// Read whatever is new, appending to `out`.
///
/// A fresh cursor starts `TAIL_BYTES` from the end. An existing one resumes
/// where the last pass stopped, which is what keeps this affordable at a tick
/// a second: an agent mid-turn appends a few kilobytes, not a megabyte.
pub fn read_more(path: &Path, provider: Provider, from: Cursor, out: &mut Vec<Message>) -> Cursor {
    use std::fs::File;
    use std::io::{BufRead, BufReader, Seek, SeekFrom};

    let Ok(file) = File::open(path) else {
        return from;
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut reader = BufReader::new(file);

    // Where to start: the tail on a first pass, otherwise where we stopped. A
    // file shorter than the cursor was replaced underneath us.
    let mut consumed = match from {
        Cursor(0) => len.saturating_sub(TAIL_BYTES),
        Cursor(at) if at <= len => at,
        _ => {
            out.clear();
            len.saturating_sub(TAIL_BYTES)
        }
    };
    if reader.seek(SeekFrom::Start(consumed)).is_err() {
        return from;
    }
    if from == Cursor(0) && consumed > 0 {
        // Landing mid-line is expected; that line belongs to the part we chose
        // not to read.
        let mut partial = String::new();
        consumed += reader.read_line(&mut partial).unwrap_or(0) as u64;
    }

    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            // A line without its newline is a half-written record; leave it for
            // the next pass rather than parsing a truncated one.
            Ok(_) if !line.ends_with('\n') => break,
            Ok(n) => {
                consumed += n as u64;
                let Ok(value) = serde_json::from_str::<Value>(line.trim_end()) else {
                    continue;
                };
                match provider {
                    Provider::Claude => claude(&value, out),
                    Provider::Codex => codex(&value, out),
                    _ => {}
                }
            }
            Err(_) => break,
        }
    }
    Cursor(consumed)
}

/// One-shot read of a session's recent conversation. Production always reads
/// incrementally; this is the same thing with the bookkeeping inlined.
#[cfg(test)]
pub fn read(path: &Path, provider: Provider, limit: usize) -> Vec<Message> {
    let mut out = Vec::new();
    read_more(path, provider, Cursor::default(), &mut out);
    if out.len() > limit {
        out.drain(..out.len() - limit);
    }
    out
}

fn push(out: &mut Vec<Message>, role: Role, text: &str, at: Option<&str>) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    // Whole, however long it is. A message used to be cut at 2000 characters on
    // the theory that a pasted file is not worth reading on a phone — but the
    // phone is where you read the conversation when you are not at the desk,
    // and a reply that stops mid-sentence under an ellipsis is worse than a
    // long one you can scroll. What bounds the payload is `TAIL_BYTES`, which
    // is how much journal a first read parses at all; after that a pass carries
    // only what was appended, and `?have=` means each message crosses once.
    out.push(Message {
        role,
        text: text.to_string(),
        at: at.map(str::to_string),
    });
}

/// Harness plumbing that Claude stores as if it were your message: system
/// reminders, slash-command expansions, captured stdout. You did not say it,
/// so it is not in the conversation.
fn is_plumbing(text: &str) -> bool {
    text.trim_start().starts_with('<')
}

// ---------------------------------------------------------------------------
// Claude Code
// ---------------------------------------------------------------------------

fn claude(v: &Value, out: &mut Vec<Message>) {
    // A subagent's transcript shares the file. Its chatter is not this
    // conversation.
    if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
        return;
    }
    let at = v.get("timestamp").and_then(Value::as_str);

    match v.get("type").and_then(Value::as_str) {
        Some("user") => {
            if v.get("isMeta").and_then(Value::as_bool) == Some(true) {
                return;
            }
            let Some(content) = v.pointer("/message/content") else {
                return;
            };
            if let Some(text) = content.as_str() {
                if !is_plumbing(text) {
                    push(out, Role::You, text, at);
                }
                return;
            }
            for block in content.as_array().into_iter().flatten() {
                if block.get("type").and_then(Value::as_str) != Some("text") {
                    continue;
                }
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    if !is_plumbing(text) {
                        push(out, Role::You, text, at);
                    }
                }
            }
        }
        Some("assistant") => {
            let Some(blocks) = v.pointer("/message/content").and_then(Value::as_array) else {
                return;
            };
            for block in blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(text) = block.get("text").and_then(Value::as_str) {
                            push(out, Role::Agent, text, at);
                        }
                    }
                    Some("tool_use") => {
                        let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                        push(out, Role::Tool, &tool_line(short_name(name), block.get("input")), at);
                    }
                    // Thinking is the agent talking to itself, and it is long.
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// `mcp__plugin_playwright_playwright__browser_click` → `browser_click`. The
/// server prefix identifies the plumbing, not the step.
fn short_name(name: &str) -> &str {
    name.rsplit("__").next().unwrap_or(name)
}

/// `Bash · cargo test --lib` — the tool and the one detail that says which
/// call this was.
fn tool_line(name: &str, input: Option<&Value>) -> String {
    const DETAILS: [&str; 8] = [
        "command",
        "file_path",
        "pattern",
        "subject",
        "url",
        "query",
        "path",
        "description",
    ];
    let detail = input.and_then(|input| {
        DETAILS
            .iter()
            .find_map(|key| input.get(key).and_then(Value::as_str))
    });
    match detail {
        Some(detail) => {
            let first = detail.lines().next().unwrap_or(detail).trim();
            format!("{name} · {first}")
        }
        None => name.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Codex
// ---------------------------------------------------------------------------

fn codex(v: &Value, out: &mut Vec<Message>) {
    let at = v.get("timestamp").and_then(Value::as_str);
    let Some(payload) = v.get("payload") else {
        return;
    };
    let kind = payload.get("type").and_then(Value::as_str);

    match (v.get("type").and_then(Value::as_str), kind) {
        // The event stream carries the words plainly. The matching
        // `response_item` entries carry the same text wrapped in environment
        // context, so reading events avoids showing you a prompt preamble you
        // never wrote.
        (Some("event_msg"), Some("user_message")) => {
            if let Some(text) = payload.get("message").and_then(Value::as_str) {
                if !is_plumbing(text) {
                    push(out, Role::You, text, at);
                }
            }
        }
        (Some("event_msg"), Some("agent_message")) => {
            if let Some(text) = payload.get("message").and_then(Value::as_str) {
                push(out, Role::Agent, text, at);
            }
        }
        (Some("response_item"), Some("function_call")) => {
            let name = payload.get("name").and_then(Value::as_str).unwrap_or("tool");
            // `arguments` is JSON in a string.
            let args = payload
                .get("arguments")
                .and_then(Value::as_str)
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
            push(out, Role::Tool, &codex_tool_line(name, args.as_ref()), at);
        }
        (Some("response_item"), Some("custom_tool_call")) => {
            let name = payload.get("name").and_then(Value::as_str).unwrap_or("tool");
            let input = payload.get("input").and_then(Value::as_str);
            let line = match input {
                Some(input) => format!("{name} · {}", input.lines().next().unwrap_or("").trim()),
                None => name.to_string(),
            };
            push(out, Role::Tool, &line, at);
        }
        _ => {}
    }
}

fn codex_tool_line(name: &str, args: Option<&Value>) -> String {
    const DETAILS: [&str; 5] = ["cmd", "command", "path", "file_path", "query"];
    let detail = args.and_then(|args| {
        DETAILS.iter().find_map(|key| match args.get(key) {
            Some(Value::String(s)) => Some(s.clone()),
            // exec takes an argv array on some versions.
            Some(Value::Array(parts)) => Some(
                parts
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            _ => None,
        })
    });
    match detail {
        Some(detail) => format!("{name} · {}", detail.lines().next().unwrap_or("").trim()),
        None => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_log(lines: &[&str]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
        file.flush().unwrap();
        file
    }

    /// Shapes taken from a real `~/.claude/projects/**/*.jsonl`.
    #[test]
    fn a_claude_session_reads_as_a_conversation() {
        let file = write_log(&[
            r#"{"type":"user","timestamp":"2026-08-03T10:00:00Z","message":{"role":"user","content":"fix the redirect"}}"#,
            r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"caveat: this is a meta line"}}"#,
            r#"{"type":"user","message":{"role":"user","content":"<system-reminder>ignore me</system-reminder>"}}"#,
            r#"{"type":"assistant","timestamp":"2026-08-03T10:00:05Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"long private reasoning"},{"type":"text","text":"Looking at the router now."}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":{"command":"cargo test --lib\nsecond line","description":"run tests"}}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"x","content":"ok"}]}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"role":"assistant","content":[{"type":"text","text":"subagent chatter"}]}}"#,
        ]);

        let messages = read(file.path(), Provider::Claude, 50);

        assert_eq!(
            messages,
            vec![
                Message {
                    role: Role::You,
                    text: "fix the redirect".into(),
                    at: Some("2026-08-03T10:00:00Z".into())
                },
                Message {
                    role: Role::Agent,
                    text: "Looking at the router now.".into(),
                    at: Some("2026-08-03T10:00:05Z".into())
                },
                Message {
                    role: Role::Tool,
                    text: "Bash · cargo test --lib".into(),
                    at: None
                },
            ],
            "meta lines, system reminders, thinking, tool results and subagent \
             chatter are not part of the conversation"
        );
    }

    /// Shapes taken from a real `~/.codex/sessions/**/rollout-*.jsonl`.
    #[test]
    fn a_codex_rollout_reads_as_a_conversation() {
        let file = write_log(&[
            r#"{"type":"event_msg","timestamp":"2026-08-03T10:00:00Z","payload":{"type":"user_message","message":"build the radio case"}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>noise</environment_context>"}]}}"#,
            r#"{"type":"event_msg","payload":{"type":"agent_message","message":"I'll inspect the app structure first."}}"#,
            r#"{"type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"pwd && ls\",\"workdir\":\"/tmp\"}"}}"#,
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{}}}"#,
            r#"{"type":"response_item","payload":{"type":"reasoning","encrypted_content":"…"}}"#,
        ]);

        let messages = read(file.path(), Provider::Codex, 50);

        assert_eq!(messages.len(), 3, "{messages:?}");
        assert_eq!(messages[0].role, Role::You);
        assert_eq!(messages[0].text, "build the radio case");
        assert_eq!(messages[1].text, "I'll inspect the app structure first.");
        assert_eq!(messages[2].text, "exec_command · pwd && ls");
    }

    #[test]
    fn only_the_recent_end_of_a_long_session_travels() {
        let lines: Vec<String> = (0..200)
            .map(|i| {
                format!(
                    r#"{{"type":"user","message":{{"role":"user","content":"message {i}"}}}}"#
                )
            })
            .collect();
        let file = write_log(&lines.iter().map(String::as_str).collect::<Vec<_>>());

        let messages = read(file.path(), Provider::Claude, 5);

        assert_eq!(messages.len(), 5);
        assert_eq!(messages[4].text, "message 199", "the newest is the last");
        assert_eq!(messages[0].text, "message 195");
    }

    /// The tick runs once a second against a journal that can be megabytes.
    /// Only the new bytes may be parsed, and a record still being written must
    /// wait for its newline rather than be parsed in half.
    #[test]
    fn a_second_pass_reads_only_what_was_appended() {
        use std::io::Write;
        let mut file = write_log(&[
            r#"{"type":"user","message":{"role":"user","content":"first"}}"#,
        ]);

        let mut messages = Vec::new();
        let cursor = read_more(file.path(), Provider::Claude, Cursor::default(), &mut messages);
        assert_eq!(messages.len(), 1);

        // A half-written record: the cursor must not advance past it.
        write!(file, r#"{{"type":"user","message":{{"role":"user","content":"second"}}}}"#).unwrap();
        file.flush().unwrap();
        let partial = read_more(file.path(), Provider::Claude, cursor, &mut messages);
        assert_eq!(partial, cursor, "a line without its newline is not consumed");
        assert_eq!(messages.len(), 1);

        writeln!(file).unwrap();
        file.flush().unwrap();
        let after = read_more(file.path(), Provider::Claude, partial, &mut messages);
        assert!(after.0 > cursor.0);
        assert_eq!(
            messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
            vec!["first", "second"],
            "the first message is kept, not read again"
        );
    }

    #[test]
    fn mcp_tools_lose_the_server_plumbing_in_their_name() {
        assert_eq!(short_name("mcp__plugin_playwright_playwright__browser_click"), "browser_click");
        assert_eq!(short_name("Bash"), "Bash");
    }

    /// A wall of text arrives whole. The phone is where the conversation gets
    /// read away from the desk, so a reply that stops under an ellipsis is a
    /// reply you have to go to the desk to finish.
    #[test]
    fn a_wall_of_text_arrives_whole() {
        let long = "x".repeat(40_000);
        let file = write_log(&[&format!(
            r#"{{"type":"user","message":{{"role":"user","content":"{long}"}}}}"#
        )]);

        let messages = read(file.path(), Provider::Claude, 5);
        assert_eq!(messages[0].text, long, "the message was cut");
        assert!(!messages[0].text.ends_with('…'));
    }

    /// Multi-byte text is not cut either — the old cap counted characters, so
    /// nothing here should be able to split one.
    #[test]
    fn a_long_message_of_wide_characters_survives_intact() {
        let long = "日本語のテキスト".repeat(500);
        let file = write_log(&[&format!(
            r#"{{"type":"user","message":{{"role":"user","content":"{long}"}}}}"#
        )]);

        let messages = read(file.path(), Provider::Claude, 5);
        assert_eq!(messages[0].text, long);
    }
}
