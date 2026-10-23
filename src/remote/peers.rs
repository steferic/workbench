//! Other Macs running Workbench, reached through their phone views.
//!
//! Each Workbench already serves its whole state to a phone over the tailnet
//! (`/api/state`) and takes replies, answers and stops on the same port. So
//! another Workbench is just one more client of it: this module reads each
//! configured machine's snapshot every couple of seconds, and the phone view
//! and the Machines window show those agents beside the local ones.
//!
//! A remote agent is addressed here as `<machine>~<its short id>` — and a
//! remote project as `<machine>~<its id>` — so every existing endpoint can
//! carry one, and the server knows to pass the command on (see `forward`)
//! rather than look for it locally.
//!
//! Reading `/api/state?local=1` asks a Workbench for its own agents only, so
//! two Macs that list each other do not echo each other's agents back.

use super::screen::Frame;
use super::{AgentView, ProjectView, Snapshot};
use crate::config::user_config::MachineConfig;
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Separates the machine from the id it uses for itself.
pub const SEP: char = '~';

/// How often each machine's agent list is read.
const LIST_EVERY: Duration = Duration::from_secs(2);
/// A conversation someone is looking at is read this often.
const DETAIL_EVERY: Duration = Duration::from_secs(1);
/// A conversation nobody asked for in this long stops being read.
const FOCUS_LASTS: Duration = Duration::from_secs(20);
/// How often the terminal on show is read. Most reads are a 304.
const SCREEN_EVERY: Duration = Duration::from_millis(60);
/// A terminal nobody drew for this long stops being read.
const SCREEN_LASTS: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PeerSnapshot {
    pub instance: String,
    pub projects: Vec<ProjectView>,
    pub agents: Vec<AgentView>,
    pub at: i64,
}

#[derive(Debug, Clone)]
pub struct Machine {
    pub config: MachineConfig,
    pub snapshot: Option<PeerSnapshot>,
    /// Why the last read failed, until one succeeds.
    pub error: Option<String>,
    /// When a read last succeeded.
    pub seen: Option<Instant>,
}

#[derive(Default)]
pub struct Peers {
    machines: Mutex<Vec<Machine>>,
    /// (machine, agent) → when someone last asked to read it.
    focus: Mutex<HashMap<(String, String), Instant>>,
    /// (machine, agent) → that agent with its conversation.
    details: Mutex<HashMap<(String, String), AgentView>>,
    /// The one remote terminal on show here, and when it was last drawn.
    screen_wanted: Mutex<Option<((String, String), Instant)>>,
    /// Its latest frame, for that (machine, agent).
    screen: Mutex<Option<((String, String), Frame)>>,
    /// Keys for remote terminals, sent in order by one thread.
    keys: Mutex<Option<std::sync::mpsc::Sender<((String, String), Vec<u8>)>>>,
}

static PEERS: OnceLock<Peers> = OnceLock::new();

/// Goes up with every new remote frame, so the event loop knows to redraw
/// a terminal that changed on another Mac — no local event says so.
static SCREEN_VERSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn screen_version() -> u64 {
    SCREEN_VERSION.load(std::sync::atomic::Ordering::Relaxed)
}

/// Start reading the configured machines. Does nothing with none configured,
/// or the second time.
pub fn start(configs: &[MachineConfig]) {
    if configs.is_empty() || PEERS.get().is_some() {
        return;
    }
    let peers = Peers {
        machines: Mutex::new(
            configs
                .iter()
                .map(|config| Machine {
                    config: config.clone(),
                    snapshot: None,
                    error: None,
                    seen: None,
                })
                .collect(),
        ),
        ..Default::default()
    };
    if PEERS.set(peers).is_err() {
        return;
    }
    std::thread::spawn(poll);
    std::thread::spawn(poll_screen);
}

pub fn get() -> Option<&'static Peers> {
    PEERS.get()
}

impl Peers {
    pub fn machines(&self) -> Vec<Machine> {
        self.machines.lock().map(|m| m.clone()).unwrap_or_default()
    }

