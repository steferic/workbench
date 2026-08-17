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

/// Where to look when what the user typed could mean several agents.
#[derive(Debug, Default, Clone)]
pub struct Scope {
    /// The project to prefer, as a workspace id. Taken from the caller's own
    /// pane when it has one, or named outright with `--project`.
    pub project_id: Option<String>,
    /// The caller's own short id, so an agent asking for "codex" never
    /// resolves to itself.
    pub exclude: Option<String>,
}

impl Scope {
    /// What the environment already knows: an agent running in a workbench
    /// pane is told which session and workspace it is.
    pub fn from_env() -> Self {
        Self {
            project_id: std::env::var(crate::comms::ENV_WORKSPACE).ok(),
            exclude: std::env::var(crate::comms::ENV_SESSION).ok(),
        }
    }
}

fn field<'a>(agent: &'a Value, key: &str) -> &'a str {
    agent.get(key).and_then(Value::as_str).unwrap_or("")
}

/// `abc12345 (backend, workbench)` — enough to retype as an unambiguous
/// address, which is the only reason an error lists candidates at all.
fn describe(agent: &Value) -> String {
    let id = field(agent, "id");
    let project = field(agent, "project");
    match agent.get("alias").and_then(Value::as_str) {
        Some(alias) if !alias.is_empty() => format!("{id} ({alias}, {project})"),
        _ => format!("{id} ({project})"),
    }
}

/// Prefer the caller's own project when a name means several agents.
///
/// This is what makes a bare provider name usable again. Several Claudes run
/// at once across projects, so `wait claude` was almost always ambiguous —
/// but from inside a pane it nearly always means "the Claude working on this
/// with me", and that one is a single filter away.
///
/// Only ever narrows an ambiguity; if the caller's project holds none of the
/// candidates, the wider set stands and the error names them all.
fn narrow<'a>(matches: Vec<&'a Value>, scope: &Scope) -> Vec<&'a Value> {
    if matches.len() <= 1 {
        return matches;
    }
    let Some(home) = scope.project_id.as_deref() else {
        return matches;
    };
    let local: Vec<&Value> = matches
        .iter()
        .copied()
        .filter(|agent| field(agent, "project_id") == home)
        .collect();
    if local.is_empty() { matches } else { local }
}

