//! Files between Macs.
//!
//! A Mac only ever sends. `workbench send` — or `f` in the Machines window —
//! pushes files to another Mac's Workbench, which writes them under
//! `~/Workbench Inbox/<sender>/<when>/`. There is no route that reads a file
//! off a Mac: getting something from the other one means asking an agent
//! there to send it (`workbench ask mbp6~ab12 "send me the ship textures"`),
//! so each Mac keeps the say over what leaves it, and the token never
//! becomes a key to every file on the disk.
//!
//! Each file is one request with the raw bytes as its body, streamed both
//! ways, so a model of several gigabytes never sits in memory.

use super::peers;
use super::{AgentView, Role};
use crate::config::user_config::{load_user_config, MachineConfig};
use anyhow::{anyhow, bail, Context, Result};
use std::io::Read;
use std::net::{IpAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const RECEIVE_PATH: &str = "/api/receive";
/// One file. A large scene with its textures baked in fits; a disk image
/// sent by mistake does not fill the other Mac's disk.
pub const MAX_FILE: u64 = 16 << 30;
/// How long one file may take to arrive.
pub const RECEIVE_TIME: Duration = Duration::from_secs(3 * 3600);
/// Folders nested deeper than this are not walked (and a symlink loop ends).
const MAX_DEPTH: usize = 32;

/// `~/Workbench Inbox`, or `$WORKBENCH_INBOX` when that is set.
pub fn inbox() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("WORKBENCH_INBOX").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    Ok(dirs::home_dir()
        .context("no home folder")?
        .join("Workbench Inbox"))
}

// ---------------------------------------------------------------------------
// Receiving
// ---------------------------------------------------------------------------

/// Who sent something, as this Mac knows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sender {
    pub name: String,
    /// This Mac has a `[[machines]]` entry for it, so it can send back.
    pub configured: bool,
}

/// Name the Mac a request came from: the `[[machines]]` entry whose address
/// is the one that connected, else the host name it gave for itself.
pub fn sender(peer: Option<IpAddr>, claimed: &str) -> Sender {
    let configured = peer.and_then(|ip| {
        let peers = peers::get()?;
        peers
            .machines()
            .into_iter()
            .find(|m| resolves_to(&m.config.url, ip))
            .map(|m| m.config.name)
    });
    match configured {
        Some(name) => Sender {
            name,
            configured: true,
        },
        None => Sender {
            name: clean_part(claimed).unwrap_or_else(|| "unknown-mac".into()),
            configured: false,
        },
    }
}

fn resolves_to(url: &str, ip: IpAddr) -> bool {
    let Ok(url) = url::Url::parse(url) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    (host, url.port().unwrap_or(80))
        .to_socket_addrs()
        .is_ok_and(|mut addrs| addrs.any(|addr| addr.ip() == ip))
}

/// A message from another Mac, as the agent reads it: who sent it, and how
/// to answer with files.
pub fn framed(sender: &Sender, text: &str) -> String {
    let back = if sender.configured {
        format!(
            "send files back with `workbench send {} <files>`",
            sender.name
        )
    } else {
        format!(
            "this Mac has no [[machines]] entry for {}, so files cannot go back yet",
            sender.name
        )
    };
    format!("[From {} through Workbench; {back}] {text}", sender.name)
}

/// Where a received file landed.
#[derive(Debug)]
pub struct Received {
    pub path: PathBuf,
    pub folder: PathBuf,
}

