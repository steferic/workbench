//! Talking to a running workbench over its control socket.
//!
//! The other side of `crate::control`, used by the `workbench` CLI. Kept here
//! rather than in `cli` so the request shapes are written once and the server
//! and its first client cannot drift apart.

use anyhow::{Result, bail};
#[cfg(unix)]
use anyhow::anyhow;
use serde_json::{Value, json};
#[cfg(unix)]
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::time::Duration;

#[cfg(unix)]
use super::socket_path;

/// Where there is no Unix socket there is no client. Kept as a type so the
/// `wait` command compiles everywhere and fails with a sentence rather than
/// being silently absent from the CLI on one platform.
#[cfg(not(unix))]
pub struct Client;

#[cfg(not(unix))]
impl Client {
    pub fn connect() -> Result<Self> {
        bail!("the control socket needs a Unix socket")
    }
    pub fn set_timeout(&mut self, _timeout: Option<Duration>) -> Result<()> {
        bail!("unsupported")
    }
    pub fn call(&mut self, _method: &str, _params: Value) -> Result<Value> {
        bail!("unsupported")
    }
    pub fn subscribe(&mut self) -> Result<()> {
        bail!("unsupported")
    }
    pub fn next_event(&mut self) -> Result<Option<Value>> {
        bail!("unsupported")
    }
}

/// An open connection. One request per line, one reply per line.
#[cfg(unix)]
pub struct Client {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: u64,
}

#[cfg(unix)]
impl Client {
    /// Connect to the workbench on this machine.
    ///
    /// A missing socket is the ordinary case of "the TUI is not running", so it
    /// says that rather than reporting a bare ENOENT on a path.
    pub fn connect() -> Result<Self> {
        let path = socket_path()?;
        let stream = UnixStream::connect(&path).map_err(|err| {
            anyhow!(
                "no workbench listening on {} ({err}) — is the TUI running?",
                path.display()
            )
        })?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self {
            stream,
            reader,
            next_id: 1,
        })
    }

    /// How long to wait for any single line. `None` blocks indefinitely.
    pub fn set_timeout(&mut self, timeout: Option<Duration>) -> Result<()> {
        self.stream.set_read_timeout(timeout)?;
        Ok(())
    }

    /// Send a request and return its `result`, turning an error reply into an
    /// `Err` so callers can use `?`.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({"id": id, "method": method, "params": params});
        writeln!(self.stream, "{request}")?;
        self.stream.flush()?;

        // Events and replies share the stream once subscribed, so skip past
        // anything that is not the answer to this request.
        loop {
            let line = self.read_line()?;
            let value: Value = serde_json::from_str(&line)?;
            if value.get("event").is_some() {
                continue;
            }
            if let Some(error) = value.get("error") {
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error");
                let code = error.get("code").and_then(Value::as_str).unwrap_or("error");
                bail!("{message} ({code})");
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    /// Ask to be told when things change. Events then arrive interleaved with
    /// replies, which `call` and `next_event` each step over.
    pub fn subscribe(&mut self) -> Result<()> {
        self.call("events.subscribe", json!({}))?;
        Ok(())
    }

    /// The next event, skipping any reply that arrives first. `None` on
    /// timeout; `Err` if the connection went away.
    pub fn next_event(&mut self) -> Result<Option<Value>> {
        loop {
            match self.read_line() {
                Ok(line) => {
                    let value: Value = serde_json::from_str(&line)?;
                    if value.get("event").is_some() {
                        return Ok(Some(value));
                    }
                }
                Err(err) => {
                    // A read timeout is "nothing yet", not a failure. Both
                    // kinds show up here depending on platform and whether the
                    // socket was mid-line.
                    if let Some(io) = err.downcast_ref::<std::io::Error>() {
                        if matches!(
                            io.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) {
                            return Ok(None);
                        }
                    }
                    return Err(err);
                }
            }
        }
    }

    fn read_line(&mut self) -> Result<String> {
        let mut line = String::new();
        let read = self.reader.read_line(&mut line)?;
        if read == 0 {
            bail!("workbench closed the connection");
        }
        Ok(line)
    }
}

pub use crate::resolve::Scope;

fn field<'a>(agent: &'a Value, key: &str) -> &'a str {
    agent.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Resolve what the user typed to exactly one agent id.
