//! The HTTP server thread and its routes.
//!
//! Bound to the Tailscale address only. That is the security boundary that
//! matters — a café network cannot reach it even with the firewall off — and
//! the token is the second one, for the case of another device on your own
//! tailnet. Nothing here is ever exposed with `tailscale funnel`.

use super::commands::Request as CommandRequest;
use super::http::Request;
use anyhow::{anyhow, Result};
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tiny_http::{Header, Response};
use tokio::sync::mpsc;

use super::{page, Shared};
use crate::app::Action;

/// Where the server listens, and the token it demands.
#[derive(Debug, Clone)]
pub struct RemoteConfig {
    pub addr: SocketAddr,
    pub token: String,
}

impl RemoteConfig {
    /// The URL to put on your phone's home screen.
    pub fn url(&self) -> String {
        format!("http://{}/?t={}", self.addr, self.token)
    }
}

/// A running server, kept alive by holding this.
pub struct Remote {
    pub config: RemoteConfig,
}

/// What the phone asked for, resolved against app state by the event loop.
///
/// The server never touches `AppState`; it sends one of these and the loop
/// applies it, so a request cannot race the UI.
#[derive(Debug, Clone)]
pub enum RemoteCommand {
    ShowMedia {
        id: String,
    },
    /// Queue work for an agent.
    Todo {
        agent: String,
        text: String,
    },
    /// Type a reply and submit it.
    Reply {
        agent: String,
        text: String,
    },
    /// Pick one of the choices the agent is offering. `key` is the option's
    /// own key as it appears on screen ("1", "2", …) or "esc" to back out.
    Answer {
        agent: String,
        key: String,
        prompt: String,
    },
    /// The conversation the phone currently has open. Only this agent's full
    /// history is published, so the snapshot stays small.
    Focus {
        agent: String,
    },
    /// Start a new agent in a project. `agent` carries the project id and
    /// `text` the provider, since every write endpoint speaks that shape.
    NewAgent {
        project: String,
        provider: String,
    },
    /// Run one of a project's repeatable jobs (see `app::jobs`). `project`
    /// is the project id and `job` the job id from its manifest.
    RunJob {
        project: String,
        job: String,
    },
    /// A device asking to be told when an agent needs you.
    Subscribe {
        endpoint: String,
    },
    /// The user deciding a proposal from the phone: the same act as `a`/`x`
    /// at the desk. `proposal` is the full id; approving queues the work.
    Decide {
        proposal: String,
        approve: bool,
    },
    /// The user deciding a manager's proposed done-when check: the same act
    /// as `a`/`x` on that row of the desk. Approving makes the command real,
    /// declining drops it. `objective` is the full objective id, because a
    /// check has no id of its own — it belongs to the objective it proves.
    DecideCheck {
        objective: String,
        approve: bool,
    },
    /// Re-arming a review the manager punted to the user. The user's approval
    /// buys what the manager's could not, so the work goes back to the agent
    /// with a fresh set of rounds and the findings attached. Declining one
    /// instead is an ordinary `Decide { approve: false }` — there is no
    /// second way to say no.
    RearmReview {
        proposal: String,
    },
    /// A manager answering its review turn. The one write a manager is
    /// allowed that reaches an agent — and only because approving the
    /// original job authorized exactly this loop.
    Review {
        manager: String,
        proposal: String,
        /// "accept" | "request_changes" | "needs_user"
        outcome: String,
        findings: String,
    },
    /// A manager's suggestion for how an objective would be checked. Stored
    /// as proposed, which is to say: not yet something anything will be held
    /// to. Approving it is the user's step.
    ProposeCheck {
        manager: String,
        objective: String,
        command: String,
    },
    /// A manager's suggestion. Recorded, never acted on: turning one into work
    /// is a separate step the user takes.
    Propose {
        manager: String,
        objective: Option<String>,
        agent: Option<String>,
        instruction: String,
        rationale: String,
    },
}

