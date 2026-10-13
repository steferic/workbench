//! Reading Claude Code and Codex transcripts into the word counts.
//!
//! Each file is read from where the last scan stopped, under one write
//! transaction that also moves its offset, so a scan that dies halfway counts
//! nothing twice and a second scanner waits rather than overlapping.

use super::tokens::{agent_words, typed_words};
use super::{open, AGENT, YOU};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Claude,
    Codex,
}

/// Count everything appended to the transcripts since the last scan.
/// `progress` hears bytes read so far and bytes to read in all.
pub fn scan(db: &Path, mut progress: impl FnMut(u64, u64)) -> Result<()> {
    let projects = Projects::load();
    let mut connection = open(db)?;

    let known: HashMap<String, u64> = connection
        .prepare("SELECT path, offset FROM files")?
        .query_map([], |row| Ok((row.get(0)?, row.get::<_, i64>(1)? as u64)))?
        .filter_map(Result::ok)
        .collect();
    let mut pending = Vec::new();
    for (path, source) in discover(&projects) {
        let Ok(size) = std::fs::metadata(&path).map(|m| m.len()) else {
            continue;
        };
        let offset = known
            .get(path.to_string_lossy().as_ref())
            .copied()
            .unwrap_or(0);
        if size != offset {
            pending.push((path, source, size.saturating_sub(offset)));
        }
    }

    let of: u64 = pending.iter().map(|(_, _, bytes)| bytes).sum();
    let mut done = 0;
    progress(done, of);
    for (path, source, bytes) in pending {
        if let Err(err) = scan_file(&mut connection, &path, source, &projects) {
            crate::logger::warn(format!("word count: {}: {err:#}", path.display()));
        }
        done += bytes;
        progress(done, of);
    }
    Ok(())
}

/// What a scan needs to carry from one read of a file to the next.
#[derive(Debug)]
struct FileState {
    offset: u64,
    project: Option<String>,
    provider: String,
    /// The model last seen in the file. Your words count against the model
    /// you were talking to.
    model: String,
    /// False for a conversation no person typed into: `codex exec`, a
    /// sub-agent, a scripted `claude -p`.
    typed: bool,
}

