//! The control socket: newline-delimited JSON over a Unix domain socket.
//!
//! Everything workbench knows is already assembled once a tick for the phone
//! (`crate::remote::publish`), and everything a caller may *change* already has
//! a shape that survives the trip into the event loop (`RemoteCommand`). This
//! module is the third door onto those two things — after the TUI's keys and
//! the phone's HTTP — for scripts, editors, and agents driving workbench from
//! the inside.
//!
//! Two halves, deliberately different:
//!
//! - **Reads are answered from the published snapshot**, in the server thread,
//!   with no round trip through the event loop. They are therefore up to one
//!   tick stale and can never block the UI.
//! - **Writes are queued** as `RemoteCommand` and applied by the event loop on
//!   its own terms, exactly where the phone's writes land. A write returns
//!   `{"accepted":true}` — that the loop *took* it, not that the agent has
//!   answered. Anything else would mean holding a lock across a model turn.
//!
//! Events are the reason this exists rather than a polling endpoint. The tick
//! diffs each published snapshot against the last and pushes what moved, so a
//! caller can wait on `agent.status_changed` instead of asking every second.
//!
//! Unix-only: the socket is a Unix domain socket, and workbench's PTY layer is
//! already POSIX. The module compiles to nothing elsewhere.

use anyhow::{Result, anyhow};
use serde::Serialize;
use serde_json::{Value, json};
#[cfg(unix)]
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;

mod client;

pub use client::{resolve_agent, resolve_project, Client, Scope};

use crate::remote::{RemoteCommand, Shared, Snapshot};

/// Refuse a line longer than this. A control message is a few hundred bytes;
/// anything at this scale is a mistake or a wedge, and reading it to the end
/// would be the bug rather than the defence.
#[cfg(unix)]
const MAX_LINE_BYTES: u64 = 1 << 20;

/// A subscriber's backlog. A caller that stops reading while workbench keeps
/// ticking must not grow the queue without bound — past this it is dropped,
/// which it discovers as a closed socket.
#[cfg(unix)]
const MAX_PENDING_EVENTS: usize = 256;

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Fan-out to whoever has subscribed. Cloned into every connection thread.
#[derive(Clone, Default)]
pub struct EventHub {
    subscribers: Arc<Mutex<Vec<Sender<String>>>>,
}

impl EventHub {
    fn subscribe(&self) -> Receiver<String> {
        let (tx, rx) = channel();
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.push(tx);
        }
        rx
    }

    fn emit(&self, event: &str, data: Value) {
        let Ok(mut subscribers) = self.subscribers.lock() else {
            return;
        };
        if subscribers.is_empty() {
            return;
        }
        let line = json!({"event": event, "data": data}).to_string();
        // A send fails once the connection thread has dropped its receiver,
        // which is how a closed socket is noticed: prune on the way past.
        subscribers.retain(|subscriber| subscriber.send(line.clone()).is_ok());
    }

    fn is_idle(&self) -> bool {
        self.subscribers
            .lock()
            .map(|subscribers| subscribers.is_empty())
            .unwrap_or(true)
    }
}

/// One agent, reduced to the fields an event is about. Diffing the whole
/// `AgentView` would fire on every keystroke of terminal output.
#[derive(Clone, PartialEq, Eq)]
struct AgentMark {
    project: String,
    status: String,
    model: Option<String>,
    reason: Option<String>,
}

/// What the last tick published, so this one can say what moved.
#[derive(Default)]
pub struct EventState {
    agents: Vec<(String, AgentMark)>,
}