/// This machine's Tailscale address, if it is on a tailnet.
///
/// Binding here rather than `0.0.0.0` is deliberate: the page is reachable
/// from your own devices and from nothing else, with no firewall rule to get
/// wrong.
pub fn tailscale_addr() -> Option<IpAddr> {
    let output = Command::new("tailscale")
        .arg("ip")
        .arg("-4")
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()?
        .trim()
        .parse()
        .ok()
}

/// A token that is easy to keep in a bookmark and hard to guess.
pub fn new_token() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..24)
        .map(|_| {
            const ALPHABET: &[u8] = b"abcdefghijkmnopqrstuvwxyz23456789";
            ALPHABET[rng.gen_range(0..ALPHABET.len())] as char
        })
        .collect()
}

impl Remote {
    /// Start serving, or explain why we cannot.
    ///
    /// `push_key` is the VAPID public key the phone subscribes with. It is
    /// fixed for the life of the process, so the server thread holds a copy
    /// rather than reaching into app state for it.
    pub fn start(
        port: u16,
        token: String,
        push_key: String,
        shared: Shared,
        commands: mpsc::UnboundedSender<CommandRequest>,
        _actions: mpsc::UnboundedSender<Action>,
    ) -> Result<Remote> {
        let ip = tailscale_addr()
            .ok_or_else(|| anyhow!("no Tailscale address; is tailscale running?"))?;
        let addr = SocketAddr::new(ip, port);
        let config = RemoteConfig { addr, token };

        serve_on(addr, &config.token, &push_key, &shared, &commands)
            .map_err(|err| anyhow!("could not bind {addr}: {err}"))?;

        // Also on loopback, so `tailscale serve` — which proxies to
        // 127.0.0.1 — can put HTTPS in front. That is what unlocks
        // dictation, which browsers refuse outside a secure context.
        let loopback = SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), port);
        if let Err(err) = serve_on(loopback, &config.token, &push_key, &shared, &commands) {
            crate::logger::warn(format!("phone view not on loopback: {err}"));
        }

        Ok(Remote { config })
    }
}

/// Bind one address and answer requests on it until the process ends.
fn serve_on(
    addr: SocketAddr,
    token: &str,
    push_key: &str,
    shared: &Shared,
    commands: &mpsc::UnboundedSender<CommandRequest>,
) -> Result<()> {
    start_workers(TcpListener::bind(addr)?, token, push_key, shared, commands);
    Ok(())
}

fn start_workers(
    listener: TcpListener,
    token: &str,
    push_key: &str,
    shared: &Shared,
    commands: &mpsc::UnboundedSender<CommandRequest>,
) {
    let listener = Arc::new(listener);
    for _ in 0..8 {
        let listener = listener.clone();
        let (token, shared, commands) = (token.to_string(), shared.clone(), commands.clone());
        let push_key = push_key.to_string();
        std::thread::spawn(move || {
            for connection in listener.incoming() {
                let Ok(mut stream) = connection else {
                    continue;
                };
                let mut head = false;
                let response = match Request::read(&mut stream) {
                    Ok(mut request) => {
                        head = request.method().as_str() == "HEAD";
                        if request.url().starts_with("/media/") {
                            let library = shared
                                .lock()
                                .map(|s| s.media_library.clone())
                                .unwrap_or_default();
                            library.response(request.url(), &request, &token)
                        } else {
                            handle(&mut request, &token, &push_key, &shared, &commands).boxed()
                        }
                    }
                    Err(err) => status(400, &format!("Could not read request: {err}")).boxed(),
                }
                .with_header(header("Connection", "close"));
                let _ =
                    response.raw_print(&mut stream, tiny_http::HTTPVersion(1, 1), &[], head, None);
                let _ = std::io::Write::flush(&mut stream);
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
        });
    }
}

/// IBM Plex Mono, compiled in rather than fetched from Google.
///
/// The usual way to use it is a stylesheet on `fonts.googleapis.com` pulling
/// files from `fonts.gstatic.com`, which would make a page whose whole point
/// is working over a private tailnet depend on the public internet to render
/// its own text — and hand every load to a third party. The latin subsets are
/// 9.8KB apiece, so there is nothing to weigh up: they ship in the binary.
///
/// SIL Open Font License 1.1. `assets/fonts/OFL.txt` travels with them, as
/// the licence requires.
const PLEX_MONO_400: &[u8] = include_bytes!("../../assets/fonts/ibm-plex-mono-400.woff2");
const PLEX_MONO_500: &[u8] = include_bytes!("../../assets/fonts/ibm-plex-mono-500.woff2");

fn font_for(path: &str) -> Option<&'static [u8]> {
    match path {
        "/font/ibm-plex-mono-400.woff2" => Some(PLEX_MONO_400),
        "/font/ibm-plex-mono-500.woff2" => Some(PLEX_MONO_500),
        _ => None,
    }
}