    /// Ask for an agent's conversation to be kept current, and return what
    /// is known of it so far.
    pub fn watch(&self, machine: &str, agent: &str) -> Option<AgentView> {
        let key = (machine.to_string(), agent.to_string());
        if let Ok(mut focus) = self.focus.lock() {
            focus.insert(key.clone(), Instant::now());
        }
        self.details.lock().ok()?.get(&key).cloned()
    }

    /// The latest frame of a remote agent's terminal, and a request to keep
    /// reading it. Call it while drawing that terminal.
    pub fn screen(&self, machine: &str, agent: &str) -> Option<Frame> {
        let key = (machine.to_string(), agent.to_string());
        if let Ok(mut wanted) = self.screen_wanted.lock() {
            *wanted = Some((key.clone(), Instant::now()));
        }
        let screen = self.screen.lock().ok()?;
        screen
            .as_ref()
            .filter(|(k, _)| *k == key)
            .map(|(_, frame)| frame.clone())
    }

    /// Type into a remote agent's terminal. Keys go in the order given:
    /// one thread sends them, joining whatever queued up while the last
    /// request was on its way.
    pub fn send_keys(&self, machine: &str, agent: &str, bytes: Vec<u8>) {
        let Ok(mut keys) = self.keys.lock() else {
            return;
        };
        let sender = keys.get_or_insert_with(|| {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || send_queued_keys(rx));
            tx
        });
        let _ = sender.send(((machine.to_string(), agent.to_string()), bytes));
    }

    fn config(&self, machine: &str) -> Option<(MachineConfig, String)> {
        let machines = self.machines.lock().ok()?;
        let found = machines.iter().find(|m| m.config.name == machine)?;
        let instance = found
            .snapshot
            .as_ref()
            .map(|s| s.instance.clone())
            .unwrap_or_default();
        Some((found.config.clone(), instance))
    }

    /// `mbp6~ab12cd34` → `("mbp6", "ab12cd34")`, for a machine that is
    /// configured. Anything else is a local id.
    pub fn split<'a>(&self, id: &'a str) -> Option<(&'a str, &'a str)> {
        let (machine, rest) = id.split_once(SEP)?;
        let known = self
            .machines
            .lock()
            .ok()?
            .iter()
            .any(|m| m.config.name == machine);
        (known && !rest.is_empty()).then_some((machine, rest))
    }
}

/// `zeta-studio @ mbp6`: a remote project's name wherever it is listed.
pub fn project_name(project: &str, machine: &str) -> String {
    format!("{project} @ {machine}")
}

fn poll() {
    let Some(peers) = get() else { return };
    let mut last_list: HashMap<String, Instant> = HashMap::new();
    let mut tags: HashMap<String, String> = HashMap::new();
    loop {
        let configs: Vec<MachineConfig> = peers.machines().into_iter().map(|m| m.config).collect();
        for config in &configs {
            if last_list
                .get(&config.name)
                .is_some_and(|at| at.elapsed() < LIST_EVERY)
            {
                continue;
            }
            last_list.insert(config.name.clone(), Instant::now());
            let tag = tags.get(&config.name).cloned();
            let result = fetch_state(config, None, tag.as_deref());
            let Ok(mut machines) = peers.machines.lock() else {
                return;
            };
            let Some(machine) = machines.iter_mut().find(|m| m.config.name == config.name) else {
                continue;
            };
            match result {
                Ok(Fetched::Same) => machine.seen = Some(Instant::now()),
                Ok(Fetched::New(mut snapshot, tag)) => {
                    // The list carries a conversation only for whatever that
                    // Mac's own phone has open; the detail reads below are
                    // what this side shows.
                    for agent in &mut snapshot.agents {
                        agent.messages.clear();
                        agent.tail.clear();
                    }
                    machine.snapshot = Some(snapshot);
                    machine.error = None;
                    machine.seen = Some(Instant::now());
                    if let Some(tag) = tag {
                        tags.insert(config.name.clone(), tag);
                    }
                }
                Err(err) => {
                    machine.error = Some(format!("{err:#}"));
                    tags.remove(&config.name);
                }
            }
        }

        let wanted: Vec<(String, String)> = peers
            .focus
            .lock()
            .map(|mut focus| {
                focus.retain(|_, at| at.elapsed() < FOCUS_LASTS);
                focus.keys().cloned().collect()
            })
            .unwrap_or_default();
        if let Ok(mut details) = peers.details.lock() {
            details.retain(|key, _| wanted.contains(key));
        }
        for (machine, agent) in wanted {
            let Some(config) = configs.iter().find(|c| c.name == machine) else {
                continue;
            };
            if let Ok(Fetched::New(snapshot, _)) = fetch_state(config, Some(&agent), None) {
                if let Some(found) = snapshot.agents.into_iter().find(|a| a.id == agent) {
                    if let Ok(mut details) = peers.details.lock() {
                        details.insert((machine, agent), found);
                    }
                }
            }
        }
        std::thread::sleep(DETAIL_EVERY);
    }
}