/// Write one file of a batch under `root/<from>/<batch>/`. `name` is its
/// path inside the batch with `/` between the parts. Every part is rebuilt
/// rather than trusted, and a file never replaces one already there.
pub fn receive(
    root: &Path,
    from: &str,
    batch: &str,
    name: &str,
    body: &mut dyn Read,
) -> Result<Received, (u16, String)> {
    let bad = |why: &str| (400, why.to_string());
    let from = clean_part(from).ok_or_else(|| bad("which Mac is this from?"))?;
    let batch = clean_part(batch).ok_or_else(|| bad("which batch is this in?"))?;
    let parts: Vec<&str> = name.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    if parts.is_empty() || parts.len() > MAX_DEPTH {
        return Err(bad("the file has no usable name"));
    }
    let mut cleaned = Vec::new();
    for part in parts {
        if part == "." || part == ".." {
            return Err(bad("a file name may not climb out of its folder"));
        }
        cleaned.push(clean_part(part).ok_or_else(|| bad("the file has no usable name"))?);
    }

    let folder = root.join(&from).join(&batch);
    let mut target = folder.clone();
    for part in &cleaned {
        target.push(part);
    }
    let io = |err: std::io::Error| (500, format!("could not store the file: {err}"));
    let parent = target.parent().unwrap_or(&folder).to_path_buf();
    std::fs::create_dir_all(&parent).map_err(io)?;
    let file_name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let partial = parent.join(format!(".{file_name}.{}.part", uuid::Uuid::new_v4()));

    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
            .map_err(io)?;
        let copied = std::io::copy(&mut body.take(MAX_FILE + 1), &mut file)
            .map_err(|err| (400, format!("the transfer broke off: {err}")))?;
        if copied > MAX_FILE {
            return Err((413, "that file is larger than 16 GB".to_string()));
        }
        // Linking fails rather than replaces when the name is taken, so two
        // files of the same name both survive.
        for n in 1..1000 {
            let candidate = if n == 1 {
                target.clone()
            } else {
                numbered(&target, n)
            };
            match std::fs::hard_link(&partial, &candidate) {
                Ok(()) => return Ok(candidate),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(io(err)),
            }
        }
        Err((409, "too many files of that name".to_string()))
    })();
    let _ = std::fs::remove_file(&partial);
    Ok(Received {
        path: written?,
        folder,
    })
}

/// `ship.glb` → `ship (2).glb`.
fn numbered(path: &Path, n: usize) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = match path.extension() {
        Some(ext) => format!("{stem} ({n}).{}", ext.to_string_lossy()),
        None => format!("{stem} ({n})"),
    };
    path.with_file_name(name)
}

/// One part of a path, safe to join under the inbox: no separators, no
/// control characters, no `..`, and not empty.
fn clean_part(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':') {
                '-'
            } else {
                c
            }
        })
        .take(200)
        .collect();
    let cleaned = cleaned.trim().to_string();
    (!cleaned.is_empty() && cleaned != "." && cleaned != "..").then_some(cleaned)
}

// ---------------------------------------------------------------------------
// Sending
// ---------------------------------------------------------------------------

/// `mbp6~ab12` → `("mbp6", "ab12")`, when `mbp6` is a configured machine.
pub fn split_target(target: &str) -> Option<(String, String)> {
    let (machine, agent) = target.split_once(peers::SEP)?;
    let known = load_user_config()
        .machines
        .iter()
        .any(|m| m.name == machine);
    (known && !agent.is_empty()).then(|| (machine.to_string(), agent.to_string()))
}

/// This Mac's name, for a Mac that has no `[[machines]]` entry for it.
fn host_name() -> String {
    std::process::Command::new("scutil")
        .args(["--get", "LocalHostName"])
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "mac".into())
}

/// Another Mac's Workbench, read once to check it answers and to learn its
/// agents.
pub struct Remote {
    pub config: MachineConfig,
    instance: String,
    pub agents: Vec<AgentView>,
}

#[derive(Debug, Default)]
pub struct Sent {
    /// Where each file landed, on the other Mac.
    pub paths: Vec<String>,
    /// The batch folder they are all in, on the other Mac.
    pub folder: String,
    pub bytes: u64,
}

/// Progress of a send: the file, and bytes sent of the whole batch.
pub struct Progress<'a> {
    pub name: &'a str,
    pub sent: u64,
    pub total: u64,
}

pub fn connect(machine: &str) -> Result<Remote> {
    let config = load_user_config()
        .machines
        .into_iter()
        .find(|m| m.name == machine)
        .ok_or_else(|| {
            anyhow!("No machine named {machine} is configured. `workbench machines` lists them.")
        })?;
    Remote::at(config)
}

fn state(config: &MachineConfig, agent: Option<&str>) -> Result<peers::PeerSnapshot> {
    let mut path = format!("/api/state?local=1&t={}", peers::encode(&config.token));
    if let Some(agent) = agent {
        path.push_str(&format!("&agent={}", peers::encode(agent)));
    }
    let response = peers::request(&config.url, "GET", &path, &[], None)
        .with_context(|| format!("could not reach {}", config.name))?;
    match response.status {
        200 => serde_json::from_slice(&response.body)
            .with_context(|| format!("{} sent a state this Workbench cannot read", config.name)),
        401 => bail!(
            "{} refused the token; copy remote_token from that Mac",
            config.name
        ),
        code => bail!("{}: HTTP {code}: {}", config.name, response.text()),
    }
}