/// A binary body, cached hard: these change only when the binary does.
fn bytes(body: &'static [u8], content_type: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_data(body)
        .with_header(header("Content-Type", content_type))
        .with_header(header(
            "Cache-Control",
            "public, max-age=31536000, immutable",
        ))
}

fn json(body: String) -> Response<std::io::Cursor<Vec<u8>>> {
    let header = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
        .expect("static header parses");
    Response::from_string(body).with_header(header)
}

fn html(body: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    with_type(body, "text/html; charset=utf-8")
}

fn with_type(body: &str, content_type: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(body.to_string()).with_header(header("Content-Type", content_type))
}

fn status(code: u16, message: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(message.to_string()).with_status_code(code)
}

fn handle(
    request: &mut Request,
    token: &str,
    push_key: &str,
    shared: &Shared,
    commands: &mpsc::UnboundedSender<CommandRequest>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let url = request.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));

    // Fonts likewise: a `src:` in a stylesheet is fetched without the query
    // string that carries the token, and a typeface is not a secret.
    if let Some(font) = font_for(path) {
        return bytes(font, "font/woff2");
    }
    if !authorized(request, &query_params(query), token) {
        return status(401, "unauthorized");
    }

    if request.method().as_str() == "POST" && path != "/api/upload" {
        let Some(instance) = request
            .headers()
            .iter()
            .find(|h| h.field.equiv("X-Workbench-Instance"))
        else {
            return status(409, "Reload the page before sending this request.");
        };
        if shared
            .lock()
            .ok()
            .is_none_or(|snapshot| snapshot.instance != instance.value.as_str())
        {
            return status(
                409,
                "Workbench restarted. Check the conversation before sending this again.",
            );
        }
    }
    match (request.method().as_str(), path) {
        ("GET", "/") => html(page::HTML),
        ("GET", "/manifest.webmanifest") => {
            let mut manifest: serde_json::Value =
                serde_json::from_str(page::MANIFEST).expect("valid embedded manifest");
            manifest["start_url"] = serde_json::json!(format!("./?t={token}"));
            with_type(&manifest.to_string(), "application/manifest+json")
        }
        // Registered as `/sw.js?t=…` so the worker inherits the token and can
        // read state when a notification arrives.
        ("GET", "/sw.js") => with_type(page::SERVICE_WORKER, "text/javascript; charset=utf-8"),
        ("GET", "/api/push-key") => with_type(push_key, "text/plain; charset=utf-8"),
        ("POST", "/api/subscribe") => command_from(request, commands, |_, endpoint| {
            (!endpoint.is_empty()).then_some(RemoteCommand::Subscribe { endpoint })
        }),
        ("POST", "/api/upload") => {
            let params = query_params(query);
            let agent = params
                .iter()
                .find(|(key, _)| key == "agent")
                .map(|(_, value)| value.as_str());
            if !shared
                .lock()
                .ok()
                .is_some_and(|state| state.agents.iter().any(|a| Some(a.id.as_str()) == agent))
            {
                return status(404, "This agent no longer exists.");
            }
            upload(request, &params)
        }
        ("GET", "/api/state") => state_body(request, &query_params(query), shared),
        ("POST", "/api/todo") => command_from(request, commands, |agent, text| {
            (!text.is_empty()).then_some(RemoteCommand::Todo { agent, text })
        }),
        ("POST", "/api/reply") => command_from(request, commands, |agent, text| {
            (!text.is_empty()).then_some(RemoteCommand::Reply { agent, text })
        }),
        ("POST", "/api/answer") => {
            let prompt = request
                .headers()
                .iter()
                .find(|h| h.field.equiv("X-Prompt-Id"))
                .map(|h| h.value.as_str().to_string())
                .unwrap_or_default();
            command_from(request, commands, |agent, key| {
                (!key.is_empty() && !prompt.is_empty()).then_some(RemoteCommand::Answer {
                    agent,
                    key,
                    prompt,
                })
            })
        }
        ("POST", "/api/focus") => command_from(request, commands, |agent, _| {
            Some(RemoteCommand::Focus { agent })
        }),
        ("POST", "/api/proposal") => command_from(request, commands, |proposal, decision| {
            Some(RemoteCommand::Decide {
                proposal,
                approve: decision == "approve",
            })
        }),
        // The desk's other two decisions. Same two-field shape as every
        // other write endpoint, so the page posts them the same way.
        ("POST", "/api/check") => command_from(request, commands, |objective, decision| {
            Some(RemoteCommand::DecideCheck {
                objective,
                approve: decision == "approve",
            })
        }),
        ("POST", "/api/rearm") => command_from(request, commands, |proposal, _| {
            Some(RemoteCommand::RearmReview { proposal })
        }),
        ("POST", "/api/new-agent") => command_from(request, commands, |project, provider| {
            Some(RemoteCommand::NewAgent { project, provider })
        }),
        ("POST", "/api/job") => command_from(request, commands, |project, job| {
            Some(RemoteCommand::RunJob { project, job })
        }),
        _ => status(404, "not found"),
    }
}