fn send_queued_keys(rx: std::sync::mpsc::Receiver<((String, String), Vec<u8>)>) {
    let mut held: Option<((String, String), Vec<u8>)> = None;
    loop {
        let (target, mut bytes) = match held.take() {
            Some(next) => next,
            None => match rx.recv() {
                Ok(next) => next,
                Err(_) => return,
            },
        };
        while let Ok((next, more)) = rx.try_recv() {
            if next == target {
                bytes.extend(more);
            } else {
                held = Some((next, more));
                break;
            }
        }
        let hex = super::screen::to_hex(&bytes);
        let (code, body) = forward(&target.0, "/api/keys", &target.1, &hex, &[]);
        if code != 200 {
            crate::logger::warn(format!(
                "keys for {}{SEP}{} were refused: {code} {body}",
                target.0, target.1
            ));
        }
    }
}

fn poll_screen() {
    let Some(peers) = get() else { return };
    let mut tag: Option<((String, String), String)> = None;
    loop {
        let wanted = peers
            .screen_wanted
            .lock()
            .ok()
            .and_then(|w| w.clone())
            .filter(|(_, at)| at.elapsed() < SCREEN_LASTS)
            .map(|(key, _)| key);
        let Some(key) = wanted else {
            tag = None;
            std::thread::sleep(Duration::from_millis(200));
            continue;
        };
        let Some((config, _)) = peers.config(&key.0) else {
            std::thread::sleep(Duration::from_millis(500));
            continue;
        };
        let known = tag
            .as_ref()
            .filter(|(k, _)| *k == key)
            .map(|(_, t)| t.clone());
        match fetch_screen(&config, &key.1, known.as_deref()) {
            Ok(Some(frame)) => {
                tag = Some((key.clone(), frame.tag.clone()));
                if let Ok(mut screen) = peers.screen.lock() {
                    *screen = Some((key, frame));
                }
                SCREEN_VERSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                std::thread::sleep(SCREEN_EVERY);
            }
            Ok(None) => std::thread::sleep(SCREEN_EVERY),
            Err(_) => std::thread::sleep(Duration::from_millis(500)),
        }
    }
}

/// `Some` for a new frame; `None` when it is unchanged or not built yet.
fn fetch_screen(config: &MachineConfig, agent: &str, tag: Option<&str>) -> Result<Option<Frame>> {
    let path = format!(
        "/api/screen?agent={}&t={}",
        encode(agent),
        encode(&config.token)
    );
    let mut headers = Vec::new();
    if let Some(tag) = tag {
        headers.push(("If-None-Match".to_string(), tag.to_string()));
    }
    let response = request(&config.url, "GET", &path, &headers, None)?;
    match response.status {
        200 => {
            let size = response.header("x-screen-size").unwrap_or_default();
            let (rows, cols) = size
                .split_once('x')
                .and_then(|(r, c)| Some((r.parse().ok()?, c.parse().ok()?)))
                .context("the screen came without its size")?;
            Ok(Some(Frame {
                rows,
                cols,
                tag: response.header("etag").unwrap_or_default(),
                bytes: std::sync::Arc::new(response.body),
            }))
        }
        202 | 304 => Ok(None),
        code => bail!("HTTP {code}: {}", response.text()),
    }
}

