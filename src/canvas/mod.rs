//! A local, read-only map of every repository registered with Workbench.
//!
//! The TUI is the right place to operate agents, but a terminal is a poor
//! surface for a spatial file tree. This module starts a loopback-only HTTP
//! server on demand and opens an embedded browser UI. The browser asks for a
//! fresh, git-aware file list when it changes workspace or refreshes, then
//! performs all layout, collapsing, searching, panning, and zooming locally.

mod page;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, UNIX_EPOCH};
use tiny_http::{Header, Response, Server};

const MAX_FILES: usize = 6_000;
const MAX_PREVIEW_BYTES: usize = 512 * 1024;
const MAX_ASK_BYTES: usize = 64 * 1024;
const MAX_PROMPT_CHARS: usize = 4_000;
const MAX_SELECTION: usize = 80;
const MAX_HISTORY_TURNS: usize = 6;
const MAX_HISTORY_CHARS: usize = 12_000;
const MAX_CONCURRENT_AGENTS: usize = 8;
const CANVAS_AGENT_TIMEOUT: Duration = Duration::from_secs(10 * 60);
pub const CANVAS_AGENT_MODEL: &str = "claude-sonnet-5";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CanvasWorkspace {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanvasCommand {
    pub request_id: String,
    pub workspace: String,
    pub note_id: String,
    pub scope: CanvasScope,
    pub intent: CanvasIntent,
    pub prompt: String,
    pub paths: Vec<String>,
    pub history: Vec<CanvasTurn>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct CanvasTurn {
    pub question: String,
    pub answer: String,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CanvasScope {
    Repository,
    #[default]
    Selection,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CanvasIntent {
    #[default]
    Analysis,
    Architecture,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CanvasRequestStatus {
    Queued,
    Working,
    Complete,
    Error,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CanvasExchange {
    pub id: String,
    pub workspace: String,
    pub note_id: String,
    pub model: String,
    pub scope: CanvasScope,
    pub intent: CanvasIntent,
    pub prompt: String,
    pub paths: Vec<String>,
    pub status: CanvasRequestStatus,
    pub answer: Option<String>,
    pub operations: Vec<CanvasOperation>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CanvasOperation {
    Highlight {
        paths: Vec<String>,
        #[serde(default)]
        color: Option<String>,
        #[serde(default)]
        label: Option<String>,
    },
    Note {
        title: String,
        body: String,
        #[serde(default)]
        paths: Vec<String>,
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
    },
    Connect {
        from: String,
        to: String,
        #[serde(default)]
        label: Option<String>,
    },
    Group {
        paths: Vec<String>,
        title: String,
        #[serde(default)]
        color: Option<String>,
    },
    Diagram {
        title: String,
        nodes: Vec<DiagramNode>,
        #[serde(default)]
        edges: Vec<DiagramEdge>,
    },
    Architecture {
        title: String,
        #[serde(default)]
        summary: String,
        #[serde(default)]
        level: ArchitectureLevel,
        #[serde(default)]
        focus_paths: Vec<String>,
        nodes: Vec<ArchitectureNode>,
        #[serde(default)]
        edges: Vec<DiagramEdge>,
    },
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchitectureLevel {
    #[default]
    Overview,
    Subsystem,
    Files,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ArchitectureNode {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DiagramNode {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DiagramEdge {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Default)]
struct CanvasShared {
    workspaces: Vec<CanvasWorkspace>,
    requests: HashMap<String, CanvasExchange>,
    commands: VecDeque<CanvasCommand>,
}

impl CanvasWorkspace {
    pub fn new(id: impl Into<String>, name: impl Into<String>, path: PathBuf) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            path,
        }
    }
}

/// The process-local server. Dropping this handle does not need an explicit
/// shutdown: its thread and socket leave with the Workbench process.
#[derive(Clone)]
pub struct CanvasServer {
    addr: SocketAddr,
    shared: Arc<RwLock<CanvasShared>>,
}

impl CanvasServer {
    pub fn start(workspaces: Vec<CanvasWorkspace>) -> Result<Self> {
        let server = Server::http((Ipv4Addr::LOCALHOST, 0))
            .map_err(|err| anyhow!("could not start repository map: {err}"))?;
        let addr = server
            .server_addr()
            .to_ip()
            .ok_or_else(|| anyhow!("repository map did not receive an IP address"))?;
        let shared = Arc::new(RwLock::new(CanvasShared {
            workspaces,
            ..CanvasShared::default()
        }));
        let served = shared.clone();

        std::thread::spawn(move || {
            for mut request in server.incoming_requests() {
                let url = request.url().to_string();
                let method = request.method().as_str().to_string();
                let mut body = String::new();
                if method == "POST" {
                    let _ = request
                        .as_reader()
                        .take((MAX_ASK_BYTES + 1) as u64)
                        .read_to_string(&mut body);
                }
                let response = handle(&url, &method, &body, &served);
                if let Err(err) = request.respond(response) {
                    crate::logger::warn(format!("repository map response failed: {err}"));
                }
            }
        });

        Ok(Self { addr, shared })
    }

    pub fn replace_workspaces(&self, workspaces: Vec<CanvasWorkspace>) {
        if let Ok(mut shared) = self.shared.write() {
            shared.workspaces = workspaces;
        }
    }

    pub fn take_commands(&self) -> Vec<CanvasCommand> {
        self.shared
            .write()
            .map(|mut shared| shared.commands.drain(..).collect())
            .unwrap_or_default()
    }

    pub fn mark_working(&self, request_id: &str) {
        if let Ok(mut shared) = self.shared.write() {
            if let Some(request) = shared.requests.get_mut(request_id) {
                request.status = CanvasRequestStatus::Working;
                request.error = None;
            }
        }
    }

    pub fn complete(&self, request_id: &str, raw_answer: &str) {
        if let Ok(mut shared) = self.shared.write() {
            let workspace_root = shared
                .requests
                .get(request_id)
                .and_then(|request| {
                    shared
                        .workspaces
                        .iter()
                        .find(|workspace| workspace.id == request.workspace)
                })
                .map(|workspace| workspace.path.clone());
            let Some(request) = shared.requests.get_mut(request_id) else {
                return;
            };
            let (answer, mut operations) = parse_canvas_response(raw_answer);
            sanitize_operations(&mut operations, workspace_root.as_deref());
            if operations.is_empty() && !request.paths.is_empty() {
                operations.push(CanvasOperation::Highlight {
                    paths: request.paths.clone(),
                    color: Some("green".into()),
                    label: Some("Agent selection".into()),
                });
            }
            request.status = CanvasRequestStatus::Complete;
            request.answer = Some(answer);
            request.operations = operations;
            request.error = None;
        }
    }

    pub fn fail(&self, request_id: &str, message: impl Into<String>) {
        if let Ok(mut shared) = self.shared.write() {
            if let Some(request) = shared.requests.get_mut(request_id) {
                request.status = CanvasRequestStatus::Error;
                request.error = Some(message.into());
            }
        }
    }

    /// Run one disposable, read-only Claude Code instance for a board note.
    /// The process is deliberately outside the normal session registry and
    /// disables Claude's on-disk transcript persistence.
    pub fn launch_agent(&self, command: CanvasCommand, working_dir: PathBuf) {
        self.mark_working(&command.request_id);
        let server = self.clone();
        std::thread::spawn(move || {
            let prompt = canvas_agent_prompt(&command);
            match run_canvas_agent(&working_dir, &prompt) {
                Ok(answer) => server.complete(&command.request_id, &answer),
                Err(message) => server.fail(&command.request_id, message),
            }
        });
    }

    pub fn url(&self, workspace: Option<&str>) -> String {
        let base = format!("http://{}", self.addr);
        match workspace {
            Some(id) => format!("{base}/?workspace={}", percent_encode(id)),
            None => base,
        }
    }
}

pub fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(url);
        command
    };

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", "", url]);
        command
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };

    command
        .spawn()
        .map(|_| ())
        .map_err(|err| anyhow!("could not open a browser: {err}"))
}

#[derive(Debug, Serialize)]
struct WorkspaceSummary {
    id: String,
    name: String,
    path: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct TreeEntry {
    path: String,
    name: String,
    kind: &'static str,
    extension: Option<String>,
    size: Option<u64>,
    modified: Option<u64>,
    status: Option<&'static str>,
}

#[derive(Debug, Serialize)]
struct TreeResponse {
    workspace: String,
    name: String,
    root: String,
    entries: Vec<TreeEntry>,
    truncated: bool,
    tracked: bool,
}

#[derive(Debug, Serialize)]
struct FileResponse {
    path: String,
    name: String,
    extension: Option<String>,
    language: &'static str,
    content: String,
    bytes: u64,
    truncated: bool,
}

fn handle(
    url: &str,
    method: &str,
    body: &str,
    shared: &Arc<RwLock<CanvasShared>>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    match (method, path) {
        ("GET", "/" | "/canvas") => html(page::HTML),
        ("GET", "/api/workspaces") => {
            let list = shared
                .read()
                .map(|shared| {
                    shared
                        .workspaces
                        .iter()
                        .map(|workspace| WorkspaceSummary {
                            id: workspace.id.clone(),
                            name: workspace.name.clone(),
                            path: workspace.path.to_string_lossy().into_owned(),
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            json(serde_json::to_string(&list).unwrap_or_else(|_| "[]".into()))
        }
        ("GET", "/api/tree") => {
            let Some(id) = query_value(query, "workspace") else {
                return status(400, "missing workspace");
            };
            let workspace = shared
                .read()
                .ok()
                .and_then(|shared| shared.workspaces.iter().find(|item| item.id == id).cloned());
            let Some(workspace) = workspace else {
                return status(404, "unknown workspace");
            };
            match scan_workspace(&workspace) {
                Ok(tree) => json(serde_json::to_string(&tree).unwrap_or_default()),
                Err(err) => status(500, &format!("could not scan repository: {err}")),
            }
        }
        ("GET", "/api/file") => {
            let Some(id) = query_value(query, "workspace") else {
                return status(400, "missing workspace");
            };
            let Some(path) = query_value(query, "path") else {
                return status(400, "missing file path");
            };
            let workspace = shared
                .read()
                .ok()
                .and_then(|shared| shared.workspaces.iter().find(|item| item.id == id).cloned());
            let Some(workspace) = workspace else {
                return status(404, "unknown workspace");
            };
            match read_file(&workspace, &path) {
                Ok(file) => json(serde_json::to_string(&file).unwrap_or_default()),
                Err((code, message)) => status(code, &message),
            }
        }
        ("POST", "/api/ask") => queue_ask(body, shared),
        ("GET", "/api/ask") => {
            let Some(id) = query_value(query, "id") else {
                return status(400, "missing request id");
            };
            let request = shared
                .read()
                .ok()
                .and_then(|shared| shared.requests.get(&id).cloned());
            match request {
                Some(request) => json(serde_json::to_string(&request).unwrap_or_default()),
                None => status(404, "unknown request"),
            }
        }
        (_, "/" | "/canvas" | "/api/workspaces" | "/api/tree" | "/api/file" | "/api/ask") => {
            status(405, "method not allowed")
        }
        _ => status(404, "not found"),
    }
}

#[derive(Debug, Deserialize)]
struct AskBody {
    workspace: String,
    note_id: String,
    #[serde(default)]
    scope: CanvasScope,
    #[serde(default)]
    intent: CanvasIntent,
    prompt: String,
    paths: Vec<String>,
    #[serde(default)]
    history: Vec<CanvasTurn>,
}

fn queue_ask(body: &str, shared: &Arc<RwLock<CanvasShared>>) -> Response<std::io::Cursor<Vec<u8>>> {
    if body.len() > MAX_ASK_BYTES {
        return status(413, "request is too large");
    }
    let Ok(mut ask) = serde_json::from_str::<AskBody>(body) else {
        return status(400, "invalid request body");
    };
    ask.prompt = ask.prompt.trim().to_string();
    if ask.prompt.is_empty() || ask.prompt.chars().count() > MAX_PROMPT_CHARS {
        return status(400, "prompt must be between 1 and 4,000 characters");
    }
    ask.paths.sort();
    ask.paths.dedup();
    if uuid::Uuid::parse_str(&ask.note_id).is_err() {
        return status(400, "invalid note id");
    }
    let mut history = Vec::new();
    let mut history_chars = 0;
    for mut turn in std::mem::take(&mut ask.history)
        .into_iter()
        .rev()
        .take(MAX_HISTORY_TURNS)
    {
        turn.question = turn.question.trim().to_string();
        turn.answer = turn.answer.trim().to_string();
        let chars = turn.question.chars().count() + turn.answer.chars().count();
        if !turn.question.is_empty()
            && !turn.answer.is_empty()
            && chars <= MAX_HISTORY_CHARS
            && history_chars + chars <= MAX_HISTORY_CHARS
        {
            history_chars += chars;
            history.push(turn);
        }
    }
    history.reverse();
    ask.history = history;
    match ask.scope {
        CanvasScope::Repository => ask.paths.clear(),
        CanvasScope::Selection if ask.paths.is_empty() || ask.paths.len() > MAX_SELECTION => {
            return status(400, "select between 1 and 80 items");
        }
        CanvasScope::Selection => {}
    }

    let Ok(mut shared) = shared.write() else {
        return status(503, "canvas is busy");
    };
    let Some(workspace) = shared
        .workspaces
        .iter()
        .find(|workspace| workspace.id == ask.workspace)
        .cloned()
    else {
        return status(404, "unknown workspace");
    };
    let active = shared
        .requests
        .values()
        .filter(|request| {
            matches!(
                request.status,
                CanvasRequestStatus::Queued | CanvasRequestStatus::Working
            )
        })
        .count();
    if active >= MAX_CONCURRENT_AGENTS {
        return status(429, "too many canvas agents are already working");
    }
    if shared.requests.values().any(|request| {
        request.note_id == ask.note_id
            && matches!(
                request.status,
                CanvasRequestStatus::Queued | CanvasRequestStatus::Working
            )
    }) {
        return status(409, "this note already has an agent working");
    }
    if ask
        .paths
        .iter()
        .any(|path| !selection_exists(&workspace, path))
    {
        return status(400, "selection contains a path outside this workspace");
    }

    let id = uuid::Uuid::new_v4().to_string();
    let exchange = CanvasExchange {
        id: id.clone(),
        workspace: ask.workspace.clone(),
        note_id: ask.note_id.clone(),
        model: CANVAS_AGENT_MODEL.into(),
        scope: ask.scope,
        intent: ask.intent,
        prompt: ask.prompt.clone(),
        paths: ask.paths.clone(),
        status: CanvasRequestStatus::Queued,
        answer: None,
        operations: Vec::new(),
        error: None,
    };
    if let Err(err) = crate::prompt_log::record_canvas_prompt(
        &workspace.id,
        &workspace.name,
        &workspace.path,
        &ask.note_id,
        &ask.prompt,
        CANVAS_AGENT_MODEL,
    ) {
        crate::logger::warn(format!("failed to record canvas prompt: {err}"));
    }
    shared.commands.push_back(CanvasCommand {
        request_id: id.clone(),
        workspace: ask.workspace,
        note_id: ask.note_id,
        scope: ask.scope,
        intent: ask.intent,
        prompt: ask.prompt,
        paths: ask.paths,
        history: ask.history,
    });
    shared.requests.insert(id, exchange.clone());
    while shared.requests.len() > 200 {
        let Some(oldest) = shared.requests.keys().next().cloned() else {
            break;
        };
        shared.requests.remove(&oldest);
    }
    json(serde_json::to_string(&exchange).unwrap_or_default()).with_status_code(202)
}

fn selection_exists(workspace: &CanvasWorkspace, requested: &str) -> bool {
    let relative = Path::new(requested);
    if requested.is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return false;
    }
    let (Ok(root), Ok(target)) = (
        workspace.path.canonicalize(),
        workspace.path.join(relative).canonicalize(),
    ) else {
        return false;
    };
    target.starts_with(root)
}

fn canvas_agent_prompt(command: &CanvasCommand) -> String {
    let (scope, context, inspection) = match command.scope {
        CanvasScope::Repository => (
            "Whole repository",
            "No individual paths were selected. Treat the repository root as the context.".into(),
            "Inspect the repository structure and the files most relevant to the question.",
        ),
        CanvasScope::Selection => (
            "Selected items",
            command
                .paths
                .iter()
                .map(|path| format!("- {path}"))
                .collect::<Vec<_>>()
                .join("\n"),
            "Inspect the selected paths and any closely related repository files needed to answer.",
        ),
    };
    let prior_context = if command.history.is_empty() {
        "This is the first turn in this note.".to_string()
    } else {
        let turns = command
            .history
            .iter()
            .enumerate()
            .map(|(index, turn)| {
                format!(
                    "Turn {}\nUser: {}\nAgent: {}",
                    index + 1,
                    turn.question,
                    turn.answer
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        format!(
            "Use these earlier turns from this board note as conversational context. They are transient and may be incomplete:\n\n{turns}"
        )
    };
    let operation_guidance = match command.intent {
        CanvasIntent::Analysis => {
            r#"Use canvas operations only when they make the explanation clearer.

Valid operations are:
- highlight: paths, optional color (green/blue/amber/violet/red), optional label
- note: title, body, optional paths, optional x/y
- connect: from, to, optional label
- group: paths, title, optional color
- diagram: title, nodes [{id,label,optional path}], edges [{from,to,optional label}]"#
        }
        CanvasIntent::Architecture => {
            r#"Create one grounded architecture lens rather than mirroring the folder tree. At repository scope, organize the code into 5-9 meaningful product or system concepts. At selection scope, organize the selected subsystem into 3-8 components. Prefer responsibilities and runtime boundaries over folder names.

Return exactly one architecture operation. Every concept must cite one or more real repository-relative files or directories in `paths`. Keep summaries to one sentence, use short relationship labels, and do not invent coordinates. Choose colors from green, blue, amber, violet, or red. Use `level` = overview for repository scope or subsystem for selection scope.

Architecture operation shape:
{"kind":"architecture","title":"System architecture","summary":"One-sentence orientation","level":"overview","focus_paths":[],"nodes":[{"id":"engine","label":"Simulation engine","summary":"Advances the physical world state.","kind":"runtime","color":"blue","paths":["src/engine"]}],"edges":[{"from":"engine","to":"ui","label":"publishes state"}]}"#
        }
    };
    let envelope_example = match command.intent {
        CanvasIntent::Analysis => {
            r#"{"answer":"Concise markdown explanation","operations":[{"kind":"highlight","paths":["path"],"color":"green","label":"why"}]}"#
        }
        CanvasIntent::Architecture => {
            r#"{"answer":"Concise orientation","operations":[{"kind":"architecture","title":"System architecture","summary":"One-sentence orientation","level":"overview","focus_paths":[],"nodes":[{"id":"engine","label":"Simulation engine","summary":"Advances world state.","kind":"runtime","color":"blue","paths":["src/engine"]}],"edges":[]}] }"#
        }
    };
    format!(
        r#"You are a disposable, read-only Claude Code agent answering from an independent note on Workbench's repository canvas. Do not modify the repository.

Scope: {scope}

Context:
{context}

Note conversation:
{prior_context}

Current question:
{question}

{inspection} Do not edit files, commit, install dependencies, access secrets, or run destructive commands. Return one final fenced block in exactly this shape:

```workbench-canvas
{envelope_example}
```

{operation_guidance}

Use repository-relative paths exactly as shown by the canvas. Put the complete user-facing response in `answer`. Escape the JSON correctly and do not write anything after the closing fence."#,
        question = command.prompt
    )
}

fn run_canvas_agent(working_dir: &Path, prompt: &str) -> std::result::Result<String, String> {
    let mut child = Command::new("claude")
        .current_dir(working_dir)
        .args(canvas_agent_args())
        .arg(prompt)
        .env("CLAUDE_CODE_SKIP_PROMPT_HISTORY", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("Could not start Claude Code: {err}"))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut stream) = stdout {
            let _ = stream.read_to_end(&mut bytes);
        }
        bytes
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut stream) = stderr {
            let _ = stream.read_to_end(&mut bytes);
        }
        bytes
    });

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() < CANVAS_AGENT_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(75));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err("Claude Code did not answer within 10 minutes.".to_string());
            }
            Err(err) => break Err(format!("Could not monitor Claude Code: {err}")),
        }
    };

    let stdout = String::from_utf8_lossy(&stdout_reader.join().unwrap_or_default())
        .trim()
        .to_string();
    let stderr = String::from_utf8_lossy(&stderr_reader.join().unwrap_or_default())
        .trim()
        .to_string();
    let status = status?;
    if !status.success() {
        let detail = if stderr.is_empty() {
            format!("Claude Code exited with {status}.")
        } else {
            format!(
                "Claude Code could not answer: {}",
                truncate_message(&stderr, 1_200)
            )
        };
        return Err(detail);
    }
    if stdout.is_empty() {
        return Err("Claude Code finished without an answer.".into());
    }
    Ok(stdout)
}

fn canvas_agent_args() -> [&'static str; 11] {
    [
        "-p",
        "--model",
        CANVAS_AGENT_MODEL,
        "--no-session-persistence",
        "--permission-mode",
        "plan",
        "--tools",
        "Read,Glob,Grep",
        "--disable-slash-commands",
        "--no-chrome",
        "--safe-mode",
    ]
}

fn truncate_message(value: &str, limit: usize) -> String {
    let mut output: String = value.chars().take(limit).collect();
    if value.chars().count() > limit {
        output.push('…');
    }
    output
}

#[derive(Debug, Deserialize)]
struct CanvasEnvelope {
    answer: String,
    #[serde(default)]
    operations: Vec<CanvasOperation>,
}

fn parse_canvas_response(raw: &str) -> (String, Vec<CanvasOperation>) {
    const OPEN: &str = "```workbench-canvas";
    let Some(start) = raw.rfind(OPEN) else {
        return (raw.trim().to_string(), Vec::new());
    };
    let json_start = start + OPEN.len();
    let Some(end_offset) = raw[json_start..].find("```") else {
        return (raw.trim().to_string(), Vec::new());
    };
    let payload = raw[json_start..json_start + end_offset].trim();
    match serde_json::from_str::<CanvasEnvelope>(payload) {
        Ok(envelope) => (envelope.answer.trim().to_string(), envelope.operations),
        Err(_) => (raw.trim().to_string(), Vec::new()),
    }
}

fn sanitize_operations(operations: &mut Vec<CanvasOperation>, workspace_root: Option<&Path>) {
    operations.truncate(30);
    operations.retain_mut(|operation| match operation {
        CanvasOperation::Highlight {
            paths,
            color,
            label,
        } => {
            sanitize_paths(paths, workspace_root);
            *color = safe_color(color.take());
            trim_optional(label, 80);
            !paths.is_empty()
        }
        CanvasOperation::Note {
            title,
            body,
            paths,
            x,
            y,
        } => {
            truncate_chars(title, 120);
            truncate_chars(body, 2_000);
            sanitize_paths(paths, workspace_root);
            if x.is_some_and(|value| !value.is_finite()) {
                *x = None;
            }
            if y.is_some_and(|value| !value.is_finite()) {
                *y = None;
            }
            !title.is_empty() && !body.is_empty()
        }
        CanvasOperation::Connect { from, to, label } => {
            trim_optional(label, 80);
            safe_canvas_path(from) && safe_canvas_path(to)
        }
        CanvasOperation::Group {
            paths,
            title,
            color,
        } => {
            sanitize_paths(paths, workspace_root);
            truncate_chars(title, 120);
            *color = safe_color(color.take());
            !paths.is_empty() && !title.is_empty()
        }
        CanvasOperation::Diagram {
            title,
            nodes,
            edges,
        } => {
            truncate_chars(title, 120);
            nodes.truncate(24);
            edges.truncate(48);
            for node in nodes.iter_mut() {
                truncate_chars(&mut node.id, 80);
                truncate_chars(&mut node.label, 160);
                if node
                    .path
                    .as_deref()
                    .is_some_and(|path| !safe_canvas_path(path))
                {
                    node.path = None;
                }
            }
            nodes.retain(|node| !node.id.is_empty() && !node.label.is_empty());
            let ids: HashSet<&str> = nodes.iter().map(|node| node.id.as_str()).collect();
            edges.retain_mut(|edge| {
                trim_optional(&mut edge.label, 80);
                ids.contains(edge.from.as_str()) && ids.contains(edge.to.as_str())
            });
            !title.is_empty() && !nodes.is_empty()
        }
        CanvasOperation::Architecture {
            title,
            summary,
            focus_paths,
            nodes,
            edges,
            ..
        } => {
            truncate_chars(title, 120);
            truncate_chars(summary, 600);
            sanitize_paths(focus_paths, workspace_root);
            nodes.truncate(12);
            edges.truncate(32);
            for node in nodes.iter_mut() {
                truncate_chars(&mut node.id, 80);
                truncate_chars(&mut node.label, 120);
                truncate_chars(&mut node.summary, 360);
                trim_optional(&mut node.kind, 40);
                node.color = safe_color(node.color.take());
                sanitize_paths(&mut node.paths, workspace_root);
            }
            nodes.retain(|node| {
                !node.id.is_empty() && !node.label.is_empty() && !node.paths.is_empty()
            });
            let ids: HashSet<&str> = nodes.iter().map(|node| node.id.as_str()).collect();
            edges.retain_mut(|edge| {
                trim_optional(&mut edge.label, 80);
                ids.contains(edge.from.as_str()) && ids.contains(edge.to.as_str())
            });
            !title.is_empty() && !nodes.is_empty()
        }
    });
}

fn sanitize_paths(paths: &mut Vec<String>, workspace_root: Option<&Path>) {
    paths.truncate(MAX_SELECTION);
    paths.retain(|path| {
        safe_canvas_path(path)
            && workspace_root.is_none_or(|root| root.join(Path::new(path)).exists())
    });
    paths.sort();
    paths.dedup();
}

fn safe_canvas_path(path: &str) -> bool {
    let path = Path::new(path);
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && !path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

fn safe_color(color: Option<String>) -> Option<String> {
    color.filter(|color| {
        matches!(
            color.as_str(),
            "green" | "blue" | "amber" | "violet" | "red"
        )
    })
}

fn truncate_chars(value: &mut String, limit: usize) {
    if value.chars().count() > limit {
        *value = value.chars().take(limit).collect();
    }
    *value = value.trim().to_string();
}

fn trim_optional(value: &mut Option<String>, limit: usize) {
    if let Some(value) = value.as_mut() {
        truncate_chars(value, limit);
        if value.is_empty() {
            *value = String::new();
        }
    }
    if value.as_deref() == Some("") {
        *value = None;
    }
}

fn read_file(
    workspace: &CanvasWorkspace,
    requested: &str,
) -> std::result::Result<FileResponse, (u16, String)> {
    let relative = Path::new(requested);
    if requested.is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err((400, "file path must stay inside the workspace".into()));
    }

    let root = workspace
        .path
        .canonicalize()
        .map_err(|err| (500, format!("could not resolve workspace: {err}")))?;
    let target = root
        .join(relative)
        .canonicalize()
        .map_err(|_| (404, "file not found".into()))?;
    if !target.starts_with(&root) {
        return Err((403, "file path escapes the workspace".into()));
    }

    let metadata = target
        .metadata()
        .map_err(|_| (404, "file not found".into()))?;
    if !metadata.is_file() {
        return Err((415, "only regular files can be previewed".into()));
    }

    let mut bytes = Vec::with_capacity(
        usize::try_from(metadata.len())
            .unwrap_or(MAX_PREVIEW_BYTES)
            .min(MAX_PREVIEW_BYTES + 1),
    );
    std::fs::File::open(&target)
        .map_err(|err| (500, format!("could not open file: {err}")))?
        .take((MAX_PREVIEW_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|err| (500, format!("could not read file: {err}")))?;

    if bytes.iter().any(|byte| *byte == 0) {
        return Err((415, "binary files cannot be previewed".into()));
    }
    let truncated = bytes.len() > MAX_PREVIEW_BYTES;
    bytes.truncate(MAX_PREVIEW_BYTES);
    let content = match String::from_utf8(bytes) {
        Ok(content) => content,
        Err(error) if truncated && error.utf8_error().error_len().is_none() => {
            let valid = error.utf8_error().valid_up_to();
            String::from_utf8(error.into_bytes()[..valid].to_vec())
                .expect("valid UTF-8 prefix remains valid")
        }
        Err(_) => return Err((415, "file is not UTF-8 text".into())),
    };

    let extension = relative
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .filter(|extension| !extension.is_empty());
    let name = relative
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| requested.to_string());
    let language = language_for(extension.as_deref(), &name);
    Ok(FileResponse {
        path: requested.replace('\\', "/"),
        name,
        language,
        extension,
        content,
        bytes: metadata.len(),
        truncated,
    })
}

fn language_for(extension: Option<&str>, name: &str) -> &'static str {
    match extension {
        Some("rs") => "Rust",
        Some("js" | "mjs" | "cjs") => "JavaScript",
        Some("ts" | "mts" | "cts") => "TypeScript",
        Some("jsx") => "JSX",
        Some("tsx") => "TSX",
        Some("py") => "Python",
        Some("go") => "Go",
        Some("rb") => "Ruby",
        Some("java") => "Java",
        Some("c" | "h") => "C",
        Some("cc" | "cpp" | "cxx" | "hpp") => "C++",
        Some("cs") => "C#",
        Some("swift") => "Swift",
        Some("kt" | "kts") => "Kotlin",
        Some("sh" | "bash" | "zsh") => "Shell",
        Some("html" | "htm") => "HTML",
        Some("css") => "CSS",
        Some("scss" | "sass") => "SCSS",
        Some("json" | "jsonc") => "JSON",
        Some("toml") => "TOML",
        Some("yaml" | "yml") => "YAML",
        Some("md" | "mdx") => "Markdown",
        Some("sql") => "SQL",
        Some("xml" | "svg") => "XML",
        Some("vue") => "Vue",
        Some("svelte") => "Svelte",
        _ if matches!(name, "Dockerfile" | "Containerfile") => "Dockerfile",
        _ if matches!(name, "Makefile" | "Justfile") => "Makefile",
        _ => "Plain text",
    }
}

fn scan_workspace(workspace: &CanvasWorkspace) -> Result<TreeResponse> {
    let (paths, tracked, truncated) = match git_file_paths(&workspace.path) {
        Some(paths) => {
            let truncated = paths.len() > MAX_FILES;
            (paths.into_iter().take(MAX_FILES).collect(), true, truncated)
        }
        None => filesystem_paths(&workspace.path, MAX_FILES)?,
    };
    let statuses = if tracked {
        git_statuses(&workspace.path)
    } else {
        HashMap::new()
    };

    let mut entries: Vec<TreeEntry> = paths
        .into_iter()
        .filter_map(|relative| tree_entry(&workspace.path, relative, &statuses))
        .collect();
    entries.sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));

    Ok(TreeResponse {
        workspace: workspace.id.clone(),
        name: workspace.name.clone(),
        root: workspace.path.to_string_lossy().into_owned(),
        entries,
        truncated,
        tracked,
    })
}

fn git_file_paths(root: &Path) -> Option<Vec<PathBuf>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| PathBuf::from(String::from_utf8_lossy(path).into_owned()))
            .collect(),
    )
}

