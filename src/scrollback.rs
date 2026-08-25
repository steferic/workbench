//! Durable conversation history read from an agent's own session log.
//!
//! Claude and Codex repaint a fixed viewport at absolute cursor addresses and
//! never scroll the terminal (verified against the wire: no newlines, no
//! scroll-region, no cursor-up), so "what scrolled off the top" is simply not
//! a fact present in the byte stream — it only exists as a difference between
//! two snapshots, which makes any terminal-side scrollback an inference.
//!
//! Both agents do, however, write their full conversation to disk. That log is
//! the deterministic record, so it is what we show when the user scrolls back;
//! the live screen stays the source for what is on screen right now.

use crate::app::{TranscriptLine, TranscriptSpan};
use crate::theme::Theme;
use ratatui::style::{Modifier, Style};
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Which log format a session's history is stored in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Claude,
    Codex,
}

impl LogFormat {
    pub fn for_agent(agent: &crate::models::AgentType) -> Option<Self> {
        if agent.is_terminal() {
            return None;
        }
        // On the command, so a custom agent or a manager wrapping one of these
        // reads back the same way the plain agent does.
        match agent.command() {
            "claude" => Some(LogFormat::Claude),
            "codex" => Some(LogFormat::Codex),
            _ => None,
        }
    }
}

/// Locate the on-disk log for a provider conversation id.
pub fn log_path(format: LogFormat, conversation_id: &str) -> Option<PathBuf> {
    match format {
        LogFormat::Claude => crate::agent_tasks::claude_log_for_session(conversation_id),
        LogFormat::Codex => codex_log_for_session(conversation_id),
    }
}

/// Codex files them under `sessions/YYYY/MM/DD/rollout-<stamp>-<id>.jsonl`.
fn codex_log_for_session(conversation_id: &str) -> Option<PathBuf> {
    let root = dirs::home_dir()?.join(".codex").join("sessions");
    let needle = format!("{conversation_id}.jsonl");
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with(&needle))
                .unwrap_or(false)
            {
                return Some(path);
            }
        }
    }
    None
}

/// One logical (unwrapped) line of history.
type Line = Vec<TranscriptSpan>;

/// Styling by *role*. The log carries no ANSI of its own — unlike the live
/// screen, which arrives pre-coloured — so history would render flat white
/// without this. Roles are known exactly here (the log is structured), which
/// makes the result more consistent than the terminal's own colouring.
#[derive(Clone, Copy)]
pub struct Palette {
    user: Style,
    assistant: Style,
    tool: Style,
    tool_args: Style,
    result: Style,
    heading: Style,
    code: Style,
    marker: Style,
}

impl Palette {
    pub fn from_theme(theme: Theme) -> Self {
        Self {
            user: Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
            assistant: Style::default().fg(theme.fg),
            tool: Style::default().fg(theme.special),
            tool_args: Style::default().fg(theme.fg_dim),
            result: Style::default().fg(theme.fg_faint),
            heading: Style::default()
                .fg(theme.info)
                .add_modifier(Modifier::BOLD),
            code: Style::default().fg(theme.success),
            marker: Style::default().fg(theme.accent),
        }
    }
}

fn span(text: impl Into<String>, style: Style) -> TranscriptSpan {
    TranscriptSpan {
        text: text.into(),
        style,
    }
}

/// Parse a session log into styled display lines, wrapped to `cols`.
pub fn history(format: LogFormat, path: &Path, cols: u16, theme: Theme) -> Vec<TranscriptLine> {
    let palette = Palette::from_theme(theme);
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let mut out: Vec<Line> = Vec::new();
    let mut fenced = false;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match format {
            LogFormat::Claude => push_claude(&mut out, &value, &palette, &mut fenced),
            LogFormat::Codex => push_codex(&mut out, &value, &palette, &mut fenced),
        }
    }
    wrap_lines(out, cols)
}

// ---------------------------------------------------------------------------
// Markdown-ish inline styling for prose
// ---------------------------------------------------------------------------