enum Fetched {
    Same,
    New(PeerSnapshot, Option<String>),
}

fn fetch_state(config: &MachineConfig, agent: Option<&str>, tag: Option<&str>) -> Result<Fetched> {
    let mut path = format!("/api/state?local=1&t={}", encode(&config.token));
    if let Some(agent) = agent {
        path.push_str(&format!("&agent={}", encode(agent)));
    }
    let mut headers = Vec::new();
    if let Some(tag) = tag {
        headers.push(("If-None-Match".to_string(), tag.to_string()));
    }
    let response = request(&config.url, "GET", &path, &headers, None)?;
    match response.status {
        304 => Ok(Fetched::Same),
        200 => {
            let snapshot = serde_json::from_slice(&response.body)
                .context("that Workbench sent a state this one cannot read")?;
            Ok(Fetched::New(snapshot, response.header("etag")))
        }
        401 => bail!("the token was refused; copy remote_token from that Mac"),
        code => bail!("HTTP {code}: {}", response.text()),
    }
}

/// Pass a phone command for a remote agent or project on to its machine.
/// `path` is the endpoint (`/api/reply`, …), `id` the id on that machine.
/// Returns the status and body to hand back to whoever asked.
pub fn forward(
    machine: &str,
    path: &str,
    id: &str,
    text: &str,
    passthrough: &[(String, String)],
) -> (u16, String) {
    let Some((config, instance)) = get().and_then(|p| p.config(machine)) else {
        return (404, format!("No machine named {machine} is configured."));
    };
    if instance.is_empty() {
        return (503, format!("{machine} has not answered yet."));
    }
    let body = serde_json::json!({ "agent": id, "text": text }).to_string();
    let mut headers = vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        ("X-Workbench-Instance".to_string(), instance),
    ];
    headers.extend(passthrough.iter().cloned());
    let path = format!("{path}?t={}", encode(&config.token));
    match request(&config.url, "POST", &path, &headers, Some(body.as_bytes())) {
        Ok(response) => (response.status, response.text()),
        Err(err) => (502, format!("Could not reach {machine}: {err:#}")),
    }
}

/// Add every machine's projects and agents to a snapshot about to go to the
/// phone. `wanted` is the agent the phone has open; a remote one gets its
/// conversation from the detail reads.
pub fn merge(snapshot: &mut Snapshot, wanted: Option<&str>) {
    let Some(peers) = get() else { return };
    let wanted = wanted.and_then(|id| peers.split(id));
    for machine in peers.machines() {
        let name = &machine.config.name;
        let Some(remote) = machine.snapshot else {
            continue;
        };
        for project in remote.projects {
            snapshot.projects.push(ProjectView {
                id: format!("{name}{SEP}{}", project.id),
                name: project_name(&project.name, name),
                // Deciding them goes through ids this side does not have.
                objectives: Vec::new(),
                proposals: Vec::new(),
                ..project
            });
        }
        for mut agent in remote.agents {
            let short = agent.id.clone();
            if wanted == Some((name.as_str(), short.as_str())) {
                if let Some(detail) = peers.watch(name, &short) {
                    agent = detail;
                }
            }
            agent.id = format!("{name}{SEP}{short}");
            agent.project = project_name(&agent.project, name);
            agent.project_id = format!("{name}{SEP}{}", agent.project_id);
            snapshot.agents.push(agent);
        }
    }
}

// ---------------------------------------------------------------------------
// A small HTTP/1.1 client: enough for one Workbench to talk to another.
// ---------------------------------------------------------------------------

pub struct Response {
    pub status: u16,
    headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    fn header(&self, name: &str) -> Option<String> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).trim().to_string()
    }
}

