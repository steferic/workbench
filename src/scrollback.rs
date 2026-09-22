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
mod render;
use crate::theme::Theme;
use ratatui::style::{Modifier, Style};
use render::Block;
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

#[derive(Clone, Debug)]
pub struct Document {
    blocks: Vec<Block>,
}

impl Document {
    pub fn render(&self, cols: u16) -> Vec<TranscriptLine> {
        render::layout(&self.blocks, cols)
    }
}

#[derive(Clone, Debug)]
pub struct History {
    pub document: std::sync::Arc<Document>,
    pub lines: Vec<TranscriptLine>,
}

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
            user: Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
            assistant: Style::default().fg(theme.fg),
            tool: Style::default().fg(theme.special),
            tool_args: Style::default().fg(theme.fg_dim),
            result: Style::default().fg(theme.fg_faint),
            heading: Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
            code: Style::default().fg(theme.success),
            marker: Style::default().fg(theme.accent),
        }
    }
}

fn span(text: impl Into<String>, style: Style) -> TranscriptSpan {
    TranscriptSpan {
        link: None,
        text: text.into(),
        style,
    }
}

/// Capture parsed terminal cells, joining only rows marked as soft wraps.
/// Hard newlines, ANSI styles and OSC hyperlinks remain part of the history.
pub fn terminal_rows(screen: &vt100::Screen) -> Vec<TranscriptLine> {
    use std::hash::{Hash, Hasher};
    let mut blocks = Vec::new();
    let mut line: Line = Vec::new();
    for (mut cells, wrapped) in screen.history_rows() {
        if wrapped && cells.last().is_some_and(|c| *c == vt100::Cell::default()) {
            cells.pop();
        }
        if !wrapped {
            while cells
                .last()
                .is_some_and(|c| !c.has_contents() && !c.is_wide_continuation())
            {
                cells.pop();
            }
        }
        for cell in cells.iter().filter(|c| !c.is_wide_continuation()) {
            let style = crate::tui::utils::convert_vt100_cell_style(cell);
            let link = cell.hyperlink().map(str::to_owned);
            let text = if cell.has_contents() {
                cell.contents()
            } else {
                " ".into()
            };
            match line.last_mut() {
                Some(last) if last.style == style && last.link == link => last.text.push_str(&text),
                _ => line.push(TranscriptSpan { text, style, link }),
            }
        }
        if !wrapped {
            blocks.push(Block::Literal(std::mem::take(&mut line)));
        }
    }
    if !line.is_empty() {
        blocks.push(Block::Literal(line));
    }
    while matches!(blocks.last(), Some(Block::Literal(line)) if line.is_empty()) {
        blocks.pop();
    }
    let ids: Vec<_> = blocks
        .iter()
        .map(|block| {
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            if let Block::Literal(line) = block {
                for span in line {
                    span.text.hash(&mut hash);
                }
            }
            hash.finish() as usize
        })
        .collect();
    let mut lines = render::layout(&blocks, screen.size().1);
    for line in &mut lines {
        if let Some((block, offset)) = line.anchor {
            line.anchor = Some((ids[block], offset));
        }
    }
    lines
}

/// Parse a session log into styled display lines, wrapped to `cols`.
pub fn history(format: LogFormat, path: &Path, cols: u16, theme: Theme) -> Result<History, String> {
    let palette = Palette::from_theme(theme);
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut out: Vec<Block> = Vec::new();
    let mut fenced = false;
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|e| e.to_string())?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match format {
            LogFormat::Claude => push_claude(&mut out, &value, &palette, &mut fenced),
            LogFormat::Codex => push_codex(&mut out, &value, &palette, &mut fenced),
        }
    }
    let document = std::sync::Arc::new(Document { blocks: out });
    Ok(History {
        lines: document.render(cols),
        document,
    })
}

// ---------------------------------------------------------------------------
// Claude: {"type":"user"|"assistant","message":{"content": str | [blocks]}}
// ---------------------------------------------------------------------------

fn push_claude(out: &mut Vec<Block>, value: &Value, palette: &Palette, fenced: &mut bool) {
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

fn push_codex(out: &mut Vec<Block>, value: &Value, palette: &Palette, fenced: &mut bool) {
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
            if !matches!(role, "user" | "assistant") {
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
            let name = payload
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("tool");
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
                push_result(out, output, palette);
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

fn push_user(out: &mut Vec<Block>, text: &str, palette: &Palette) {
    if text.trim().is_empty() {
        return;
    }
    blank_between(out);
    for (idx, line) in text.lines().enumerate() {
        let prefix = if idx == 0 { "› " } else { "  " };
        out.push(Block::Flow(
            vec![span(format!("{prefix}{line}"), palette.user)],
            2,
        ));
    }
}

fn push_prose(out: &mut Vec<Block>, text: &str, palette: &Palette, fenced: &mut bool) {
    if text.trim().is_empty() {
        return;
    }
    blank_between(out);
    let _ = fenced;
    out.extend(render::prose(text, palette));
}

fn push_tool_call(out: &mut Vec<Block>, name: &str, args: &str, palette: &Palette) {
    out.push(Block::Flow(
        vec![
            span("⏺ ", palette.marker),
            span(name.to_string(), palette.tool),
            span(format!("({args})"), palette.tool_args),
        ],
        2,
    ));
}

fn push_tool_result(out: &mut Vec<Block>, block: &Value, palette: &Palette) {
    let text = match block.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|i| i.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    push_result(out, &text, palette);
}

fn push_result(out: &mut Vec<Block>, text: &str, palette: &Palette) {
    for (i, line) in text.lines().enumerate() {
        out.push(Block::Literal(vec![span(
            format!(
                "{}{}",
                if i == 0 { "  ⎿ " } else { "    " },
                line.replace('\t', "    ")
            ),
            palette.result,
        )]));
    }
}

fn blank_between(out: &mut Vec<Block>) {
    if !out.is_empty() {
        out.push(Block::Flow(Vec::new(), 0));
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

#[cfg(test)]
mod tests;