///
/// Deliberately not scoped to one workspace the way the roster is: the socket
/// already reports every agent on the machine, which is the right scope for
/// "wait for this one" — including from a plain shell, where there is no
/// `WORKBENCH_SESSION` to say which workspace the caller is in.
///
/// The ladder itself lives in `crate::resolve`, shared with the comms verbs so
/// a name cannot mean one agent after `wait` and another after `ask`. `wait`
/// asks for `Anywhere`: it reads no state and spends nobody's turn, so
/// reaching into another project to answer an unambiguous name is a
/// convenience rather than a risk.
pub fn resolve_agent(agents: &[Value], target: &str, scope: &Scope) -> Result<String> {
    let candidates: Vec<crate::resolve::Candidate> = agents
        .iter()
        .map(|agent| crate::resolve::Candidate {
            id: field(agent, "id"),
            alias: Some(field(agent, "alias")).filter(|alias| !alias.is_empty()),
            provider: field(agent, "provider"),
            project_id: field(agent, "project_id"),
            project: field(agent, "project"),
        })
        .collect();

    match crate::resolve::pick(&candidates, target, scope, crate::resolve::Reach::Anywhere) {
        Ok(index) => Ok(candidates[index].id.to_string()),
        Err(message) => bail!("{message}"),
    }
}

/// Turn a project name into its workspace id, for `--project`.
pub fn resolve_project(projects: &[Value], name: &str) -> Result<String> {
    let wanted = name.trim().to_lowercase();
    let matches: Vec<&Value> = projects
        .iter()
        .filter(|project| field(project, "name").to_lowercase() == wanted)
        .collect();
    match matches.len() {
        1 => Ok(field(matches[0], "id").to_string()),
        0 => bail!(
            "no project named `{name}` — open ones are {}",
            projects
                .iter()
                .map(|p| field(p, "name").to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => bail!("several projects are named `{name}`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str, provider: &str, project: &str, alias: Option<&str>) -> Value {
        json!({
            "id": id, "provider": provider, "project": project,
            "project_id": format!("{project}-id"), "alias": alias,
        })
    }

    /// The ladder itself is tested in `crate::resolve`; what belongs here is
    /// the projection off the wire — a missing or null `alias` must read as
    /// "no alias" rather than as an agent addressable by the empty string.
    #[test]
    fn socket_rows_project_onto_candidates() {
        let agents = vec![
            agent("abc12345", "Claude", "workbench", Some("backend")),
            agent("def67890", "Codex", "workbench", None),
            json!({"id": "aaa11111", "provider": "Codex", "project": "canvas"}),
        ];
        let scope = Scope::default();
        assert_eq!(resolve_agent(&agents, "backend", &scope).unwrap(), "abc12345");
        assert_eq!(resolve_agent(&agents, "def", &scope).unwrap(), "def67890");
        // A row with no `alias` key at all still resolves by its other fields.
        assert_eq!(resolve_agent(&agents, "aaa11111", &scope).unwrap(), "aaa11111");
        // And no agent is addressable as "".
        assert!(resolve_agent(&agents, "", &scope).is_err());
    }

    /// `wait` reaches across projects for an unambiguous name — the
    /// convenience the comms verbs deliberately decline (see `crate::resolve`).
    #[test]
    fn wait_resolves_a_name_into_another_project() {
        let agents = vec![agent("def67890", "Codex", "canvas", None)];
        let scope = Scope {
            project_id: Some("workbench-id".into()),
            exclude: None,
        };
        assert_eq!(resolve_agent(&agents, "codex", &scope).unwrap(), "def67890");
    }

    #[test]
    fn a_project_name_resolves_to_its_id() {
        let projects = vec![
            json!({"id": "p1", "name": "workbench"}),
            json!({"id": "p2", "name": "canvas"}),
        ];
        assert_eq!(resolve_project(&projects, "canvas").unwrap(), "p2");
        let err = resolve_project(&projects, "nope").unwrap_err().to_string();
        assert!(err.contains("workbench") && err.contains("canvas"), "{err}");
    }
}