fn git_statuses(root: &Path) -> HashMap<PathBuf, &'static str> {
    let Some(output) = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=normal"])
        .output()
        .ok()
        .filter(|output| output.status.success())
    else {
        return HashMap::new();
    };

    let records: Vec<&[u8]> = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .collect();
    let mut statuses = HashMap::new();
    let mut index = 0;
    while index < records.len() {
        let record = records[index];
        if record.len() < 4 {
            index += 1;
            continue;
        }
        let xy = &record[..2];
        let path = PathBuf::from(String::from_utf8_lossy(&record[3..]).into_owned());
        let label = if xy == b"??" {
            "untracked"
        } else if xy.contains(&b'A') {
            "added"
        } else if xy.contains(&b'R') || xy.contains(&b'C') {
            "renamed"
        } else if xy.contains(&b'M') {
            "modified"
        } else if xy.contains(&b'D') {
            "deleted"
        } else {
            "changed"
        };
        statuses.insert(path, label);
        if xy.contains(&b'R') || xy.contains(&b'C') {
            index += 1; // The second NUL record is the other rename path.
        }
        index += 1;
    }
    statuses
}

fn filesystem_paths(root: &Path, limit: usize) -> Result<(Vec<PathBuf>, bool, bool)> {
    const SKIP_DIRS: &[&str] = &[
        ".git",
        ".worktrees",
        "node_modules",
        "target",
        "dist",
        "build",
        "vendor",
        ".next",
        ".cache",
    ];

    let mut paths = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let mut visited = HashSet::new();
    let mut truncated = false;

    while let Some(directory) = stack.pop() {
        let canonical = directory
            .canonicalize()
            .unwrap_or_else(|_| directory.clone());
        if !visited.insert(canonical) {
            continue;
        }
        let mut entries: Vec<_> = std::fs::read_dir(&directory)?
            .filter_map(Result::ok)
            .collect();
        entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_lowercase());
        entries.reverse();

        for entry in entries {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                let name = entry.file_name();
                if !SKIP_DIRS.contains(&name.to_string_lossy().as_ref()) {
                    stack.push(path);
                }
                continue;
            }
            if let Ok(relative) = path.strip_prefix(root) {
                paths.push(relative.to_path_buf());
            }
            if paths.len() > limit {
                paths.truncate(limit);
                truncated = true;
                break;
            }
        }
        if truncated {
            break;
        }
    }

    Ok((paths, false, truncated))
}

