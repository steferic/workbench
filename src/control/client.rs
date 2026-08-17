//! Talking to a running workbench over its control socket.
//!
//! The other side of `crate::control`, used by the `workbench` CLI. Kept here
//! rather than in `cli` so the request shapes are written once and the server
//! and its first client cannot drift apart.

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use super::socket_path;

/// An open connection. One request per line, one reply per line.
pub struct Client {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: u64,
}

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

/// Resolve what the user typed to exactly one agent id.
///
/// Deliberately not the roster used by `ask`/`handoff`: that is per-workspace
/// and needs `WORKBENCH_SESSION`, so it cannot serve a script running outside
/// an agent pane. The socket already reports every agent on the machine, which
/// is the right scope for "wait for this one".
///
/// Accepts a short id (or an unambiguous prefix of one) and a provider name
/// when only one such agent is running — the convention the other commands
/// document. Ambiguity is an error naming the candidates, never a guess: the
/// wrong agent silently is worse than a question.
pub fn resolve_agent(agents: &[Value], target: &str) -> Result<String> {
    let wanted = target.trim();
    if wanted.is_empty() {
        bail!("name an agent");
    }
    let id_of = |agent: &Value| agent.get("id").and_then(Value::as_str).unwrap_or("").to_string();

    if let Some(exact) = agents
        .iter()
        .find(|agent| id_of(agent).eq_ignore_ascii_case(wanted))
    {
        return Ok(id_of(exact));
    }

    let by_prefix: Vec<String> = agents
        .iter()
        .filter(|agent| {
            id_of(agent)
                .to_lowercase()
                .starts_with(&wanted.to_lowercase())
        })
        .map(id_of)
        .collect();
    if by_prefix.len() == 1 {
        return Ok(by_prefix[0].clone());
    }
    if by_prefix.len() > 1 {
        bail!("`{wanted}` matches {}", by_prefix.join(", "));
    }

    let by_provider: Vec<&Value> = agents
        .iter()
        .filter(|agent| {
            agent
                .get("provider")
                .and_then(Value::as_str)
                .map(|provider| provider.eq_ignore_ascii_case(wanted))
                .unwrap_or(false)
        })
        .collect();
    match by_provider.len() {
        1 => Ok(id_of(by_provider[0])),
        0 => bail!("no agent matches `{wanted}`"),
        _ => bail!(
            "several {wanted} agents are running — name one of {}",
            by_provider
                .iter()
                .map(|agent| {
                    let project = agent
                        .get("project")
                        .and_then(Value::as_str)
                        .unwrap_or("?");
                    format!("{} ({project})", id_of(agent))
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str, provider: &str, project: &str) -> Value {
        json!({"id": id, "provider": provider, "project": project})
    }

    #[test]
    fn an_id_or_a_prefix_resolves() {
        let agents = vec![
            agent("abc12345", "Claude", "workbench"),
            agent("def67890", "Codex", "workbench"),
        ];
        assert_eq!(resolve_agent(&agents, "abc12345").unwrap(), "abc12345");
        assert_eq!(resolve_agent(&agents, "ABC12345").unwrap(), "abc12345");
        assert_eq!(resolve_agent(&agents, "def").unwrap(), "def67890");
    }

    #[test]
    fn a_provider_resolves_only_when_it_is_unique() {
        let one = vec![
            agent("abc12345", "Claude", "workbench"),
            agent("def67890", "Codex", "workbench"),
        ];
        assert_eq!(resolve_agent(&one, "codex").unwrap(), "def67890");

        let two = vec![
            agent("abc12345", "Codex", "workbench"),
            agent("def67890", "Codex", "canvas"),
        ];
        let err = resolve_agent(&two, "codex").unwrap_err().to_string();
        assert!(err.contains("abc12345") && err.contains("def67890"), "{err}");
    }

    /// Typing into the wrong agent because a prefix was ambiguous is the exact
    /// failure worth being noisy about.
    #[test]
    fn an_ambiguous_prefix_is_an_error() {
        let agents = vec![
            agent("ab111111", "Claude", "workbench"),
            agent("ab222222", "Claude", "workbench"),
        ];
        let err = resolve_agent(&agents, "ab").unwrap_err().to_string();
        assert!(err.contains("ab111111"), "{err}");

        assert!(resolve_agent(&agents, "nope").is_err());
        assert!(resolve_agent(&agents, "  ").is_err());
    }
}