/// Render one line of assistant prose: fenced code, headings and bullets at
/// line level, `**bold**` and `` `code` `` inline.
fn styled_prose(text: &str, base: Style, palette: &Palette, fenced: &mut bool) -> Line {
    let trimmed = text.trim_start();

    if trimmed.starts_with("```") {
        *fenced = !*fenced;
        return vec![span(text.to_string(), palette.result)];
    }
    if *fenced {
        return vec![span(text.to_string(), palette.code)];
    }
    if trimmed.starts_with('#') {
        return vec![span(text.to_string(), palette.heading)];
    }

    let indent_len = text.len() - trimmed.len();
    let mut out: Line = Vec::new();
    let mut rest = trimmed;
    if let Some(after) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    {
        out.push(span(format!("{}• ", &text[..indent_len]), palette.marker));
        rest = after;
    } else if indent_len > 0 {
        out.push(span(text[..indent_len].to_string(), base));
    }

    out.extend(inline_spans(rest, base, palette));
    out
}

/// Split on `**bold**` and `` `code` `` runs.
fn inline_spans(text: &str, base: Style, palette: &Palette) -> Line {
    let mut out: Line = Vec::new();
    let mut buf = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    let flush = |buf: &mut String, out: &mut Line| {
        if !buf.is_empty() {
            out.push(span(std::mem::take(buf), base));
        }
    };

    while i < chars.len() {
        if chars[i] == '`' {
            if let Some(end) = (i + 1..chars.len()).find(|&j| chars[j] == '`') {
                flush(&mut buf, &mut out);
                let code: String = chars[i + 1..end].iter().collect();
                out.push(span(code, palette.code));
                i = end + 1;
                continue;
            }
        }
        if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            if let Some(end) = (i + 2..chars.len().saturating_sub(1))
                .find(|&j| chars[j] == '*' && chars[j + 1] == '*')
            {
                flush(&mut buf, &mut out);
                let bold: String = chars[i + 2..end].iter().collect();
                out.push(span(bold, base.add_modifier(Modifier::BOLD)));
                i = end + 2;
                continue;
            }
        }
        buf.push(chars[i]);
        i += 1;
    }
    flush(&mut buf, &mut out);
    out
}

// ---------------------------------------------------------------------------
// Claude: {"type":"user"|"assistant","message":{"content": str | [blocks]}}
// ---------------------------------------------------------------------------