/// The snapshot, minus whatever the caller already has.
///
/// Two savings, and the phone is polling once a second on a cellular radio, so
/// both are worth having: `?have=` drops the messages it already holds, and an
/// ETag turns a tick where nothing at all moved into a 304 with no body.
fn state_body(
    request: &Request,
    params: &[(String, String)],
    shared: &Shared,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let have = params
        .iter()
        .find(|(key, _)| key == "have")
        .and_then(|(_, value)| value.parse::<usize>().ok());
    let epoch = params
        .iter()
        .find(|(key, _)| key == "epoch")
        .map(|(_, value)| value.as_str());

    let mut snapshot = {
        let Ok(mut snapshot) = shared.lock() else {
            return status(500, "state unavailable");
        };
        let wanted = params
            .iter()
            .find(|(key, _)| key == "agent")
            .map(|(_, value)| value.clone())
            .or_else(|| snapshot.open.clone());
        if let Some(id) = &wanted {
            if snapshot.agents.iter().any(|agent| &agent.id == id) {
                snapshot.watching.insert(id.clone(), Instant::now());
            }
        }
        let mut selected = snapshot.clone();
        selected.open = wanted;
        for agent in &mut selected.agents {
            if Some(&agent.id) != selected.open.as_ref() {
                agent.messages.clear();
                agent.tail.clear();
                agent.msg_total = 0;
                agent.msg_epoch.clear();
            }
        }
        selected
    };
    snapshot = match have {
        Some(have) => super::since(&snapshot, have, epoch),
        None => snapshot,
    };
    let body = serde_json::to_string(&snapshot).unwrap_or_default();
    // Heartbeat and elapsed seconds are metadata, not new content. An agent
    // entering/leaving the recent-finish window still changes the ETag.
    snapshot.at = 0;
    for agent in &mut snapshot.agents {
        agent.finished_ago = agent.finished_ago.map(|_| 0);
    }
    let tag = etag(&serde_json::to_string(&snapshot).unwrap_or_default());
    let known = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("If-None-Match"))
        .map(|header| header.value.as_str().to_string());
    if known.as_deref() == Some(tag.as_str()) {
        return Response::from_string(String::new())
            .with_status_code(304)
            .with_header(header("ETag", &tag));
    }
    json(body).with_header(header("ETag", &tag))
}