/// Resolve what the user typed to exactly one agent id.
///
/// Deliberately not the roster used by `ask`/`handoff`: that is per-workspace
/// and needs `WORKBENCH_SESSION`, so it cannot serve a script running outside
/// an agent pane. The socket already reports every agent on the machine, which
/// is the right scope for "wait for this one".
///
/// Tried in order of how specific the address is — id, then alias, then an id
/// prefix, then a provider name — and each is narrowed to the caller's project
/// before being called ambiguous. Ambiguity that survives that is an error
/// naming the candidates, never a guess: waiting on the wrong agent silently
/// is worse than a question.
pub fn resolve_agent(agents: &[Value], target: &str, scope: &Scope) -> Result<String> {
    let wanted = target.trim().to_lowercase();
    if wanted.is_empty() {
        bail!("name an agent");
    }

    let pool: Vec<&Value> = agents
        .iter()
        .filter(|agent| match scope.exclude.as_deref() {
            Some(me) => !field(agent, "id").eq_ignore_ascii_case(me),
            None => true,
        })
        .collect();

    let strategies: [(&str, Box<dyn Fn(&Value) -> bool>); 4] = [
        (
            "id",
            Box::new({
                let wanted = wanted.clone();
                move |a: &Value| field(a, "id").to_lowercase() == wanted
            }),
        ),
        (
            "alias",
            Box::new({
                let wanted = wanted.clone();
                move |a: &Value| {
                    let alias = field(a, "alias").to_lowercase();
                    !alias.is_empty() && alias == wanted
                }
            }),
        ),
        (
            "prefix",
            Box::new({
                let wanted = wanted.clone();
                move |a: &Value| field(a, "id").to_lowercase().starts_with(&wanted)
            }),
        ),
        (
            "provider",
            Box::new({
                let wanted = wanted.clone();
                move |a: &Value| field(a, "provider").to_lowercase() == wanted
            }),
        ),
    ];

    for (kind, matches_it) in strategies {
        let found = narrow(
            pool.iter().copied().filter(|a| matches_it(a)).collect(),
            scope,
        );
        match found.len() {
            0 => continue,
            1 => return Ok(field(found[0], "id").to_string()),
            _ => {
                let names: Vec<String> = found.iter().map(|a| describe(a)).collect();
                // Which advice actually helps depends on why it is ambiguous.
                // Candidates spread across projects can be narrowed by one;
                // candidates sharing a project cannot, and saying otherwise
                // sends the reader somewhere that will not work. (The scope
                // may be set and still not have narrowed anything — a caller
                // in a project that holds none of these.)
                let mut projects: Vec<&str> =
                    found.iter().map(|a| field(a, "project")).collect();
                projects.sort_unstable();
                projects.dedup();
                let hint = if projects.len() > 1 {
                    " — pass --project, or address one by id or alias"
                } else {
                    " — address one by id, or give it an alias"
                };
                bail!(
                    "`{target}` matches {} agents by {kind}: {}{hint}",
                    found.len(),
                    names.join(", ")
                );
            }
        }
    }

    bail!("no agent matches `{target}`")
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

    fn anywhere() -> Scope {
        Scope::default()
    }

    fn inside(project: &str) -> Scope {
        Scope {
            project_id: Some(format!("{project}-id")),
            exclude: None,
        }
    }

    #[test]
    fn an_id_or_a_prefix_resolves() {
        let agents = vec![
            agent("abc12345", "Claude", "workbench", None),
            agent("def67890", "Codex", "workbench", None),
        ];
        assert_eq!(resolve_agent(&agents, "abc12345", &anywhere()).unwrap(), "abc12345");
        assert_eq!(resolve_agent(&agents, "ABC12345", &anywhere()).unwrap(), "abc12345");
        assert_eq!(resolve_agent(&agents, "def", &anywhere()).unwrap(), "def67890");
    }

    /// An alias is the only address that survives a restart, so it outranks
    /// every guess below it.
    #[test]
    fn an_alias_resolves_and_beats_a_provider_name() {
        let agents = vec![
            agent("abc12345", "Claude", "workbench", Some("backend")),
            agent("def67890", "Claude", "workbench", None),
        ];
        assert_eq!(resolve_agent(&agents, "backend", &anywhere()).unwrap(), "abc12345");
        assert_eq!(resolve_agent(&agents, "BACKEND", &anywhere()).unwrap(), "abc12345");
    }

    /// The fix this exists for: several Claudes run at once, so a bare
    /// provider name was almost always ambiguous. From inside a project it
    /// means the one working on that project.
    #[test]
    fn a_provider_name_resolves_within_the_callers_project() {
        let agents = vec![
            agent("abc12345", "Claude", "workbench", None),
            agent("def67890", "Claude", "canvas", None),
            agent("aaa11111", "Codex", "workbench", None),
        ];
        // From nowhere in particular it is still ambiguous, and says so.
        let err = resolve_agent(&agents, "claude", &anywhere()).unwrap_err().to_string();
        assert!(err.contains("abc12345") && err.contains("def67890"), "{err}");
        assert!(err.contains("--project"), "spread across projects: --project helps: {err}");

        // Two in the SAME project: --project cannot help, so do not suggest it.
        let together = vec![
            agent("abc12345", "Claude", "workbench", None),
            agent("aaa11111", "Claude", "workbench", None),
        ];
        let err = resolve_agent(&together, "claude", &inside("workbench")).unwrap_err().to_string();
        assert!(!err.contains("--project"), "should not send them somewhere useless: {err}");
        assert!(err.contains("alias"), "{err}");

        // From inside one, it is not.
        assert_eq!(resolve_agent(&agents, "claude", &inside("workbench")).unwrap(), "abc12345");
        assert_eq!(resolve_agent(&agents, "claude", &inside("canvas")).unwrap(), "def67890");
    }

    /// Scoping narrows an ambiguity; it must not hide the only match there is.
    #[test]
    fn scoping_does_not_hide_an_agent_in_another_project() {
        let agents = vec![agent("def67890", "Codex", "canvas", None)];
        assert_eq!(resolve_agent(&agents, "codex", &inside("workbench")).unwrap(), "def67890");
    }

    /// An agent asking for "codex" means a peer, never itself — otherwise
    /// `wait` returns instantly on the caller's own state.
    #[test]
    fn an_agent_never_resolves_to_itself() {
        let agents = vec![
            agent("abc12345", "Codex", "workbench", None),
            agent("def67890", "Codex", "workbench", None),
        ];
        let me = Scope {
            project_id: Some("workbench-id".into()),
            exclude: Some("abc12345".into()),
        };
        assert_eq!(resolve_agent(&agents, "codex", &me).unwrap(), "def67890");
    }

    #[test]
    fn an_ambiguous_prefix_is_an_error_that_names_the_candidates() {
        let agents = vec![
            agent("ab111111", "Claude", "workbench", Some("one")),
            agent("ab222222", "Claude", "workbench", Some("two")),
        ];
        let err = resolve_agent(&agents, "ab", &inside("workbench")).unwrap_err().to_string();
        assert!(err.contains("ab111111") && err.contains("one"), "{err}");
        assert!(err.contains("ab222222") && err.contains("two"), "{err}");

        assert!(resolve_agent(&agents, "nope", &anywhere()).is_err());
        assert!(resolve_agent(&agents, "  ", &anywhere()).is_err());
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