fn tree_entry(
    root: &Path,
    relative: PathBuf,
    statuses: &HashMap<PathBuf, &'static str>,
) -> Option<TreeEntry> {
    let path = relative.to_string_lossy().replace('\\', "/");
    let name = relative.file_name()?.to_string_lossy().into_owned();
    let metadata = std::fs::symlink_metadata(root.join(&relative)).ok();
    let kind = match metadata.as_ref().map(std::fs::Metadata::file_type) {
        Some(kind) if kind.is_symlink() => "symlink",
        _ => "file",
    };
    let extension = relative
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .filter(|extension| !extension.is_empty());
    let size = metadata.as_ref().map(std::fs::Metadata::len);
    let modified = metadata
        .as_ref()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs());
    let status = statuses.get(&relative).copied();
    Some(TreeEntry {
        path,
        name,
        kind,
        extension,
        size,
        modified,
        status,
    })
}

fn query_value(query: &str, key: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(candidate, _)| *candidate == key)
        .map(|(_, value)| percent_decode(value))
}

fn percent_decode(value: &str) -> String {
    let bytes = value.replace('+', " ").into_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(byte) =
                u8::from_str_radix(&String::from_utf8_lossy(&bytes[index + 1..index + 3]), 16)
            {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static header parses")
}

fn response(body: String, content_type: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(body)
        .with_header(header("Content-Type", content_type))
        .with_header(header("Cache-Control", "no-store"))
        .with_header(header("X-Content-Type-Options", "nosniff"))
        .with_header(header(
            "Content-Security-Policy",
            "default-src 'self'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:",
        ))
}

fn html(body: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    response(body.to_string(), "text/html; charset=utf-8")
}

fn json(body: String) -> Response<std::io::Cursor<Vec<u8>>> {
    response(body, "application/json; charset=utf-8")
}

fn status(code: u16, message: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    response(message.to_string(), "text/plain; charset=utf-8").with_status_code(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn fallback_scan_skips_generated_trees() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src/nested")).unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules/pkg")).unwrap();
        std::fs::create_dir_all(dir.path().join(".git/objects")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("src/nested/lib.rs"), "").unwrap();
        std::fs::write(dir.path().join("node_modules/pkg/index.js"), "").unwrap();
        std::fs::write(dir.path().join(".env"), "LOCAL=1").unwrap();

        let (mut paths, tracked, truncated) = filesystem_paths(dir.path(), MAX_FILES).unwrap();
        paths.sort();
        assert!(!tracked);
        assert!(!truncated);
        assert_eq!(
            paths,
            vec![
                PathBuf::from(".env"),
                PathBuf::from("src/main.rs"),
                PathBuf::from("src/nested/lib.rs")
            ]
        );
    }

    #[test]
    fn query_values_are_decoded() {
        assert_eq!(
            query_value("workspace=repo%2D1&unused=x", "workspace").as_deref(),
            Some("repo-1")
        );
        assert_eq!(query_value("unused=x", "workspace"), None);
    }

    #[test]
    fn canvas_agent_launch_is_ephemeral_read_only_and_uses_sonnet_5() {
        assert_eq!(
            canvas_agent_args(),
            [
                "-p",
                "--model",
                "claude-sonnet-5",
                "--no-session-persistence",
                "--permission-mode",
                "plan",
                "--tools",
                "Read,Glob,Grep",
                "--disable-slash-commands",
                "--no-chrome",
                "--safe-mode",
            ]
        );
    }

    #[test]
    fn note_prompt_includes_only_transient_conversation_context() {
        let command = CanvasCommand {
            request_id: "request-1".into(),
            workspace: "repo-1".into(),
            note_id: "00000000-0000-4000-8000-000000000001".into(),
            scope: CanvasScope::Selection,
            intent: CanvasIntent::Analysis,
            prompt: "What calls this?".into(),
            paths: vec!["src/main.rs".into()],
            history: vec![CanvasTurn {
                question: "What is this?".into(),
                answer: "The program entry point.".into(),
            }],
        };

        let prompt = canvas_agent_prompt(&command);
        assert!(prompt.contains("src/main.rs"));
        assert!(prompt.contains("User: What is this?"));
        assert!(prompt.contains("Agent: The program entry point."));
        assert!(prompt.contains("Current question:\nWhat calls this?"));
        assert!(prompt.contains("Do not modify the repository."));
    }

    #[test]
    fn file_preview_is_text_only_and_stays_inside_workspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("image.bin"), [0, 1, 2, 3]).unwrap();
        let workspace = CanvasWorkspace::new("repo-1", "Example", dir.path().to_path_buf());

        let preview = read_file(&workspace, "src/main.rs").unwrap();
        assert_eq!(preview.path, "src/main.rs");
        assert_eq!(preview.language, "Rust");
        assert_eq!(preview.content, "fn main() {}\n");
        assert!(!preview.truncated);

        assert_eq!(read_file(&workspace, "../secret").unwrap_err().0, 400);
        assert_eq!(read_file(&workspace, "image.bin").unwrap_err().0, 415);
    }

    #[test]
    fn file_metadata_becomes_a_tree_entry() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        let entry = tree_entry(dir.path(), PathBuf::from("src/main.rs"), &HashMap::new()).unwrap();
        assert_eq!(entry.path, "src/main.rs");
        assert_eq!(entry.name, "main.rs");
        assert_eq!(entry.extension.as_deref(), Some("rs"));
        assert_eq!(entry.kind, "file");
        assert!(entry.size.unwrap() > 0);
    }

    #[test]
    fn loopback_server_serves_workspaces_and_their_tree() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        let server = CanvasServer::start(vec![CanvasWorkspace::new(
            "repo-1",
            "Example",
            dir.path().to_path_buf(),
        )])
        .unwrap();

        fn get(addr: SocketAddr, path: &str) -> String {
            let mut stream = std::net::TcpStream::connect(addr).unwrap();
            stream
                .write_all(format!("GET {path} HTTP/1.0\r\nHost: localhost\r\n\r\n").as_bytes())
                .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        }

        let workspaces = get(server.addr, "/api/workspaces");
        assert!(workspaces.starts_with("HTTP/1.0 200"), "{workspaces}");
        assert!(workspaces.contains("\"name\":\"Example\""));

        let tree = get(server.addr, "/api/tree?workspace=repo-1");
        assert!(tree.starts_with("HTTP/1.0 200"), "{tree}");
        assert!(tree.contains("src/main.rs"));

        let file = get(server.addr, "/api/file?workspace=repo-1&path=src%2Fmain.rs");
        assert!(file.starts_with("HTTP/1.0 200"), "{file}");
        assert!(file.contains("\"language\":\"Rust\""));
        assert!(file.contains("fn main() {}"));
    }

    #[test]
    fn loopback_server_queues_a_bounded_canvas_question_and_returns_its_answer() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        let server = CanvasServer::start(vec![CanvasWorkspace::new(
            "repo-1",
            "Example",
            dir.path().to_path_buf(),
        )])
        .unwrap();
        fn request(addr: SocketAddr, head: &str, body: &str) -> String {
            let mut stream = std::net::TcpStream::connect(addr).unwrap();
            stream
                .write_all(
                    format!(
                        "{head}\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        }

        let body = r#"{"workspace":"repo-1","note_id":"00000000-0000-4000-8000-000000000001","prompt":"Explain this entry point","paths":["src/main.rs"],"history":[{"question":"What is this?","answer":"A Rust entry point."}]}"#;
        let queued = request(server.addr, "POST /api/ask HTTP/1.0", body);
        assert!(queued.starts_with("HTTP/1.0 202"), "{queued}");
        let command = server.take_commands().pop().unwrap();
        assert_eq!(command.paths, vec!["src/main.rs"]);
        assert_eq!(command.prompt, "Explain this entry point");
        assert_eq!(command.scope, CanvasScope::Selection);
        assert_eq!(command.intent, CanvasIntent::Analysis);
        assert_eq!(command.note_id, "00000000-0000-4000-8000-000000000001");
        assert_eq!(command.history.len(), 1);

        server.mark_working(&command.request_id);
        server.complete(
            &command.request_id,
            r#"```workbench-canvas
{"answer":"This starts the program.","operations":[{"kind":"highlight","paths":["src/main.rs"],"color":"blue","label":"entry"}]}
```"#,
        );
        let answered = request(
            server.addr,
            &format!("GET /api/ask?id={} HTTP/1.0", command.request_id),
            "",
        );
        assert!(answered.starts_with("HTTP/1.0 200"), "{answered}");
        assert!(answered.contains("\"status\":\"complete\""));
        assert!(answered.contains("This starts the program."));
        assert!(answered.contains("\"kind\":\"highlight\""));

        let whole_repo = r#"{"workspace":"repo-1","note_id":"00000000-0000-4000-8000-000000000001","scope":"repository","prompt":"Explain the architecture","paths":[]}"#;
        let queued = request(server.addr, "POST /api/ask HTTP/1.0", whole_repo);
        assert!(queued.starts_with("HTTP/1.0 202"), "{queued}");
        let repository_command = server.take_commands().pop().unwrap();
        assert_eq!(repository_command.scope, CanvasScope::Repository);
        assert_eq!(repository_command.intent, CanvasIntent::Analysis);
        assert!(repository_command.paths.is_empty());
        server.complete(&repository_command.request_id, "A repository-level answer");
        let repository_answer = request(
            server.addr,
            &format!("GET /api/ask?id={} HTTP/1.0", repository_command.request_id),
            "",
        );
        assert!(repository_answer.contains("A repository-level answer"));
        assert!(repository_answer.contains("\"operations\":[]"));

        let architecture = r#"{"workspace":"repo-1","note_id":"00000000-0000-4000-8000-000000000001","scope":"repository","intent":"architecture","prompt":"Map the architecture","paths":[]}"#;
        let queued = request(server.addr, "POST /api/ask HTTP/1.0", architecture);
        assert!(queued.starts_with("HTTP/1.0 202"), "{queued}");
        let architecture_command = server.take_commands().pop().unwrap();
        assert_eq!(architecture_command.intent, CanvasIntent::Architecture);
        server.complete(
            &architecture_command.request_id,
            r#"```workbench-canvas
{"answer":"Mapped.","operations":[{"kind":"architecture","title":"Example architecture","summary":"A small Rust program.","level":"overview","nodes":[{"id":"entry","label":"Entry point","summary":"Starts the program.","kind":"runtime","color":"blue","paths":["src/main.rs"]},{"id":"imaginary","label":"Imaginary","paths":["../outside"]}],"edges":[{"from":"entry","to":"imaginary","label":"invalid"}]}]}
```"#,
        );
        let architecture_answer = request(
            server.addr,
            &format!(
                "GET /api/ask?id={} HTTP/1.0",
                architecture_command.request_id
            ),
            "",
        );
        assert!(architecture_answer.contains("\"kind\":\"architecture\""));
        assert!(architecture_answer.contains("\"id\":\"entry\""));
        assert!(!architecture_answer.contains("imaginary"));

        let escaping = r#"{"workspace":"repo-1","note_id":"00000000-0000-4000-8000-000000000001","prompt":"Read it","paths":["../secret"]}"#;
        let rejected = request(server.addr, "POST /api/ask HTTP/1.0", escaping);
        assert!(rejected.starts_with("HTTP/1.0 400"), "{rejected}");
    }

    #[test]
    fn canvas_response_falls_back_safely_when_the_envelope_is_invalid() {
        let (answer, operations) = parse_canvas_response("A useful plain-text answer");
        assert_eq!(answer, "A useful plain-text answer");
        assert!(operations.is_empty());

        let raw = r#"```workbench-canvas
{"answer":"Mapped.","operations":[{"kind":"connect","from":"src/a.rs","to":"src/b.rs","label":"calls"}]}
```"#;
        let (answer, mut operations) = parse_canvas_response(raw);
        sanitize_operations(&mut operations, None);
        assert_eq!(answer, "Mapped.");
        assert_eq!(operations.len(), 1);

        let raw = r#"```workbench-canvas
{"answer":"Architecture mapped.","operations":[{"kind":"architecture","title":"System","nodes":[{"id":"core","label":"Core","summary":"Runs the system.","paths":["src"]}],"edges":[]}]}
```"#;
        let (_, mut operations) = parse_canvas_response(raw);
        sanitize_operations(&mut operations, None);
        assert!(matches!(
            operations.as_slice(),
            [CanvasOperation::Architecture {
                level: ArchitectureLevel::Overview,
                ..
            }]
        ));
    }
}