fn etag(body: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    body.hash(&mut hasher);
    format!("\"{:x}\"", hasher.finish())
}

fn header(field: &str, value: &str) -> Header {
    Header::from_bytes(field.as_bytes(), value.as_bytes()).expect("well-formed header")
}

/// The token may travel in the bookmark's query string or a header. Both are
/// fine over the tailnet; the query form is what makes a home-screen icon work.
fn authorized(request: &Request, params: &[(String, String)], token: &str) -> bool {
    if params
        .iter()
        .any(|(key, value)| key == "t" && value == token)
    {
        return true;
    }
    request.headers().iter().any(|header| {
        header.field.equiv("Authorization") && header.value.as_str() == format!("Bearer {token}")
    })
}

fn query_params(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (key.to_string(), percent_decode(value))
        })
        .collect()
}

/// Enough decoding for a token and a short id; bodies are JSON, not forms.
fn percent_decode(value: &str) -> String {
    let bytes = value.replace('+', " ").into_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&String::from_utf8_lossy(&bytes[i + 1..i + 3]), 16)
            {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A phone photo is a few megabytes; anything much past that is not a
/// screenshot of a bug and should not be quietly written to disk.
const MAX_UPLOAD: usize = 25 * 1024 * 1024;

/// Take a file from the phone and put it somewhere the agent can read.
///
/// Agents take images by path — Claude's Read renders one, and Codex has its
/// own viewer — so the useful thing to hand back is the path, which the page
/// then attaches to whatever you type. Raw bytes rather than base64 or
/// multipart: the name and owner ride in the query, and a photo is large
/// enough that a third more of it is worth avoiding.
fn upload(
    request: &mut Request,
    params: &[(String, String)],
) -> Response<std::io::Cursor<Vec<u8>>> {
    let value = |key: &str| {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let agent = value("agent");
    if agent.is_empty() {
        return status(400, "which agent is this for?");
    }
    if request.body_length().unwrap_or(0) > MAX_UPLOAD {
        return status(413, "that file is too large to send this way");
    }

    let bytes = match read_limited(request.as_reader(), MAX_UPLOAD) {
        Ok(bytes) => bytes,
        Err((code, reason)) => return status(code, reason),
    };
    if bytes.is_empty() {
        return status(400, "empty file");
    }

    match store_upload(&agent, &value("name"), &bytes) {
        Ok(path) => {
            crate::logger::info(format!("phone sent {} a file: {}", agent, path.display()));
            json(serde_json::json!({ "path": path.to_string_lossy() }).to_string())
        }
        Err(err) => {
            crate::logger::warn(format!("could not store an upload: {err}"));
            status(500, "could not store the file")
        }
    }
}

/// Write it beside the other cross-process state, under the agent it is for.
///
/// The name is rebuilt rather than trusted: it arrives from a phone and ends
/// up as a path an agent is told to open, so only the stem and a known
/// extension survive.
fn store_upload(agent: &str, name: &str, bytes: &[u8]) -> Result<std::path::PathBuf> {
    let dir = crate::comms::comms_root()?
        .join("uploads")
        .join(safe_part(agent));
    std::fs::create_dir_all(&dir)?;

    let name = std::path::Path::new(name);
    let stem = name
        .file_stem()
        .map(|s| safe_part(&s.to_string_lossy()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "upload".to_string());
    let extension = name
        .extension()
        .map(|e| safe_part(&e.to_string_lossy()))
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| "bin".to_string());

    let stamp = uuid::Uuid::new_v4();
    let path = dir.join(format!("{stamp}-{stem}.{extension}"));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    if let Err(err) = std::io::Write::write_all(&mut file, bytes) {
        let _ = std::fs::remove_file(&path);
        return Err(err.into());
    }
    Ok(path)
}

/// Letters, digits, dot, dash and underscore. Everything else — separators,
/// `..`, spaces, anything exotic — becomes a dash.
fn safe_part(raw: &str) -> String {
    raw.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => c,
            _ => '-',
        })
        .take(60)
        .collect::<String>()
        .replace("..", "-")
}

