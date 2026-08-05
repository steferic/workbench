//! Dev servers, noticed and made reachable from the phone.
//!
//! A project's dev server is the thing you actually want to look at from the
//! sofa, and it is the one thing the phone view could not show you. Not
//! because of Tailscale — the tailnet address is a real interface address, and
//! workbench's own page is served from it — but because dev servers bind
//! loopback. `http://100.x.x.x:5173` reaches nothing when vite is listening on
//! `[::1]:5173`.
//!
//! So: notice what is listening, work out whose it is, and splice the tailnet
//! address to loopback on the *same port number*. The URL is then the one you
//! already know with a different host, HMR websockets work because nothing is
//! interpreting the bytes, and there is no per-project configuration.
//!
//! ```text
//! phone ──▶ 100.86.134.123:5173 ──splice──▶ 127.0.0.1:5173 (vite)
//! ```
//!
//! Which ports are eligible is decided by whose working directory the process
//! is in. That is not a heuristic to keep the list tidy — it is the boundary.
//! A scan of a working machine turns up postgres, mysql, redis and ollama
//! alongside the dev servers, and forwarding a database to the tailnet is a
//! different proposition from forwarding vite. A process running inside one of
//! your projects is the thing you started to work on it.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command;

pub mod forward;

/// Ports at or above this are handed out by the OS, not chosen by a dev
/// server — phoenix's distribution port, ollama's helper. Nobody types one
/// into a browser, so they are not what this is for.
const EPHEMERAL_FROM: u16 = 49152;

/// Something listening, and whose it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevServer {
    pub port: u16,
    /// The program, as the OS names it ("node", "beam.smp").
    pub command: String,
    /// Where it is running, which is what attributes it to a project.
    pub cwd: PathBuf,
    /// Bound to loopback only, so it needs a forwarder to be reachable. A
    /// server already on `0.0.0.0` is reachable over the tailnet as it is.
    pub loopback_only: bool,
}

/// Everything listening on this machine, attributed by working directory.
///
/// Two `lsof` calls: one for the listeners, one for those processes' working
/// directories. Blocking and a fork apiece, so it belongs off the event loop.
pub fn scan() -> Vec<DevServer> {
    let listeners = run(&["-nP", "-iTCP", "-sTCP:LISTEN", "-F", "pcn"])
        .map(|out| parse_listeners(&out))
        .unwrap_or_default();
    if listeners.is_empty() {
        return Vec::new();
    }

    let pids: Vec<String> = {
        let mut seen: Vec<u32> = listeners.iter().map(|l| l.pid).collect();
        seen.sort_unstable();
        seen.dedup();
        seen.iter().map(u32::to_string).collect()
    };
    let cwds = run(&["-a", "-p", &pids.join(","), "-d", "cwd", "-F", "pn"])
        .map(|out| parse_cwds(&out))
        .unwrap_or_default();

    let mut servers: Vec<DevServer> = Vec::new();
    for listener in listeners {
        if listener.port >= EPHEMERAL_FROM {
            continue;
        }
        let Some(cwd) = cwds.get(&listener.pid) else {
            continue;
        };
        // One process can listen on the same port over v4 and v6; the port is
        // what we forward, so it is the identity.
        if let Some(existing) = servers.iter_mut().find(|s| s.port == listener.port) {
            existing.loopback_only &= listener.loopback;
            continue;
        }
        servers.push(DevServer {
            port: listener.port,
            command: listener.command,
            cwd: cwd.clone(),
            loopback_only: listener.loopback,
        });
    }
    servers.sort_by_key(|s| s.port);
    servers
}