type Tally = HashMap<(&'static str, String), HashMap<String, u64>>;

fn scan_file(
    connection: &mut Connection,
    path: &Path,
    source: Source,
    projects: &Projects,
) -> Result<()> {
    let key = path.to_string_lossy().to_string();
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut state = tx
        .query_row(
            "SELECT offset, project, provider, model, typed FROM files WHERE path = ?1",
            [&key],
            |row| {
                Ok(FileState {
                    offset: row.get::<_, i64>(0)? as u64,
                    project: row.get(1)?,
                    provider: row.get(2)?,
                    model: row.get(3)?,
                    typed: row.get(4)?,
                })
            },
        )
        .optional()?
        .unwrap_or(FileState {
            offset: 0,
            project: None,
            provider: match source {
                Source::Claude => "anthropic",
                Source::Codex => "openai",
            }
            .to_string(),
            model: String::new(),
            typed: true,
        });

    let size = std::fs::metadata(path)?.len();
    let mut tally = Tally::new();
    if state.project.as_deref() == Some("") || state.offset > size {
        // Not one of your projects, or rewritten shorter: nothing to read.
        state.offset = size;
    } else if state.offset < size {
        let mut file = std::fs::File::open(path)?;
        file.seek(SeekFrom::Start(state.offset))?;
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        loop {
            line.clear();
            let read = reader.read_until(b'\n', &mut line)?;
            // Stop before a line the agent is still writing.
            if read == 0 || line.last() != Some(&b'\n') {
                break;
            }
            state.offset += read as u64;
            if let Ok(text) = std::str::from_utf8(&line) {
                match source {
                    Source::Claude => claude_line(text, &mut state, projects, &mut tally),
                    Source::Codex => codex_line(text, &mut state, projects, &mut tally),
                }
            }
            if state.project.as_deref() == Some("") {
                state.offset = size.max(state.offset);
                tally.clear();
                break;
            }
        }
    }

    if let Some(project) = state.project.as_deref().filter(|p| !p.is_empty()) {
        let mut add_count = tx.prepare_cached(
            "INSERT INTO counts (project, provider, model, speaker, word, n)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT DO UPDATE SET n = n + excluded.n",
        )?;
        let mut add_total = tx.prepare_cached(
            "INSERT INTO totals (project, provider, model, speaker, n)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT DO UPDATE SET n = n + excluded.n",
        )?;
        for ((speaker, model), words) in &tally {
            let mut sum = 0;
            for (word, n) in words {
                add_count.execute(params![
                    project,
                    state.provider,
                    model,
                    speaker,
                    word,
                    *n as i64
                ])?;
                sum += n;
            }
            add_total.execute(params![project, state.provider, model, speaker, sum as i64])?;
        }
    }
    tx.execute(
        "INSERT INTO files (path, offset, project, provider, model, typed)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (path) DO UPDATE SET offset = excluded.offset,
             project = excluded.project, model = excluded.model, typed = excluded.typed",
        params![
            key,
            state.offset as i64,
            state.project,
            state.provider,
            state.model,
            state.typed
        ],
    )?;
    tx.commit()?;
    Ok(())
}

fn add(tally: &mut Tally, speaker: &'static str, model: &str, words: Vec<String>) {
    if words.is_empty() {
        return;
    }
    let counts = tally.entry((speaker, model.to_string())).or_default();
    for word in words {
        *counts.entry(word).or_default() += 1;
    }
}

/// One line of a Claude Code transcript.
fn claude_line(line: &str, state: &mut FileState, projects: &Projects, tally: &mut Tally) {
    // Tool results are most of the bytes and none of the words; skip them
    // before paying for a parse.
    let user = line.contains(r#""type":"user""#);
    if line.contains(r#""toolUseResult""#) || !(user || line.contains(r#""type":"assistant""#)) {
        return;
    }
    let Ok(record) = serde_json::from_str::<Value>(line) else {
        return;
    };
    if state.project.is_none() {
        let Some(cwd) = record.get("cwd").and_then(Value::as_str) else {
            return;
        };
        state.project = Some(projects.name_for(Path::new(cwd)).unwrap_or_default());
        if state.project.as_deref() == Some("") {
            return;
        }
    }
    let flag = |name: &str| record.get(name).and_then(Value::as_bool).unwrap_or(false);
    let message = &record["message"];
    match record.get("type").and_then(Value::as_str) {
        Some("assistant") => {
            let Some(model) = message.get("model").and_then(Value::as_str) else {
                return;
            };
            if model.starts_with('<') {
                return; // `<synthetic>`: Claude Code's own placeholder turns.
            }
            state.model = model.to_string();
            for text in blocks(&message["content"], "text") {
                add(tally, AGENT, model, agent_words(text));
            }
        }
        Some("user") => {
            if [
                "isMeta",
                "isSidechain",
                "isCompactSummary",
                "isVisibleInTranscriptOnly",
            ]
            .iter()
            .any(|name| flag(name))
            {
                return;
            }
            let origin = record.pointer("/origin/kind").and_then(Value::as_str);
            let entry = record.get("entrypoint").and_then(Value::as_str);
            if origin.is_some_and(|kind| kind != "human") || entry.is_some_and(|e| e != "cli") {
                return;
            }
            let content = &message["content"];
            if content
                .as_array()
                .is_some_and(|parts| parts.iter().any(|p| p["type"] == "tool_result"))
            {
                return;
            }
            let texts = match content.as_str() {
                Some(text) => vec![text],
                None => blocks(content, "text"),
            };
            let model = state.model.clone();
            for text in texts {
                add(tally, YOU, &model, typed_words(text));
            }
        }
        _ => {}
    }
}

/// One line of a Codex rollout.
fn codex_line(line: &str, state: &mut FileState, projects: &Projects, tally: &mut Tally) {
    let wanted = [
        r#""session_meta""#,
        r#""turn_context""#,
        r#""thread_settings_applied""#,
        r#""role":""#,
    ];
    if !wanted.iter().any(|marker| line.contains(marker)) {
        if state.project.is_none() {
            state.project = Some(String::new()); // A format older than this reader.
        }
        return;
    }
    let Ok(record) = serde_json::from_str::<Value>(line) else {
        return;
    };
    let payload = &record["payload"];
    let kind = record.get("type").and_then(Value::as_str);
    if state.project.is_none() {
        let cwd = payload.get("cwd").and_then(Value::as_str);
        let (Some("session_meta"), Some(cwd)) = (kind, cwd) else {
            state.project = Some(String::new());
            return;
        };
        state.project = Some(projects.name_for(Path::new(cwd)).unwrap_or_default());
        if let Some(provider) = payload.get("model_provider").and_then(Value::as_str) {
            state.provider = provider.to_string();
        }
        // `exec` is a script talking, and a sub-agent is its parent talking.
        state.typed = payload.get("source").and_then(Value::as_str) == Some("cli");
        return;
    }
    match kind {
        Some("turn_context") => {
            if let Some(model) = payload.get("model").and_then(Value::as_str) {
                state.model = model.to_string();
            }
        }
        Some("event_msg") => {
            if let Some(model) = payload
                .pointer("/thread_settings/model")
                .and_then(Value::as_str)
            {
                state.model = model.to_string();
            }
        }
        Some("response_item") if payload["type"] == "message" => {
            let model = state.model.clone();
            match payload.get("role").and_then(Value::as_str) {
                Some("assistant") => {
                    for text in blocks(&payload["content"], "output_text") {
                        add(tally, AGENT, &model, agent_words(text));
                    }
                }
                Some("user") if state.typed => {
                    for text in blocks(&payload["content"], "input_text") {
                        add(tally, YOU, &model, typed_words(text));
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// The `text` of each content block of the given type.
fn blocks<'a>(content: &'a Value, kind: &str) -> Vec<&'a str> {
    content
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter(|part| part["type"] == kind)
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default()
}

/// Your projects: Workbench's own, by name, and any other directory beside
/// them (`~/Code/<name>`), so older repos count too.
pub(super) struct Projects {
    /// Longest path first, so a nested project wins over its parent.
    named: Vec<(PathBuf, String)>,
    roots: Vec<PathBuf>,
}

impl Projects {
    pub(super) fn load() -> Self {
        #[derive(serde::Deserialize)]
        struct State {
            #[serde(default)]
            workspaces: Vec<Workspace>,
        }
        #[derive(serde::Deserialize)]
        struct Workspace {
            name: String,
            path: PathBuf,
            #[serde(default)]
            global: bool,
        }
        // Read directly rather than through `persistence::load`, which moves
        // aside a file it cannot parse — not a step to take from a word count
        // that may race the app's own save.
        let workspaces = dirs::config_dir()
            .map(|dir| dir.join("workbench").join("state.json"))
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str::<State>(&text).ok())
            .map(|state| state.workspaces)
            .unwrap_or_default();
        let home = dirs::home_dir().unwrap_or_default();
        Self::new(
            workspaces
                .into_iter()
                .map(|w| (w.path, w.name, w.global))
                .collect(),
            &home,
        )
    }

    pub(super) fn new(workspaces: Vec<(PathBuf, String, bool)>, home: &Path) -> Self {
        let mut roots: Vec<PathBuf> = Vec::new();
        for (path, _, global) in &workspaces {
            let Some(parent) = path.parent() else {
                continue;
            };
            // A repo straight under your home would make the whole home a
            // root, and every stray directory a project.
            if !global && !home.starts_with(parent) && !roots.iter().any(|r| r == parent) {
                roots.push(parent.to_path_buf());
            }
        }
        let mut named: Vec<(PathBuf, String)> =
            workspaces.into_iter().map(|(p, n, _)| (p, n)).collect();
        named.sort_by_key(|(path, _)| std::cmp::Reverse(path.as_os_str().len()));
        Projects { named, roots }
    }

    fn name_for(&self, cwd: &Path) -> Option<String> {
        if let Some((_, name)) = self.named.iter().find(|(path, _)| cwd.starts_with(path)) {
            return Some(name.clone());
        }
        self.roots.iter().find_map(|root| {
            let first = cwd.strip_prefix(root).ok()?.components().next()?;
            Some(first.as_os_str().to_string_lossy().to_string())
        })
    }

    /// Claude Code names a project's directory after its path with every
    /// character that is not a letter or digit turned into `-`.
    pub(super) fn wants_claude_dir(&self, name: &str) -> bool {
        let encode = |path: &Path| -> String {
            path.to_string_lossy()
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                .collect()
        };
        self.named
            .iter()
            .map(|(path, _)| path)
            .chain(&self.roots)
            .any(|path| name.starts_with(&encode(path)))
    }
}

pub(super) fn claude_projects_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".claude"))
        .join("projects")
}

pub(super) fn codex_sessions_dir() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".codex"))
        .join("sessions")
}