fn command_from(
    request: &mut Request,
    commands: &mpsc::UnboundedSender<CommandRequest>,
    build: impl FnOnce(String, String) -> Option<RemoteCommand>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let bytes = match read_limited(request.as_reader(), 256 * 1024) {
        Ok(bytes) => bytes,
        Err((code, reason)) => return status(code, reason),
    };
    let Ok(body) = String::from_utf8(bytes) else {
        return status(400, "unreadable body");
    };
    let value = |name: &'static str| {
        request
            .headers()
            .iter()
            .find(|h| h.field.equiv(name))
            .map(|h| h.value.as_str().to_string())
    };
    let id = value("X-Command-Id").unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if uuid::Uuid::parse_str(&id).is_err() {
        return status(400, "invalid command ID");
    }
    let issued = match value("X-Command-Time") {
        Some(value) => match value.parse::<i64>() {
            Ok(value) => value,
            Err(_) => return status(400, "invalid command time"),
        },
        None => chrono::Utc::now().timestamp(),
    };
    let Some((agent, text)) = parse_command_body(&body) else {
        return status(400, "expected {\"agent\": \"…\"}");
    };
    match build(agent, text) {
        Some(command) => {
            let fingerprint = etag(&format!("{command:?}"));
            let (reply, receipt) = std::sync::mpsc::sync_channel(1);
            if commands
                .send(CommandRequest {
                    id,
                    issued,
                    fingerprint,
                    command,
                    reply,
                })
                .is_err()
            {
                return status(503, "workbench is shutting down");
            }
            match receipt.recv_timeout(Duration::from_secs(8)) {
                Ok(Ok(())) => json("{\"ok\":true,\"state\":\"accepted\"}".to_string()),
                Ok(Err(reason)) => status(409, &reason),
                Err(_) => status(
                    504,
                    "Delivery is unconfirmed. Retry to check the same request.",
                ),
            }
        }
        None => status(400, "nothing to do"),
    }
}

/// Detect oversize chunked bodies as well as declared lengths. A socket idle
/// timeout complements this total deadline, including clients that trickle bytes.
fn read_limited(
    reader: &mut dyn std::io::Read,
    limit: usize,
) -> Result<Vec<u8>, (u16, &'static str)> {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        if Instant::now() >= deadline {
            return Err((408, "request timed out"));
        }
        let size = chunk.len().min(limit + 1 - bytes.len());
        let read = reader
            .read(&mut chunk[..size])
            .map_err(|_| (408, "could not finish reading the request"))?;
        if read == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() > limit {
            return Err((413, "that request is too large"));
        }
    }
}