pub fn request(
    base: &str,
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
) -> Result<Response> {
    let body = body.unwrap_or_default();
    let length = (method != "GET").then_some(body.len() as u64);
    let mut stream = open(base, method, path, headers, length, Duration::from_secs(15))?;
    stream.write_all(body)?;
    read_response(stream)
}

/// A POST whose body is copied from `body` as it is sent, so a file of
/// several gigabytes never sits in memory. `sent` hears the running total.
pub fn request_stream(
    base: &str,
    path: &str,
    headers: &[(String, String)],
    body: &mut dyn Read,
    length: u64,
    sent: &mut dyn FnMut(u64),
) -> Result<Response> {
    let mut stream = open(
        base,
        "POST",
        path,
        headers,
        Some(length),
        Duration::from_secs(60),
    )?;
    let mut buffer = vec![0u8; 256 * 1024];
    let mut total = 0u64;
    loop {
        let read = body.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        stream.write_all(&buffer[..read])?;
        total += read as u64;
        sent(total);
    }
    if total != length {
        bail!("the file changed size while it was sent");
    }
    read_response(stream)
}

/// Connect and write the request head.
fn open(
    base: &str,
    method: &str,
    path: &str,
    headers: &[(String, String)],
    length: Option<u64>,
    timeout: Duration,
) -> Result<TcpStream> {
    let url = url::Url::parse(base).with_context(|| format!("bad url {base}"))?;
    if url.scheme() != "http" {
        bail!("only http:// addresses are supported, got {base}");
    }
    let host = url.host_str().context("the url has no host")?;
    let port = url.port().unwrap_or(80);
    let addr = (host, port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow!("{host} does not resolve"))?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;

    let mut head =
        format!("{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if let Some(length) = length {
        head.push_str(&format!("Content-Length: {length}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    Ok(stream)
}

fn read_response(stream: TcpStream) -> Result<Response> {
    let mut raw = Vec::new();
    stream.take(64 << 20).read_to_end(&mut raw)?;
    parse_response(&raw)
}

fn parse_response(raw: &[u8]) -> Result<Response> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .context("the response ended before its headers did")?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .context("no status line")?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let mut body = raw[split + 4..].to_vec();
    let chunked = headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("transfer-encoding") && v.eq_ignore_ascii_case("chunked")
    });
    if chunked {
        body = dechunk(&body)?;
    }
    Ok(Response {
        status,
        headers,
        body,
    })
}

fn dechunk(mut raw: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let end = raw
            .windows(2)
            .position(|w| w == b"\r\n")
            .context("a chunk size has no end")?;
        let size_text = String::from_utf8_lossy(&raw[..end]);
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)?;
        raw = &raw[end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if raw.len() < size {
            bail!("a chunk ended early");
        }
        out.extend_from_slice(&raw[..size]);
        raw = raw.get(size + 2..).unwrap_or_default();
    }
}

pub fn encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_response_reads_with_or_without_chunks() {
        let plain = b"HTTP/1.1 200 OK\r\nETag: \"ab\"\r\nContent-Length: 2\r\n\r\nhi";
        let response = parse_response(plain).unwrap();
        assert_eq!((response.status, response.text()), (200, "hi".to_string()));
        assert_eq!(response.header("etag").as_deref(), Some("\"ab\""));

        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n";
        assert_eq!(parse_response(chunked).unwrap().text(), "abcde");
    }

    #[test]
    fn a_remote_agent_is_listed_under_its_machine() {
        let peers = Peers {
            machines: Mutex::new(vec![Machine {
                config: MachineConfig {
                    name: "mbp6".into(),
                    url: "http://127.0.0.1:1".into(),
                    token: "t".into(),
                },
                snapshot: None,
                error: None,
                seen: None,
            }]),
            ..Default::default()
        };
        assert_eq!(peers.split("mbp6~ab12"), Some(("mbp6", "ab12")));
        assert_eq!(
            peers.split("other~ab12"),
            None,
            "an unknown machine is not remote"
        );
        assert_eq!(peers.split("ab12"), None);
        assert_eq!(peers.split("mbp6~"), None);
    }
}