fn push_claude(out: &mut Vec<Line>, value: &Value, palette: &Palette, fenced: &mut bool) {
    // Sidechains are subagent conversations; they never appear on this screen.
    if value.get("isSidechain").and_then(Value::as_bool) == Some(true) {
        return;
    }
    let role = value.get("type").and_then(Value::as_str).unwrap_or("");
    let Some(content) = value.get("message").and_then(|m| m.get("content")) else {
        return;
    };

    match role {
        "user" => match content {
            Value::String(text) => push_user(out, text, palette),
            Value::Array(blocks) => {
                for block in blocks {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => push_user(
                            out,
                            block.get("text").and_then(Value::as_str).unwrap_or(""),
                            palette,
                        ),
                        // Tool results come back as user turns; the call itself
                        // is already shown, so only note the outcome.
                        Some("tool_result") => push_tool_result(out, block, palette),
                        _ => {}
                    }
                }
            }
            _ => {}
        },
        "assistant" => {
            if let Value::Array(blocks) = content {
                for block in blocks {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => push_prose(
                            out,
                            block.get("text").and_then(Value::as_str).unwrap_or(""),
                            palette,
                            fenced,
                        ),
                        Some("tool_use") => push_tool_call(
                            out,
                            block.get("name").and_then(Value::as_str).unwrap_or("tool"),
                            &block.get("input").map(summarize_input).unwrap_or_default(),
                            palette,
                        ),
                        // "thinking" blocks are encrypted or empty on disk.
                        _ => {}
                    }
                }
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Codex: {"type":"response_item","payload":{...}}
// ---------------------------------------------------------------------------

fn push_codex(out: &mut Vec<Line>, value: &Value, palette: &Palette, fenced: &mut bool) {
    if value.get("type").and_then(Value::as_str) != Some("response_item") {
        return;
    }
    let Some(payload) = value.get("payload") else {
        return;
    };
    match payload.get("type").and_then(Value::as_str) {
        Some("message") => {
            let role = payload.get("role").and_then(Value::as_str).unwrap_or("");
            // "developer" turns are the injected instruction preamble.
            if role == "developer" {
                return;
            }
            if let Some(blocks) = payload.get("content").and_then(Value::as_array) {
                for block in blocks {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        if role == "user" {
                            push_user(out, text, palette);
                        } else {
                            push_prose(out, text, palette, fenced);
                        }
                    }
                }
            }
        }
        Some("function_call") | Some("custom_tool_call") => {
            let name = payload.get("name").and_then(Value::as_str).unwrap_or("tool");
            let args = payload
                .get("arguments")
                .or_else(|| payload.get("input"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let brief = serde_json::from_str::<Value>(args)
                .map(|v| summarize_input(&v))
                .unwrap_or_else(|_| truncate(args, 100));
            push_tool_call(out, name, &brief, palette);
        }
        Some("function_call_output") | Some("custom_tool_call_output") => {
            if let Some(output) = payload.get("output").and_then(Value::as_str) {
                if let Some(first) = output.lines().find(|l| !l.trim().is_empty()) {
                    out.push(vec![span(
                        format!("  ⎿ {}", truncate(first, 160)),
                        palette.result,
                    )]);
                }
            }
        }
        // "reasoning" summaries are the model's private notes; skip them, as
        // the live view does.
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Emitters
// ---------------------------------------------------------------------------

fn push_user(out: &mut Vec<Line>, text: &str, palette: &Palette) {
    if text.trim().is_empty() {
        return;
    }
    blank_between(out);
    for (idx, line) in text.lines().enumerate() {
        let prefix = if idx == 0 { "› " } else { "  " };
        out.push(vec![span(format!("{prefix}{line}"), palette.user)]);
    }
}

fn push_prose(out: &mut Vec<Line>, text: &str, palette: &Palette, fenced: &mut bool) {
    if text.trim().is_empty() {
        return;
    }
    blank_between(out);
    for line in text.lines() {
        out.push(styled_prose(line, palette.assistant, palette, fenced));
    }
}

fn push_tool_call(out: &mut Vec<Line>, name: &str, args: &str, palette: &Palette) {
    out.push(vec![
        span("⏺ ", palette.marker),
        span(name.to_string(), palette.tool),
        span(format!("({args})"), palette.tool_args),
    ]);
}

fn push_tool_result(out: &mut Vec<Line>, block: &Value, palette: &Palette) {
    let text = match block.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|i| i.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    let mut non_empty = text.lines().filter(|l| !l.trim().is_empty());
    if let Some(first) = non_empty.next() {
        let more = match non_empty.count() {
            0 => String::new(),
            n => format!(" (+{n} lines)"),
        };
        out.push(vec![span(
            format!("  ⎿ {}{}", truncate(first, 160), more),
            palette.result,
        )]);
    }
}

fn blank_between(out: &mut Vec<Line>) {
    if !out.is_empty() {
        out.push(Vec::new());
    }
}

/// A one-line gist of a tool's arguments: the most descriptive field, or the
/// first scalar if none of the usual ones are present.
fn summarize_input(input: &Value) -> String {
    const PREFERRED: [&str; 6] = ["command", "file_path", "path", "pattern", "query", "url"];
    let Some(map) = input.as_object() else {
        return truncate(&input.to_string(), 100);
    };
    for key in PREFERRED {
        if let Some(v) = map.get(key).and_then(Value::as_str) {
            return truncate(v, 100);
        }
    }
    map.values()
        .find_map(Value::as_str)
        .map(|v| truncate(v, 100))
        .unwrap_or_default()
}

fn truncate(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(max_chars).collect();
    format!("{cut}…")
}

// ---------------------------------------------------------------------------
// Wrapping
// ---------------------------------------------------------------------------

/// Wrap to the pane width, preserving each span's style across the break.
/// Log text is unwrapped prose, unlike the screen rows the live view produces.
fn wrap_lines(lines: Vec<Line>, cols: u16) -> Vec<TranscriptLine> {
    let width = (cols as usize).max(20);
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        for wrapped in wrap_one(line, width) {
            out.push(TranscriptLine::from_styled_spans(wrapped));
        }
    }
    out
}

fn wrap_one(line: Line, width: usize) -> Vec<Line> {
    // Flattening to styled characters keeps the break logic independent of
    // where span boundaries happen to fall.
    let chars: Vec<(char, Style)> = line
        .iter()
        .flat_map(|s| s.text.chars().map(move |c| (c, s.style)))
        .collect();
    if chars.len() <= width {
        return vec![line];
    }

    let mut rows: Vec<Line> = Vec::new();
    let mut start = 0usize;
    while start < chars.len() {
        if chars.len() - start <= width {
            rows.push(regroup(&chars[start..]));
            break;
        }
        // Prefer breaking at the last space inside the window.
        let hard = start + width;
        let cut = (start..hard)
            .rev()
            .find(|&i| chars[i].0 == ' ')
            .map(|i| i + 1)
            .filter(|&i| i > start)
            .unwrap_or(hard);
        rows.push(regroup(&chars[start..cut]));
        start = cut;
    }
    rows
}

/// Re-join runs of identically styled characters back into spans.
fn regroup(chars: &[(char, Style)]) -> Line {
    let mut out: Line = Vec::new();
    for (ch, style) in chars {
        match out.last_mut() {
            Some(last) if last.style == *style => last.text.push(*ch),
            _ => out.push(span(ch.to_string(), *style)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> Palette {
        Palette::from_theme(crate::theme::Theme::DARK)
    }

    /// Flatten styled lines back to plain text for content assertions.
    fn texts(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.iter().map(|s| s.text.as_str()).collect::<String>())
            .collect()
    }

    #[test]
    fn claude_user_and_assistant_turns_render_in_order() {
        let (p, mut fenced, mut out) = (palette(), false, Vec::new());
        push_claude(
            &mut out,
            &serde_json::json!({"type":"user","message":{"content":"fix the bug"}}),
            &p,
            &mut fenced,
        );
        push_claude(
            &mut out,
            &serde_json::json!({"type":"assistant","message":{"content":[
                {"type":"thinking","thinking":"secret"},
                {"type":"text","text":"On it."},
                {"type":"tool_use","name":"Read","input":{"file_path":"/tmp/a.rs"}}
            ]}}),
            &p,
            &mut fenced,
        );
        assert_eq!(
            texts(&out),
            vec![
                "› fix the bug".to_string(),
                String::new(),
                "On it.".to_string(),
                "⏺ Read(/tmp/a.rs)".to_string(),
            ]
        );
    }

    /// The whole point of styling by role: history must not render flat.
    #[test]
    fn roles_get_distinct_styles() {
        let (p, mut fenced, mut out) = (palette(), false, Vec::new());
        push_user(&mut out, "hello", &p);
        push_prose(&mut out, "answer", &p, &mut fenced);
        push_tool_call(&mut out, "Bash", "ls", &p);

        let user = out[0][0].style;
        let assistant = out[2][0].style;
        let tool_marker = out[3][0].style;
        let tool_name = out[3][1].style;
        assert_ne!(user, assistant, "user turns must stand out from prose");
        assert_ne!(tool_name, assistant, "tool calls must stand out");
        assert_ne!(tool_marker, tool_name);
        assert!(user.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn markdown_headings_code_and_bold_are_styled() {
        let p = palette();
        let mut fenced = false;

        let heading = styled_prose("## Plan", p.assistant, &p, &mut fenced);
        assert_eq!(heading[0].style, p.heading);

        let inline = styled_prose("use `cargo test` and **stop**", p.assistant, &p, &mut fenced);
        assert!(inline.iter().any(|s| s.text == "cargo test" && s.style == p.code));
        assert!(inline
            .iter()
            .any(|s| s.text == "stop" && s.style.add_modifier.contains(Modifier::BOLD)));

        // Fenced blocks style their whole body, and the fence toggles state.
        let fence = styled_prose("```rust", p.assistant, &p, &mut fenced);
        assert!(fenced, "opening fence enters code mode");
        assert_eq!(fence[0].style, p.result);
        let body = styled_prose("let x = 1; // **not bold**", p.assistant, &p, &mut fenced);
        assert_eq!(body.len(), 1, "code is not inline-parsed");
        assert_eq!(body[0].style, p.code);
        styled_prose("```", p.assistant, &p, &mut fenced);
        assert!(!fenced, "closing fence leaves code mode");
    }

    #[test]
    fn bullets_get_a_marker_span() {
        let p = palette();
        let mut fenced = false;
        let bullet = styled_prose("- first item", p.assistant, &p, &mut fenced);
        assert_eq!(bullet[0].style, p.marker);
        assert!(bullet[0].text.contains('•'));
    }

    #[test]
    fn claude_sidechains_are_skipped() {
        let (p, mut fenced, mut out) = (palette(), false, Vec::new());
        push_claude(
            &mut out,
            &serde_json::json!({"type":"user","isSidechain":true,"message":{"content":"sub"}}),
            &p,
            &mut fenced,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn codex_messages_calls_and_output_render() {
        let (p, mut fenced, mut out) = (palette(), false, Vec::new());
        for value in [
            serde_json::json!({"type":"response_item","payload":{"type":"message","role":"developer",
                "content":[{"type":"input_text","text":"preamble"}]}}),
            serde_json::json!({"type":"response_item","payload":{"type":"message","role":"user",
                "content":[{"type":"input_text","text":"ship it"}]}}),
            serde_json::json!({"type":"response_item","payload":{"type":"function_call",
                "name":"shell_command","arguments":"{\"command\":\"cargo test\"}"}}),
            serde_json::json!({"type":"response_item","payload":{"type":"function_call_output",
                "output":"ok\nmore"}}),
            serde_json::json!({"type":"response_item","payload":{"type":"reasoning",
                "summary":[{"type":"summary_text","text":"private"}]}}),
        ] {
            push_codex(&mut out, &value, &p, &mut fenced);
        }
        assert_eq!(
            texts(&out),
            vec![
                "› ship it".to_string(),
                "⏺ shell_command(cargo test)".to_string(),
                "  ⎿ ok".to_string(),
            ],
            "developer preamble and reasoning must not appear"
        );
    }

    #[test]
    fn wrapping_preserves_styles_across_the_break() {
        let p = palette();
        let line = vec![
            span("⏺ ", p.marker),
            span("Bash", p.tool),
            span("(a very long command line that must wrap somewhere)", p.tool_args),
        ];
        let rows = wrap_one(line, 20);
        assert!(rows.len() > 1, "should have wrapped");
        for row in &rows {
            let width: usize = row.iter().map(|s| s.text.chars().count()).sum();
            assert!(width <= 20, "row too wide: {width}");
        }
        // The continuation keeps the argument styling rather than reverting.
        let last = rows.last().unwrap();
        assert_eq!(last.last().unwrap().style, p.tool_args);
        // No text is lost.
        let joined: String = rows
            .iter()
            .flat_map(|r| r.iter().map(|s| s.text.as_str()))
            .collect();
        assert_eq!(joined.replace(' ', ""), "⏺Bash(averylongcommandlinethatmustwrapsomewhere)");
    }

    #[test]
    fn wrapping_breaks_words_longer_than_the_pane() {
        let p = palette();
        let rows = wrap_one(vec![span("x".repeat(50), p.assistant)], 20);
        assert!(rows.len() >= 3);
        for row in &rows {
            assert!(row.iter().map(|s| s.text.chars().count()).sum::<usize>() <= 20);
        }
    }
}