/// Compare the snapshot just published with the one before it and push the
/// difference. Called on the tick, after `remote::publish`.
///
/// Diffing is what keeps this honest: the alternative is emitting from the
/// dozen places that can change an agent's state, and the one that gets
/// forgotten is a caller waiting forever.
pub fn publish_events(hub: &EventHub, previous: &mut EventState, snapshot: &Snapshot) {
    // Nobody is listening: keep the marks current so the first subscriber does
    // not receive the entire backlog as "changes", but skip the formatting.
    let quiet = hub.is_idle();

    let current: Vec<(String, AgentMark)> = snapshot
        .agents
        .iter()
        .map(|agent| {
            (
                agent.id.clone(),
                AgentMark {
                    project: agent.project.clone(),
                    status: agent.status.clone(),
                    model: agent.model.clone(),
                    reason: agent.reason.clone(),
                },
            )
        })
        .collect();

    if !quiet {
        for (id, mark) in &current {
            match previous.agents.iter().find(|(known, _)| known == id) {
                None => hub.emit(
                    "agent.added",
                    json!({"agent": id, "project": mark.project, "status": mark.status}),
                ),
                Some((_, was)) if was != mark => {
                    if was.status != mark.status {
                        hub.emit(
                            "agent.status_changed",
                            json!({
                                "agent": id,
                                "project": mark.project,
                                "from": was.status,
                                "to": mark.status,
                                "reason": mark.reason,
                            }),
                        );
                    }
                    if was.model != mark.model {
                        hub.emit(
                            "agent.model_changed",
                            json!({"agent": id, "project": mark.project, "model": mark.model}),
                        );
                    }
                }
                Some(_) => {}
            }
        }
        for (id, mark) in &previous.agents {
            if !current.iter().any(|(known, _)| known == id) {
                hub.emit("agent.removed", json!({"agent": id, "project": mark.project}));
            }
        }
    }

    previous.agents = current;
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

/// The listening socket. Dropping it removes the socket file.
pub struct ControlServer {
    path: PathBuf,
    pub hub: EventHub,
}

impl ControlServer {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The environment variable naming the socket. Injected into every PTY
/// workbench spawns, so an agent running inside workbench can find the socket
/// without being told where it is — and so overriding it moves the socket.
pub const ENV_SOCKET: &str = "WORKBENCH_CONTROL_SOCK";

/// A `sockaddr_un` carries about 104 bytes of path on macOS and 108 on Linux,
/// and `bind` fails outright past that. Stay under the smaller one.
const MAX_SOCKET_PATH: usize = 100;

/// Where the socket lives. Beside the rest of workbench's state, so a client
/// with no arguments can find it.
///
/// With a fallback, because that path is not always usable: a socket address
/// is a fixed-size buffer, and on macOS the config directory is the roomy
/// `~/Library/Application Support/…`. A long enough home — a test harness, a
/// sandbox, a network account — pushes it past the limit, and `bind` fails
/// with nothing that reads like a path-length problem. When that happens the
/// socket moves to the temp directory, which macOS already makes per-user.
pub fn socket_path() -> Result<PathBuf> {
    if let Some(explicit) = std::env::var_os(ENV_SOCKET) {
        return Ok(PathBuf::from(explicit));
    }
    let preferred = dirs::config_dir()
        .ok_or_else(|| anyhow!("could not locate config directory"))?
        .join("workbench")
        .join("control.sock");
    if preferred.as_os_str().len() <= MAX_SOCKET_PATH {
        return Ok(preferred);
    }
    // Per-user by construction on macOS (`/var/folders/…`), and the 0600 mode
    // set at bind covers the shared `/tmp` that some Linuxes hand back.
    #[cfg(unix)]
    {
        let uid = unsafe { libc::getuid() };
        Ok(std::env::temp_dir().join(format!("workbench-control-{uid}.sock")))
    }
    #[cfg(not(unix))]
    Ok(preferred)
}

/// Start listening. Errors are the caller's to log and shrug at: the TUI runs
/// perfectly well without a control socket, exactly as it does without a phone.
///
/// Windows gets the error rather than a socket. Named pipes would serve, but
/// nothing here has been run against them, and a stub that claims to work is
/// worse than one that says it does not.
#[cfg(not(unix))]
pub fn start(
    _shared: Shared,
    _commands: UnboundedSender<RemoteCommand>,
) -> Result<ControlServer> {
    Err(anyhow!("the control socket needs a Unix socket"))
}

#[cfg(unix)]
pub fn start(
    shared: Shared,
    commands: UnboundedSender<RemoteCommand>,
) -> Result<ControlServer> {
    let path = socket_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // A socket file outlives the process that made it, so one left by a crash
    // would keep every future workbench from binding. Connecting is the only
    // way to tell a live server from a dead file: if nobody answers, the file
    // is debris and clearing it is safe. If somebody does, another workbench
    // owns this machine's socket and we leave it alone.
    if path.exists() {
        if UnixStream::connect(&path).is_ok() {
            return Err(anyhow!(
                "another workbench already owns {}",
                path.display()
            ));
        }
        std::fs::remove_file(&path)?;
    }

    let listener = UnixListener::bind(&path)?;
    // The socket can type into every agent on the machine. Default umask would
    // usually land on 0755 here; say 0600 outright rather than inherit it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }

    let hub = EventHub::default();
    let server = ControlServer {
        path: path.clone(),
        hub: hub.clone(),
    };

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                continue;
            };
            let shared = shared.clone();
            let commands = commands.clone();
            let hub = hub.clone();
            std::thread::spawn(move || {
                if let Err(err) = serve(stream, shared, commands, hub) {
                    crate::logger::warn(format!("control connection ended: {err}"));
                }
            });
        }
    });

    Ok(server)
}