impl Remote {
    pub fn at(config: MachineConfig) -> Result<Remote> {
        let snapshot = state(&config, None)?;
        Ok(Remote {
            instance: snapshot.instance,
            agents: snapshot.agents,
            config,
        })
    }

    /// An agent there, by id, id prefix, alias, or a provider name only one
    /// agent there has.
    pub fn agent(&self, query: &str) -> Result<&AgentView> {
        let q = query.to_lowercase();
        let by = |test: &dyn Fn(&AgentView) -> bool| -> Vec<&AgentView> {
            self.agents.iter().filter(|a| test(a)).collect()
        };
        let tries: [Vec<&AgentView>; 4] = [
            by(&|a| a.id.to_lowercase() == q),
            by(&|a| a.alias.as_ref().is_some_and(|x| x.to_lowercase() == q)),
            by(&|a| a.id.to_lowercase().starts_with(&q)),
            by(&|a| a.provider.to_lowercase() == q),
        ];
        for found in tries {
            match found.as_slice() {
                [] => continue,
                [one] => return Ok(one),
                many => bail!(
                    "{query} could be any of these on {}: {}",
                    self.config.name,
                    many.iter()
                        .map(|a| describe(a))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        }
        bail!(
            "No agent {query} on {}. `workbench machines` lists its agents.",
            self.config.name
        )
    }

    /// The agent with its recent conversation.
    pub fn read_agent(&self, id: &str) -> Result<AgentView> {
        state(&self.config, Some(id))?
            .agents
            .into_iter()
            .find(|a| a.id == id)
            .ok_or_else(|| anyhow!("{id} is gone from {}", self.config.name))
    }

    /// Put text in front of an agent there, as if typed at that Mac. The
    /// other side says who it came from.
    pub fn tell(&self, agent: &str, text: &str) -> Result<()> {
        let body = serde_json::json!({ "agent": agent, "text": text }).to_string();
        let headers = [
            ("Content-Type".to_string(), "application/json".to_string()),
            ("X-Workbench-Instance".to_string(), self.instance.clone()),
        ];
        let path = format!(
            "/api/message?t={}&from={}",
            peers::encode(&self.config.token),
            peers::encode(&host_name())
        );
        let response = peers::request(
            &self.config.url,
            "POST",
            &path,
            &headers,
            Some(body.as_bytes()),
        )?;
        match response.status {
            200 => Ok(()),
            404 if response.text() == "not found" => bail!(
                "{} runs an older Workbench that cannot take messages; update it there",
                self.config.name
            ),
            code => bail!("{}: HTTP {code}: {}", self.config.name, response.text()),
        }
    }

    /// Send files and folders. A folder goes with everything in it, under
    /// its own name.
    pub fn send_files(
        &self,
        paths: &[PathBuf],
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Sent> {
        let files = collect(paths)?;
        // An older Workbench there would refuse only after part of a large
        // file went, and the error would be a broken pipe. An empty request
        // asks first: this route answers it with a 400, an old one with 404.
        let probe = peers::request(
            &self.config.url,
            "POST",
            &format!("{RECEIVE_PATH}?t={}", peers::encode(&self.config.token)),
            &[("X-Workbench-Instance".to_string(), self.instance.clone())],
            Some(b""),
        )
        .with_context(|| format!("could not reach {}", self.config.name))?;
        if probe.status == 404 {
            bail!(
                "{} runs an older Workbench that cannot take files; update it there",
                self.config.name
            );
        }
        let total: u64 = files.iter().map(|f| f.size).sum();
        let batch = chrono::Local::now().format("%Y-%m-%d %H.%M.%S").to_string();
        let from = host_name();
        let mut sent = Sent::default();
        for file in &files {
            let mut reader = std::fs::File::open(&file.path)
                .with_context(|| format!("could not open {}", file.path.display()))?;
            let path = format!(
                "{RECEIVE_PATH}?t={}&from={}&batch={}&name={}",
                peers::encode(&self.config.token),
                peers::encode(&from),
                peers::encode(&batch),
                peers::encode(&file.name)
            );
            let before = sent.bytes;
            let response = peers::request_stream(
                &self.config.url,
                &path,
                &[(
                    "Content-Type".to_string(),
                    "application/octet-stream".to_string(),
                )],
                &mut reader,
                file.size,
                &mut |done| {
                    progress(Progress {
                        name: &file.name,
                        sent: before + done,
                        total,
                    })
                },
            )
            .with_context(|| format!("could not send {}", file.name))?;
            match response.status {
                200 => {}
                404 if response.text() == "not found" => bail!(
                    "{} runs an older Workbench that cannot take files; update it there",
                    self.config.name
                ),
                code => bail!(
                    "{}: {}: HTTP {code}: {}",
                    self.config.name,
                    file.name,
                    response.text()
                ),
            }
            let landed: serde_json::Value = serde_json::from_slice(&response.body)
                .context("the other Mac did not say where the file went")?;
            sent.paths
                .push(landed["path"].as_str().unwrap_or_default().to_string());
            sent.folder = landed["folder"].as_str().unwrap_or_default().to_string();
            sent.bytes += file.size;
        }
        Ok(sent)
    }
}

/// What the receiving agent is told after a send with `--to`.
pub fn notice(sent: &Sent, note: Option<&str>) -> String {
    let mut text = match sent.paths.as_slice() {
        [one] => format!("I sent you a file: {one}"),
        many => {
            let names: Vec<&str> = many
                .iter()
                .map(|p| {
                    p.strip_prefix(&sent.folder)
                        .unwrap_or(p)
                        .trim_start_matches('/')
                })
                .collect();
            let mut list = names
                .iter()
                .take(10)
                .copied()
                .collect::<Vec<_>>()
                .join(", ");
            if names.len() > 10 {
                list.push_str(&format!(", and {} more", names.len() - 10));
            }
            format!(
                "I sent you {} files, in {}: {list}",
                many.len(),
                sent.folder
            )
        }
    };
    if let Some(note) = note.map(str::trim).filter(|n| !n.is_empty()) {
        text.push_str(". ");
        text.push_str(note);
    }
    text
}

/// The text a remote agent ends its last turn with, once it is done
/// answering. `None` while it is still at work on what was asked.
pub fn answer_after(agent: &AgentView, before: usize) -> Option<String> {
    if agent.msg_total <= before || agent.status == "working" {
        return None;
    }
    let last_you = agent.messages.iter().rposition(|m| m.role == Role::You)?;
    let said: Vec<&str> = agent.messages[last_you + 1..]
        .iter()
        .filter(|m| m.role == Role::Agent)
        .map(|m| m.text.as_str())
        .collect();
    (!said.is_empty()).then(|| said.join("\n\n"))
}

pub fn describe(agent: &AgentView) -> String {
    match &agent.alias {
        Some(alias) => format!("{alias} ({}, {})", agent.id, agent.provider),
        None => format!("{} ({})", agent.id, agent.provider),
    }
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = "B";
    for next in UNITS {
        if value < 1000.0 {
            break;
        }
        value /= 1000.0;
        unit = next;
    }
    format!("{value:.1} {unit}")
}

struct Outgoing {
    path: PathBuf,
    /// Its path inside the batch, `/` between the parts.
    name: String,
    size: u64,
}

fn collect(paths: &[PathBuf]) -> Result<Vec<Outgoing>> {
    let mut out = Vec::new();
    for path in paths {
        let meta =
            std::fs::metadata(path).with_context(|| format!("cannot read {}", path.display()))?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .or_else(|| {
                std::fs::canonicalize(path)
                    .ok()?
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "files".into());
        if meta.is_dir() {
            walk(path, &name, 0, &mut out)?;
        } else {
            out.push(Outgoing {
                path: path.clone(),
                name,
                size: meta.len(),
            });
        }
    }
    if out.is_empty() {
        bail!("there is nothing to send");
    }
    if let Some(big) = out.iter().find(|f| f.size > MAX_FILE) {
        bail!(
            "{} is {}; one file can be 16 GB at most",
            big.name,
            human_bytes(big.size)
        );
    }
    Ok(out)
}

fn walk(dir: &Path, prefix: &str, depth: usize, out: &mut Vec<Outgoing>) -> Result<()> {
    if depth >= MAX_DEPTH {
        bail!("{} is nested too deep to send", dir.display());
    }
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("cannot read {}", dir.display()))?
        .flatten()
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".DS_Store" {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let inner = format!("{prefix}/{name}");
        if meta.is_dir() {
            walk(&path, &inner, depth + 1, out)?;
        } else if meta.is_file() {
            out.push(Outgoing {
                path,
                name: inner,
                size: meta.len(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn take(root: &Path, name: &str, body: &str) -> Result<Received, (u16, String)> {
        receive(
            root,
            "mbp6",
            "2026-10-23 14.00.00",
            name,
            &mut body.as_bytes(),
        )
    }

    #[test]
    fn a_received_file_lands_under_its_sender_and_batch_and_never_replaces_one() {
        let root = tempfile::tempdir().unwrap();
        let first = take(root.path(), "ship/textures/hull.png", "a").unwrap();
        let folder = root.path().join("mbp6").join("2026-10-23 14.00.00");
        assert_eq!(first.folder, folder);
        assert_eq!(first.path, folder.join("ship/textures/hull.png"));

        let second = take(root.path(), "ship/textures/hull.png", "b").unwrap();
        assert_eq!(second.path, folder.join("ship/textures/hull (2).png"));
        assert_eq!(std::fs::read_to_string(&first.path).unwrap(), "a");
        assert_eq!(std::fs::read_to_string(&second.path).unwrap(), "b");

        let left: Vec<_> = std::fs::read_dir(folder.join("ship/textures"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".part"))
            .collect();
        assert!(left.is_empty(), "no partial file stays behind");
    }

    #[test]
    fn a_name_cannot_climb_out_of_the_inbox() {
        let root = tempfile::tempdir().unwrap();
        for name in ["../escape", "a/../../b", "", "/", "./"] {
            assert_eq!(take(root.path(), name, "x").unwrap_err().0, 400, "{name:?}");
        }
        let odd = take(root.path(), "/abs/path:x.glb", "x").unwrap();
        assert!(odd.path.starts_with(root.path().join("mbp6")));
        assert_eq!(odd.path.file_name().unwrap(), "path-x.glb");
        assert!(receive(root.path(), "..", "b", "f", &mut &b"x"[..]).is_err());
    }

    #[test]
    fn a_folder_is_sent_with_everything_in_it_under_its_own_name() {
        let dir = tempfile::tempdir().unwrap();
        let ship = dir.path().join("ship");
        std::fs::create_dir_all(ship.join("tex")).unwrap();
        std::fs::write(ship.join("ship.glb"), "glb").unwrap();
        std::fs::write(ship.join("tex/hull.png"), "png!").unwrap();
        std::fs::write(ship.join(".DS_Store"), "").unwrap();
        let single = dir.path().join("note.txt");
        std::fs::write(&single, "hi").unwrap();

        let files = collect(&[ship, single]).unwrap();
        let names: Vec<_> = files.iter().map(|f| (f.name.as_str(), f.size)).collect();
        assert_eq!(
            names,
            vec![
                ("ship/ship.glb", 3),
                ("ship/tex/hull.png", 4),
                ("note.txt", 2)
            ]
        );
    }

    #[test]
    fn the_notice_names_one_file_or_the_folder_and_a_few_names() {
        let one = Sent {
            paths: vec!["/i/mbp/b/ship.glb".into()],
            folder: "/i/mbp/b".into(),
            bytes: 1,
        };
        assert_eq!(
            notice(&one, Some("the hull")),
            "I sent you a file: /i/mbp/b/ship.glb. the hull"
        );
        let two = Sent {
            paths: vec!["/i/mbp/b/s/a.glb".into(), "/i/mbp/b/s/t.png".into()],
            folder: "/i/mbp/b".into(),
            bytes: 2,
        };
        assert_eq!(
            notice(&two, None),
            "I sent you 2 files, in /i/mbp/b: s/a.glb, s/t.png"
        );
    }

    #[test]
    fn an_answer_is_the_agent_speech_after_the_question_once_it_stops() {
        let message = |role, text: &str| super::super::thread::Message {
            role,
            text: text.into(),
            at: None,
        };
        let mut agent = AgentView {
            status: "working".into(),
            msg_total: 5,
            messages: vec![
                message(Role::You, "send the ship"),
                message(Role::Tool, "workbench send"),
                message(Role::Agent, "Sent it."),
            ],
            ..Default::default()
        };
        assert_eq!(answer_after(&agent, 2), None, "still working");
        agent.status = "idle".into();
        assert_eq!(answer_after(&agent, 5), None, "nothing new yet");
        assert_eq!(answer_after(&agent, 2).as_deref(), Some("Sent it."));
    }

    #[test]
    fn sizes_read_in_decimal_units() {
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(240_000_000), "240.0 MB");
        assert_eq!(human_bytes(3_500_000_000), "3.5 GB");
    }
}