fn run(args: &[&str]) -> Option<String> {
    let output = Command::new("lsof").args(args).output().ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The ones running inside `roots`, newest project first in the caller's order.
///
/// `roots` maps a directory to the project it belongs to — workspace paths,
/// and the worktrees of sessions inside them.
pub fn owned_by<'a>(
    servers: &'a [DevServer],
    roots: &'a [(PathBuf, uuid::Uuid)],
) -> Vec<(&'a DevServer, uuid::Uuid)> {
    servers
        .iter()
        .filter_map(|server| {
            // The longest matching root wins: a worktree lives inside its
            // workspace on some setups, and the worktree is the better answer.
            roots
                .iter()
                .filter(|(root, _)| server.cwd.starts_with(root))
                .max_by_key(|(root, _)| root.as_os_str().len())
                .map(|(_, project)| (server, *project))
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
struct Listener {
    pid: u32,
    command: String,
    port: u16,
    loopback: bool,
}

/// `lsof -F pcn` emits a process line, a command line, then a line per open
/// file. Fields carry a one-letter tag, so state carries forward until the
/// next process.
fn parse_listeners(output: &str) -> Vec<Listener> {
    let mut listeners = Vec::new();
    let (mut pid, mut command) = (0u32, String::new());

    for line in output.lines() {
        let (tag, value) = match line.split_at_checked(1) {
            Some(pair) => pair,
            None => continue,
        };
        match tag {
            "p" => {
                pid = value.parse().unwrap_or(0);
                command.clear();
            }
            "c" => command = value.to_string(),
            "n" => {
                if let Some((address, port)) = split_address(value) {
                    listeners.push(Listener {
                        pid,
                        command: command.clone(),
                        port,
                        loopback: is_loopback(address),
                    });
                }
            }
            _ => {}
        }
    }
    listeners
}

/// `127.0.0.1:6379` / `[::1]:5173` / `*:7000` → the address and the port.
fn split_address(name: &str) -> Option<(&str, u16)> {
    // Rsplit: an IPv6 address is full of colons.
    let (address, port) = name.rsplit_once(':')?;
    // "->" means an established connection, not a listener.
    if name.contains("->") {
        return None;
    }
    Some((address, port.parse().ok()?))
}

fn is_loopback(address: &str) -> bool {
    let trimmed = address.trim_start_matches('[').trim_end_matches(']');
    match trimmed.parse::<IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        // `*` — every interface, so already reachable.
        Err(_) => false,
    }
}

fn parse_cwds(output: &str) -> HashMap<u32, PathBuf> {
    let mut cwds = HashMap::new();
    let mut pid = 0u32;
    for line in output.lines() {
        match line.split_at_checked(1) {
            Some(("p", value)) => pid = value.parse().unwrap_or(0),
            Some(("n", value)) if pid != 0 => {
                cwds.insert(pid, PathBuf::from(value));
            }
            _ => {}
        }
    }
    cwds
}

/// Splice `bind` to `upstream` until the process ends. Returns the address it
/// actually bound, which is the one to hand out.
///
/// Callers pass the same port on both sides on purpose: the URL becomes the
/// one already in your browser with the host swapped, rather than a number to
/// look up.
pub fn expose(bind: SocketAddr, upstream: SocketAddr) -> std::io::Result<SocketAddr> {
    let listener = TcpListener::bind(bind)?;
    let bound = listener.local_addr()?;
    std::thread::spawn(move || {
        for incoming in listener.incoming() {
            let Ok(from_phone) = incoming else { continue };
            // Dialled per connection rather than held open, so a dev server
            // that restarts is picked up with no bookkeeping — and one that
            // has gone away simply refuses, as it would locally.
            let Ok(to_server) = TcpStream::connect(upstream) else {
                continue;
            };
            forward::splice(from_phone, to_server);
        }
    });
    Ok(bound)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured verbatim from `lsof -nP -iTCP -sTCP:LISTEN -F pcn` on a
    /// working machine, trimmed to the interesting shapes: a wildcard bind, a
    /// v4 loopback, a v6 loopback, and one process listening twice.
    const LISTENERS: &str = "\
p900
crapportd
f17
n*:51892
p3650
credis-server
f6
n127.0.0.1:6379
f7
n[::1]:6379
p52149
cnode
f22
n[::1]:5173
p39057
cnode
f22
n*:3099
";

    const CWDS: &str = "\
p900
fcwd
n/
p3650
fcwd
n/Users/me
p52149
fcwd
n/Users/me/Code/site
p39057
fcwd
n/Users/me/Code/site/packages/api
";

    #[test]
    fn listeners_are_read_with_their_address_family() {
        let listeners = parse_listeners(LISTENERS);
        assert_eq!(listeners.len(), 5);

        let redis: Vec<&Listener> = listeners.iter().filter(|l| l.port == 6379).collect();
        assert_eq!(redis.len(), 2, "v4 and v6 are separate files");
        assert!(redis.iter().all(|l| l.loopback && l.command == "redis-server"));

        let vite = listeners.iter().find(|l| l.port == 5173).unwrap();
        assert!(vite.loopback, "[::1] is loopback");
        assert_eq!(vite.pid, 52149);

        // A wildcard bind is already reachable over the tailnet.
        assert!(!listeners.iter().find(|l| l.port == 3099).unwrap().loopback);
        assert!(!listeners.iter().find(|l| l.port == 51892).unwrap().loopback);
    }

    #[test]
    fn a_port_is_listed_once_however_many_families_it_binds() {
        let listeners = parse_listeners(LISTENERS);
        let cwds = parse_cwds(CWDS);

        let mut servers: Vec<DevServer> = Vec::new();
        for listener in listeners {
            let Some(cwd) = cwds.get(&listener.pid) else {
                continue;
            };
            if let Some(existing) = servers.iter_mut().find(|s| s.port == listener.port) {
                existing.loopback_only &= listener.loopback;
                continue;
            }
            servers.push(DevServer {
                port: listener.port,
                command: listener.command,
                cwd: cwd.clone(),
                loopback_only: listener.loopback,
            });
        }
        assert_eq!(servers.iter().filter(|s| s.port == 6379).count(), 1);
    }

    #[test]
    fn cwds_are_read_per_process() {
        let cwds = parse_cwds(CWDS);
        assert_eq!(cwds[&52149], PathBuf::from("/Users/me/Code/site"));
        assert_eq!(cwds[&900], PathBuf::from("/"));
    }

    /// The point of the filter: a scan of a working machine finds databases
    /// and model servers next to the dev servers, and those are not ours to
    /// put on the tailnet.
    #[test]
    fn only_what_runs_inside_a_project_is_eligible() {
        let servers = vec![
            DevServer {
                port: 6379,
                command: "redis-server".into(),
                cwd: PathBuf::from("/Users/me"),
                loopback_only: true,
            },
            DevServer {
                port: 5173,
                command: "node".into(),
                cwd: PathBuf::from("/Users/me/Code/site"),
                loopback_only: true,
            },
            DevServer {
                port: 3099,
                command: "node".into(),
                cwd: PathBuf::from("/Users/me/Code/site/packages/api"),
                loopback_only: false,
            },
        ];
        let site = uuid::Uuid::new_v4();
        let roots = vec![(PathBuf::from("/Users/me/Code/site"), site)];

        let owned = owned_by(&servers, &roots);
        let ports: Vec<u16> = owned.iter().map(|(s, _)| s.port).collect();
        assert_eq!(ports, vec![5173, 3099], "redis is not in a project");
        assert!(owned.iter().all(|(_, project)| *project == site));
    }

    /// A worktree is its own directory, and the session working there belongs
    /// to the project — so the deeper root has to win.
    #[test]
    fn a_worktree_claims_its_own_servers() {
        let (project, other) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        let servers = vec![DevServer {
            port: 5174,
            command: "node".into(),
            cwd: PathBuf::from("/Users/me/Code/site/.worktrees/feature"),
            loopback_only: true,
        }];
        let roots = vec![
            (PathBuf::from("/Users/me/Code/site"), other),
            (PathBuf::from("/Users/me/Code/site/.worktrees/feature"), project),
        ];

        let owned = owned_by(&servers, &roots);
        assert_eq!(owned[0].1, project, "the longest matching root wins");
    }

    /// The whole point, end to end: something listening only on loopback,
    /// reached through the forwarder from another address.
    ///
    /// In production the bind side is the tailnet address. It cannot be that
    /// here — a machine does not reliably reach its own tailnet address, and
    /// this one does not. What crosses the wire is the same code either way.
    #[test]
    fn a_loopback_server_is_reachable_through_the_forwarder() {
        use std::io::{BufRead, BufReader, Write};

        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream = server.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in server.incoming().flatten() {
                let mut out = stream.try_clone().unwrap();
                let mut line = String::new();
                BufReader::new(stream).read_line(&mut line).unwrap();
                out.write_all(format!("served {}\n", line.trim()).as_bytes()).unwrap();
            }
        });

        let front = expose("127.0.0.1:0".parse().unwrap(), upstream).expect("the forwarder binds");
        assert_ne!(front.port(), upstream.port(), "a different socket entirely");

        let mut client = TcpStream::connect(front).unwrap();
        client.write_all(b"a request\n").unwrap();
        let mut reply = String::new();
        BufReader::new(client).read_line(&mut reply).unwrap();
        assert_eq!(reply, "served a request\n");
    }

    #[test]
    fn a_port_the_os_handed_out_is_not_a_dev_server() {
        // Phoenix's distribution port and ollama's helper turn up on a real
        // machine; a dev server is on a number someone chose.
        assert!(63327 >= EPHEMERAL_FROM && 49541 >= EPHEMERAL_FROM);
        assert!(5173 < EPHEMERAL_FROM && 4000 < EPHEMERAL_FROM && 33060 < EPHEMERAL_FROM);
    }

    #[test]
    fn an_established_connection_is_not_a_listener() {
        assert!(split_address("127.0.0.1:5173->127.0.0.1:60123").is_none());
        assert_eq!(split_address("[::1]:5173"), Some(("[::1]", 5173)));
        assert_eq!(split_address("*:3099"), Some(("*", 3099)));
        assert!(split_address("nonsense").is_none());
    }
}