#[cfg(unix)]
/// One connection: read a request per line, answer it on the same line-based
/// stream. A subscription hands the write half to its own thread so events
/// keep flowing while this one goes on reading requests.
fn serve(
    stream: UnixStream,
    shared: Shared,
    commands: UnboundedSender<RemoteCommand>,
    hub: EventHub,
) -> Result<()> {
    let mut out = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let mut subscribed = false;

    loop {
        // Capped per line rather than per connection: a control connection is
        // meant to stay open for hours, so a budget spanning all of it would
        // eventually strangle a healthy caller.
        let mut line = String::new();
        let read = (&mut reader).take(MAX_LINE_BYTES).read_line(&mut line)?;
        if read == 0 {
            break; // the caller hung up
        }
        if !line.ends_with('\n') && read as u64 == MAX_LINE_BYTES {
            reply(
                &mut out,
                &Value::Null,
                Err(("too_long", "request exceeded 1 MiB".into())),
            )?;
            break; // the rest of that line would parse as garbage
        }
        if line.trim().is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(err) => {
                reply(&mut out, &Value::Null, Err(("bad_json", err.to_string())))?;
                continue;
            }
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let params = request.get("params").cloned().unwrap_or(json!({}));

        if method == "events.subscribe" {
            if subscribed {
                reply(&mut out, &id, Ok(json!({"subscribed": true})))?;
                continue;
            }
            subscribed = true;
            let events = hub.subscribe();
            let mut sink = out.try_clone()?;
            std::thread::spawn(move || {
                let mut pending = 0usize;
                for event in events {
                    pending += 1;
                    if pending > MAX_PENDING_EVENTS {
                        break;
                    }
                    if writeln!(sink, "{event}").is_err() || sink.flush().is_err() {
                        break;
                    }
                    pending = 0;
                }
            });
            reply(&mut out, &id, Ok(json!({"subscribed": true})))?;
            continue;
        }

        let result = dispatch(method, &params, &shared, &commands);
        if reply(&mut out, &id, result).is_err() {
            // The caller hung up without reading — which the hook forwarder
            // does by design, since nothing acts on the answer and a hook that
            // waits is a hook that slows the agent down. Not worth a line in
            // the log 24 times a second.
            return Ok(());
        }
    }

    Ok(())
}