fn discover(projects: &Projects) -> Vec<(PathBuf, Source)> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(claude_projects_dir()) {
        for entry in entries.flatten() {
            if projects.wants_claude_dir(&entry.file_name().to_string_lossy()) {
                walk(&entry.path(), Source::Claude, &mut out);
            }
        }
    }
    walk(&codex_sessions_dir(), Source::Codex, &mut out);
    out
}

fn walk(dir: &Path, source: Source, out: &mut Vec<(PathBuf, Source)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => walk(&path, source, out),
            Ok(kind) if kind.is_file() && path.extension().is_some_and(|e| e == "jsonl") => {
                out.push((path, source))
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projects() -> Projects {
        Projects::new(
            vec![
                ("/h/Code/workbench".into(), "workbench".into(), false),
                ("/h/Code/site".into(), "stefanlenoach.com".into(), false),
                ("/h/Lib/workbench/global".into(), "Global".into(), true),
            ],
            Path::new("/h"),
        )
    }

    #[test]
    fn a_directory_maps_to_its_project_or_to_its_folder_beside_them() {
        let p = projects();
        let name = |cwd: &str| p.name_for(Path::new(cwd));
        assert_eq!(
            name("/h/Code/workbench/.worktrees/s-1").as_deref(),
            Some("workbench")
        );
        assert_eq!(name("/h/Code/site").as_deref(), Some("stefanlenoach.com"));
        assert_eq!(name("/h/Lib/workbench/global").as_deref(), Some("Global"));
        assert_eq!(name("/h/Code/old-repo/src").as_deref(), Some("old-repo"));
        assert_eq!(name("/private/var/folders/x/T/run-1"), None);
        assert_eq!(name("/h/Code"), None);
        assert!(p.wants_claude_dir("-h-Code-old-repo"));
        assert!(p.wants_claude_dir("-h-Lib-workbench-global"));
        assert!(!p.wants_claude_dir("-private-var-folders-x-T-run-1"));
    }

    fn write(path: &Path, lines: &[Value]) {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
    }

    fn counts(connection: &Connection) -> Vec<(String, String, String, String, i64)> {
        connection
            .prepare(
                "SELECT project, model, speaker, word, n FROM counts
                 WHERE word IN ('deploy', 'shipped', 'pasted', 'result', 'hidden', 'tested')
                 ORDER BY speaker, word",
            )
            .unwrap()
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn a_claude_transcript_counts_each_line_once_across_scans() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        let db = dir.path().join("words.sqlite3");
        let cwd = "/h/Code/workbench";
        write(
            &file,
            &[
                json!({"type":"user","cwd":cwd,"entrypoint":"cli","origin":{"kind":"human"},
                       "message":{"role":"user","content":"deploy it <pasted_content id=\"x\">pasted</pasted_content>"}}),
                json!({"type":"user","cwd":cwd,"toolUseResult":{},
                       "message":{"role":"user","content":[{"type":"tool_result","content":"result"}]}}),
                json!({"type":"assistant","cwd":cwd,"message":{"model":"claude-opus-5-5",
                       "content":[{"type":"thinking","thinking":"hidden"},{"type":"text","text":"Shipped the deploy."}]}}),
            ],
        );
        let p = projects();
        let mut connection = open(&db).unwrap();
        scan_file(&mut connection, &file, Source::Claude, &p).unwrap();
        // A second pass with nothing new adds nothing.
        scan_file(&mut connection, &file, Source::Claude, &p).unwrap();
        write(
            &file,
            &[
                json!({"type":"assistant","cwd":cwd,"message":{"model":"claude-fable-5-1",
                     "content":[{"type":"text","text":"Tested the deploy."}]}}),
            ],
        );
        scan_file(&mut connection, &file, Source::Claude, &p).unwrap();

        let row = |model: &str, speaker: &str, word: &str, n: i64| {
            (
                "workbench".to_string(),
                model.to_string(),
                speaker.to_string(),
                word.to_string(),
                n,
            )
        };
        assert_eq!(
            counts(&connection),
            vec![
                row("claude-fable-5-1", "agent", "deploy", 1),
                row("claude-opus-5-5", "agent", "deploy", 1),
                row("claude-opus-5-5", "agent", "shipped", 1),
                row("claude-fable-5-1", "agent", "tested", 1),
                // Your first message comes before any reply names a model.
                row("", "you", "deploy", 1),
            ]
        );
    }

    #[test]
    fn a_codex_rollout_attributes_words_and_ignores_scripts() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("words.sqlite3");
        let rollout = |name: &str, source: &str| {
            let file = dir.path().join(name);
            write(
                &file,
                &[
                    json!({"type":"session_meta","payload":{"cwd":"/h/Code/old-repo","source":source,"model_provider":"openai"}}),
                    json!({"type":"turn_context","payload":{"model":"gpt-6-astra","cwd":"/h/Code/old-repo"}}),
                    json!({"type":"response_item","payload":{"type":"message","role":"user",
                           "content":[{"type":"input_text","text":"<environment_context>hidden</environment_context>"},
                                      {"type":"input_text","text":"deploy now"}]}}),
                    json!({"type":"response_item","payload":{"type":"message","role":"assistant",
                           "content":[{"type":"output_text","text":"Shipped."}]}}),
                ],
            );
            file
        };
        let p = projects();
        let mut connection = open(&db).unwrap();
        scan_file(
            &mut connection,
            &rollout("a.jsonl", "cli"),
            Source::Codex,
            &p,
        )
        .unwrap();
        scan_file(
            &mut connection,
            &rollout("b.jsonl", "exec"),
            Source::Codex,
            &p,
        )
        .unwrap();

        let row = |speaker: &str, word: &str, n: i64| {
            (
                "old-repo".to_string(),
                "gpt-6-astra".to_string(),
                speaker.to_string(),
                word.to_string(),
                n,
            )
        };
        assert_eq!(
            counts(&connection),
            vec![row("agent", "shipped", 2), row("you", "deploy", 1)],
            "the exec rollout's prompt was a script, not you"
        );
    }

    #[test]
    fn a_transcript_outside_your_projects_is_read_once_and_then_skipped() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        let db = dir.path().join("words.sqlite3");
        write(
            &file,
            &[json!({"type":"user","cwd":"/tmp/run","message":{"role":"user","content":"deploy"}})],
        );
        let mut connection = open(&db).unwrap();
        scan_file(&mut connection, &file, Source::Claude, &projects()).unwrap();
        let (offset, project): (i64, String) = connection
            .query_row("SELECT offset, project FROM files", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(offset as u64, std::fs::metadata(&file).unwrap().len());
        assert_eq!(project, "");
        assert!(counts(&connection).is_empty());
    }
}