/// `{"agent": "ab12cd34", "text": "…"}` → the pair, or nothing if malformed.
fn parse_command_body(body: &str) -> Option<(String, String)> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let agent = value.get("agent")?.as_str()?.trim().to_string();
    if agent.is_empty() || agent.len() > 128 {
        return None;
    }
    let text = value
        .get("text")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    Some((agent, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_uploads_do_not_block_state_reads_on_another_connection() {
        use std::io::{Read, Write};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let shared: Shared = Default::default();
        shared.lock().unwrap().agents.push(super::super::AgentView {
            id: "a".into(),
            ..Default::default()
        });
        let (tx, _rx) = mpsc::unbounded_channel();
        start_workers(listener, "test", "", &shared, &tx);
        let mut upload = std::net::TcpStream::connect(addr).unwrap();
        upload.write_all(b"POST /api/upload?t=test&agent=a&name=slow.jpg HTTP/1.1\r\nHost: localhost\r\nContent-Length: 65536\r\n\r\nx").unwrap();
        let mut read = std::net::TcpStream::connect(addr).unwrap();
        read.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        read.write_all(b"GET /api/state?t=test&agent=a HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut reply = String::new();
        read.read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
        let _ = upload.shutdown(std::net::Shutdown::Both);
    }

    #[test]
    fn upload_names_are_unique_even_in_the_same_second() {
        let a = store_upload("audit-unique", "photo.jpg", b"one").unwrap();
        let b = store_upload("audit-unique", "photo.jpg", b"two").unwrap();
        assert_ne!(a, b);
        assert_eq!(std::fs::read(&a).unwrap(), b"one");
        assert_eq!(std::fs::read(&b).unwrap(), b"two");
        std::fs::remove_file(a).unwrap();
        std::fs::remove_file(b).unwrap();
    }

    #[test]
    fn oversize_bodies_are_rejected_instead_of_saved_as_prefixes() {
        assert_eq!(read_limited(&mut &b"1234"[..], 4).unwrap(), b"1234");
        assert_eq!(read_limited(&mut &b"12345"[..], 4).unwrap_err().0, 413);
    }

    fn read_state(
        shared: &Shared,
        agent: &str,
        tag: Option<&str>,
    ) -> Response<std::io::Cursor<Vec<u8>>> {
        let mut request = tiny_http::TestRequest::new();
        if let Some(tag) = tag {
            request = request.with_header(header("If-None-Match", tag));
        }
        state_body(&request.into(), &[("agent".into(), agent.into())], shared)
    }

    #[test]
    fn reads_select_independent_conversations_and_ignore_the_heartbeat_for_caching() {
        let agents = ["a", "b"]
            .into_iter()
            .map(|id| super::super::AgentView {
                id: id.into(),
                tail: vec![format!("output {id}")],
                ..Default::default()
            })
            .collect();
        let shared: Shared = Arc::new(std::sync::Mutex::new(super::super::Snapshot {
            agents,
            ..Default::default()
        }));
        let a = read_state(&shared, "a", None);
        let tag = a
            .headers()
            .iter()
            .find(|h| h.field.equiv("ETag"))
            .unwrap()
            .value
            .as_str()
            .to_string();
        let body: serde_json::Value = serde_json::from_slice(a.into_reader().get_ref()).unwrap();
        assert_eq!(body["agents"][0]["tail"][0], "output a");
        assert_eq!(body["agents"][1]["tail"], serde_json::json!([]));
        let b = read_state(&shared, "b", None);
        let body: serde_json::Value = serde_json::from_slice(b.into_reader().get_ref()).unwrap();
        assert_eq!(body["agents"][1]["tail"][0], "output b");
        assert_eq!(body["agents"][0]["tail"], serde_json::json!([]));
        shared.lock().unwrap().at += 1;
        assert_eq!(read_state(&shared, "a", Some(&tag)).status_code().0, 304);
        let watching = &shared.lock().unwrap().watching;
        assert!(watching.contains_key("a") && watching.contains_key("b"));
    }

    #[test]
    fn command_responses_wait_for_the_app_and_return_rejections() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let worker = std::thread::spawn(move || {
            let mut request = tiny_http::TestRequest::new()
                .with_body(r#"{"agent":"gone","text":"hello"}"#)
                .into();
            command_from(&mut request, &tx, |agent, text| {
                Some(RemoteCommand::Reply { agent, text })
            })
        });
        let request = rx.blocking_recv().unwrap();
        assert!(!worker.is_finished(), "an enqueue is not an acknowledgment");
        request
            .reply
            .send(Err("This agent no longer exists".into()))
            .unwrap();
        let response = worker.join().unwrap();
        assert_eq!(response.status_code().0, 409);
        assert_eq!(
            response.into_reader().get_ref(),
            b"This agent no longer exists"
        );
    }

    #[test]
    fn a_command_body_needs_an_agent() {
        assert_eq!(
            parse_command_body(r#"{"agent":"ab12cd34","text":"do the thing"}"#),
            Some(("ab12cd34".to_string(), "do the thing".to_string()))
        );
        // Approve/deny carry no text.
        assert_eq!(
            parse_command_body(r#"{"agent":"ab12cd34"}"#),
            Some(("ab12cd34".to_string(), String::new()))
        );
        assert_eq!(parse_command_body(r#"{"text":"orphan"}"#), None);
        assert_eq!(parse_command_body(r#"{"agent":"  "}"#), None);
        assert_eq!(parse_command_body("not json"), None);
    }

    #[test]
    fn the_token_survives_the_trip_through_a_bookmark() {
        let params = query_params("t=abc%2Ddef&x=1");
        assert_eq!(params[0], ("t".to_string(), "abc-def".to_string()));
        assert_eq!(params[1], ("x".to_string(), "1".to_string()));
        assert!(query_params("").is_empty());
    }

    /// The name comes from a phone and ends up as a path an agent is told to
    /// open, so nothing of the original survives except a stem and a suffix.
    #[test]
    fn an_uploaded_name_cannot_escape_its_directory() {
        // An ordinary name survives intact; that is the whole point.
        assert_eq!(safe_part("IMG_0348.PNG"), "IMG_0348.PNG");
        assert_eq!(safe_part("a name with spaces"), "a-name-with-spaces");

        // Anything that could steer a path does not. Asserted as properties
        // rather than exact output — the guarantee is "no separator and no
        // parent", not one particular arrangement of dashes.
        for hostile in [
            "../../etc/passwd",
            "..",
            "/etc/passwd",
            "x/../..",
            "\\\\server\\share",
        ] {
            let safe = safe_part(hostile);
            assert!(!safe.contains('/'), "{hostile} -> {safe}");
            assert!(!safe.contains('\\'), "{hostile} -> {safe}");
            assert!(!safe.contains(".."), "{hostile} -> {safe}");
        }
        // Long enough to be a nuisance is cut, not rejected.
        assert!(safe_part(&"x".repeat(200)).len() <= 60);
    }

    #[test]
    fn an_upload_is_written_under_the_agent_it_is_for() {
        let name = "../../../sneaky photo.PNG";
        let path = store_upload("ab12cd34", name, b"not really a png").unwrap();

        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(file.ends_with(".PNG"), "the suffix is kept: {file}");
        assert!(!file.contains('/') && !file.contains(".."), "{file}");
        assert_eq!(
            path.parent().unwrap().file_name().unwrap(),
            "ab12cd34",
            "filed under the agent"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"not really a png");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tokens_are_long_and_unguessable_enough_to_sit_in_a_url() {
        let a = new_token();
        let b = new_token();
        assert_eq!(a.len(), 24);
        assert_ne!(a, b);
        // No look-alike characters: this gets read off a screen sometimes.
        assert!(!a.contains('l') && !a.contains('1') && !a.contains('0'));
    }
}

#[cfg(test)]
pub(super) fn visual_fixture(shared: Shared) -> String {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, _rx) = mpsc::unbounded_channel();
    start_workers(listener, "fixture", "", &shared, &tx);
    format!("http://127.0.0.1:{port}/?t=fixture")
}