#[cfg(unix)]
fn reply(
    out: &mut UnixStream,
    id: &Value,
    result: std::result::Result<Value, (&'static str, String)>,
) -> Result<()> {
    let body = match result {
        Ok(result) => json!({"id": id, "result": result}),
        Err((code, message)) => json!({"id": id, "error": {"code": code, "message": message}}),
    };
    writeln!(out, "{body}")?;
    out.flush()?;
    Ok(())
}

type Answer = std::result::Result<Value, (&'static str, String)>;

fn dispatch(
    method: &str,
    params: &Value,
    shared: &Shared,
    commands: &UnboundedSender<RemoteCommand>,
) -> Answer {
    match method {
        "api.schema" => Ok(schema()),
        "state.get" => with_snapshot(shared, |snapshot| Ok(to_value(snapshot))),
        "agents.list" => with_snapshot(shared, |snapshot| {
            Ok(json!(
                snapshot.agents.iter().map(summarize).collect::<Vec<_>>()
            ))
        }),
        "agent.get" => {
            let wanted = text_param(params, "agent")?;
            with_snapshot(shared, |snapshot| {
                snapshot
                    .agents
                    .iter()
                    .find(|agent| agent.id.eq_ignore_ascii_case(&wanted))
                    .map(|agent| to_value(agent))
                    .ok_or_else(|| ("no_such_agent", format!("no agent {wanted}")))
            })
        }
        "projects.list" => with_snapshot(shared, |snapshot| Ok(to_value(&snapshot.projects))),

        "agent.prompt" => queue(
            commands,
            RemoteCommand::Reply {
                agent: text_param(params, "agent")?,
                text: text_param(params, "text")?,
            },
        ),
        "agent.todo" => queue(
            commands,
            RemoteCommand::Todo {
                agent: text_param(params, "agent")?,
                text: text_param(params, "text")?,
            },
        ),
        "agent.answer" => queue(
            commands,
            RemoteCommand::Answer {
                agent: text_param(params, "agent")?,
                key: text_param(params, "key")?,
            },
        ),
        "agent.focus" => queue(
            commands,
            RemoteCommand::Focus {
                agent: text_param(params, "agent")?,
            },
        ),
        "agent.new" => queue(
            commands,
            RemoteCommand::NewAgent {
                project: text_param(params, "project")?,
                provider: text_param(params, "provider")?,
            },
        ),

        // The hook fast path (see `src/bin/wbhook.rs`). Interpreting the
        // event stays here rather than in the little forwarder, so the rule
        // for what an event means lives in one place.
        "hook" => {
            let workspace = text_param(params, "workspace")?;
            let session = text_param(params, "session")?;
            let raw = params.get("payload").and_then(Value::as_str).unwrap_or("");
            let payload: Option<Value> = serde_json::from_str(raw).ok();
            // Codex's hook command carries no arguments and names the event in
            // the payload instead; Claude passes it as one.
            let named = params.get("event").and_then(Value::as_str).unwrap_or("");
            let event = if named.is_empty() {
                payload
                    .as_ref()
                    .and_then(|p| p.get("hook_event_name"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
            } else {
                named
            };
            if event.is_empty() {
                return Ok(json!({"ignored": "no event"}));
            }
            match crate::agent_status::interpret(event, payload.as_ref()) {
                Some(status) => {
                    crate::agent_status::record(&workspace, &session, &status)
                        .map_err(|err| ("record_failed", err.to_string()))?;
                    Ok(json!({"recorded": true}))
                }
                None => Ok(json!({"ignored": event})),
            }
        }

        "" => Err(("no_method", "every request needs a `method`".into())),
        other => Err((
            "unknown_method",
            format!("no method {other} — try api.schema"),
        )),
    }
}

fn with_snapshot(shared: &Shared, read: impl FnOnce(&Snapshot) -> Answer) -> Answer {
    match shared.lock() {
        Ok(snapshot) => read(&snapshot),
        Err(_) => Err(("unavailable", "state is momentarily unreadable".into())),
    }
}

/// A write is accepted, not performed: the event loop applies it on its own
/// tick. Saying so in the reply is the honest version of a status code.
fn queue(commands: &UnboundedSender<RemoteCommand>, command: RemoteCommand) -> Answer {
    match commands.send(command) {
        Ok(()) => Ok(json!({"accepted": true})),
        Err(_) => Err(("shutting_down", "workbench is going away".into())),
    }
}

fn text_param(params: &Value, key: &'static str) -> std::result::Result<String, (&'static str, String)> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ("bad_params", format!("`{key}` is required")))
}

fn to_value<T: Serialize>(value: T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// The compact form: what an agent *is*, without its conversation. `state.get`
/// is there for callers that want everything.
fn summarize(agent: &crate::remote::AgentView) -> Value {
    json!({
        "id": agent.id,
        "project": agent.project,
        "project_id": agent.project_id,
        "provider": agent.provider,
        "alias": agent.alias,
        "model": agent.model,
        "status": agent.status,
        "reason": agent.reason,
        "running": agent.running,
        "queued": agent.queued.len(),
        "paused": agent.paused,
        "blocked_on": agent.prompt.as_ref().map(|_| true).unwrap_or(false),
    })
}

/// Self-description, so a caller can discover the surface without the docs —
/// the same reason `herdr api schema` exists.
fn schema() -> Value {
    json!({
        "methods": [
            {"name": "api.schema", "params": [], "kind": "read"},
            {"name": "state.get", "params": [], "kind": "read"},
            {"name": "agents.list", "params": [], "kind": "read"},
            {"name": "agent.get", "params": ["agent"], "kind": "read"},
            {"name": "projects.list", "params": [], "kind": "read"},
            {"name": "agent.prompt", "params": ["agent", "text"], "kind": "write"},
            {"name": "agent.todo", "params": ["agent", "text"], "kind": "write"},
            {"name": "agent.answer", "params": ["agent", "key"], "kind": "write"},
            {"name": "agent.focus", "params": ["agent"], "kind": "write"},
            {"name": "agent.new", "params": ["project", "provider"], "kind": "write"},
            {"name": "events.subscribe", "params": [], "kind": "stream"},
            {"name": "hook", "params": ["workspace", "session", "event", "payload"], "kind": "write"}
        ],
        "events": [
            "agent.added",
            "agent.removed",
            "agent.status_changed",
            "agent.model_changed"
        ],
        "notes": "Reads answer from the last published snapshot (up to one tick old). Writes are queued for the event loop and answer {\"accepted\":true}."
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::{AgentView, ProjectView};

    fn snapshot_with(agents: Vec<AgentView>) -> Shared {
        Arc::new(Mutex::new(Snapshot {
            projects: vec![ProjectView {
                id: "p1".into(),
                name: "workbench".into(),
                servers: Vec::new(),
            }],
            agents,
            open: None,
            at: 0,
        }))
    }

    fn agent(id: &str, status: &str) -> AgentView {
        AgentView {
            id: id.into(),
            project: "workbench".into(),
            project_id: "p1".into(),
            provider: "Claude".into(),
            alias: None,
            model: Some("Opus 5".into()),
            status: status.into(),
            reason: None,
            running: None,
            steps: Vec::new(),
            queued: Vec::new(),
            paused: false,
            holding: None,
            prompt: None,
            messages: Vec::new(),
            msg_total: 0,
            msg_reset: false,
            tail: Vec::new(),
            finished_ago: None,
        }
    }

    fn channel_pair() -> (
        UnboundedSender<RemoteCommand>,
        tokio::sync::mpsc::UnboundedReceiver<RemoteCommand>,
    ) {
        tokio::sync::mpsc::unbounded_channel()
    }

    #[test]
    fn reads_are_answered_from_the_snapshot() {
        let shared = snapshot_with(vec![agent("abc12345", "working")]);
        let (tx, _rx) = channel_pair();

        let listed = dispatch("agents.list", &json!({}), &shared, &tx).unwrap();
        assert_eq!(listed[0]["id"], "abc12345");
        assert_eq!(listed[0]["status"], "working");
        assert_eq!(listed[0]["model"], "Opus 5");

        let one = dispatch("agent.get", &json!({"agent": "abc12345"}), &shared, &tx).unwrap();
        assert_eq!(one["provider"], "Claude");
    }

    /// A write must reach the event loop rather than being applied here: the
    /// server thread holds no lock on app state and never should.
    #[test]
    fn a_write_is_queued_for_the_event_loop() {
        let shared = snapshot_with(vec![agent("abc12345", "idle")]);
        let (tx, mut rx) = channel_pair();

        let answer = dispatch(
            "agent.prompt",
            &json!({"agent": "abc12345", "text": "ship it"}),
            &shared,
            &tx,
        )
        .unwrap();
        assert_eq!(answer["accepted"], true);

        match rx.try_recv().unwrap() {
            RemoteCommand::Reply { agent, text } => {
                assert_eq!(agent, "abc12345");
                assert_eq!(text, "ship it");
            }
            other => panic!("expected a reply, got {other:?}"),
        }
    }

    #[test]
    fn a_bad_request_is_an_error_not_a_panic() {
        let shared = snapshot_with(Vec::new());
        let (tx, _rx) = channel_pair();

        assert_eq!(
            dispatch("agent.prompt", &json!({"text": "hi"}), &shared, &tx).unwrap_err().0,
            "bad_params"
        );
        assert_eq!(
            dispatch("nope", &json!({}), &shared, &tx).unwrap_err().0,
            "unknown_method"
        );
        assert_eq!(
            dispatch("agent.get", &json!({"agent": "ghost"}), &shared, &tx)
                .unwrap_err()
                .0,
            "no_such_agent"
        );
    }

    /// The point of the socket: a caller can wait to be told, rather than ask
    /// every second. Only the fields an event is about may fire one.
    #[test]
    fn a_status_change_is_pushed_to_subscribers() {
        let hub = EventHub::default();
        let events = hub.subscribe();
        let mut marks = EventState::default();

        let first = Snapshot {
            agents: vec![agent("abc12345", "working")],
            ..Default::default()
        };
        publish_events(&hub, &mut marks, &first);
        let added: Value = serde_json::from_str(&events.recv().unwrap()).unwrap();
        assert_eq!(added["event"], "agent.added");
        assert_eq!(added["data"]["agent"], "abc12345");

        // Same status, and the conversation moved on underneath: silence.
        let mut quiet = agent("abc12345", "working");
        quiet.messages = Vec::new();
        quiet.finished_ago = Some(4);
        publish_events(
            &hub,
            &mut marks,
            &Snapshot {
                agents: vec![quiet],
                ..Default::default()
            },
        );
        assert!(
            events.try_recv().is_err(),
            "output churn must not look like a state change"
        );

        publish_events(
            &hub,
            &mut marks,
            &Snapshot {
                agents: vec![agent("abc12345", "idle")],
                ..Default::default()
            },
        );
        let changed: Value = serde_json::from_str(&events.recv().unwrap()).unwrap();
        assert_eq!(changed["event"], "agent.status_changed");
        assert_eq!(changed["data"]["agent"], "abc12345");
        assert_eq!(changed["data"]["from"], "working");
        assert_eq!(changed["data"]["to"], "idle");

        publish_events(&hub, &mut marks, &Snapshot::default());
        let removed: Value = serde_json::from_str(&events.recv().unwrap()).unwrap();
        assert_eq!(removed["event"], "agent.removed");
    }

    /// With nobody subscribed the marks still have to advance, or the first
    /// subscriber is handed every agent that ever existed as "news".
    #[test]
    fn a_late_subscriber_does_not_inherit_the_backlog() {
        let hub = EventHub::default();
        let mut marks = EventState::default();
        publish_events(
            &hub,
            &mut marks,
            &Snapshot {
                agents: vec![agent("abc12345", "working")],
                ..Default::default()
            },
        );

        let events = hub.subscribe();
        publish_events(
            &hub,
            &mut marks,
            &Snapshot {
                agents: vec![agent("abc12345", "working")],
                ..Default::default()
            },
        );
        assert!(events.try_recv().is_err(), "nothing moved, nothing said");
    }
}
