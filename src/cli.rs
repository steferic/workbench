//! `workbench` CLI verbs used by agents (and humans) for agent-to-agent
//! comms. These are pure file readers/writers against the comms directory —
//! the running TUI does the live work (see `app::comms_tick`).

use crate::comms::{self, Directory, InboxMessage, Reply};
use crate::resolve::Scope;
use anyhow::{anyhow, bail, Result};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Identity of the calling process, from the env vars workbench injects at
/// PTY spawn (absent when run from a plain shell).
struct CallerCtx {
    workspace_id: String,
    session: Option<String>,
}

fn caller_ctx() -> Result<CallerCtx> {
    let session = std::env::var(comms::ENV_SESSION).ok();
    let workspace_id = match std::env::var(comms::ENV_WORKSPACE) {
        Ok(ws) => ws,
        Err(_) => {
            let cwd = std::env::current_dir()?;
            comms::find_workspace_for_cwd(&cwd)?
        }
    };
    Ok(CallerCtx {
        workspace_id,
        session,
    })
}

impl CallerCtx {
    /// Where a bare name is looked up first. The caller's own project, and
    /// never the caller itself.
    fn scope(&self) -> Scope {
        Scope {
            project_id: Some(self.workspace_id.clone()),
            exclude: self.session.clone(),
        }
    }
}

pub fn cmd_agents(all: bool) -> Result<()> {
    let ctx = caller_ctx()?;
    let directory = Directory::load()?;
    let roster = comms::load_roster(&ctx.workspace_id)?;
    println!(
        "workspace: {} ({})  updated: {}",
        roster.workspace_name, roster.workspace_path, roster.updated_at
    );
    if roster.agents.is_empty() {
        println!("no agent sessions");
    }
    for a in &roster.agents {
        let you = if Some(&a.id) == ctx.session.as_ref() {
            "  (you)"
        } else {
            ""
        };
        let alias = a
            .alias
            .as_deref()
            .map(|al| format!("  alias:{al}"))
            .unwrap_or_default();
        let consult = if a.supports_consult { "" } else { "  [no-consult]" };
        println!(
            "{}  {:<8}{}{}  branch:{}  {}{}  cwd:{}",
            a.id, a.provider, alias, you, a.branch, a.status, consult, a.cwd
        );
    }

    // Agents elsewhere are summarised rather than listed by default. This
    // output is read by a model on every turn it collaborates, and most turns
    // are about the project it is sitting in; a machine-wide dump would bury
    // the three peers that matter under twenty that do not.
    let elsewhere = directory.outside(&ctx.workspace_id);
    if !elsewhere.is_empty() {
        println!();
        if all {
            println!("in other projects:");
            let mut current = "";
            for entry in &elsewhere {
                if entry.workspace_name != current {
                    current = &entry.workspace_name;
                    println!("  {current}");
                }
                let alias = entry
                    .agent
                    .alias
                    .as_deref()
                    .map(|al| format!("  alias:{al}"))
                    .unwrap_or_default();
                let consult = if entry.agent.supports_consult {
                    ""
                } else {
                    "  [no-consult]"
                };
                println!(
                    "    {}  {:<8}{}  {}{}",
                    entry.agent.id, entry.agent.provider, alias, entry.agent.status, consult
                );
            }
        } else {
            // Named, not enumerated. A machine running a dozen projects would
            // otherwise spend a paragraph here on peers this turn is not about.
            let mut projects: Vec<&str> = elsewhere
                .iter()
                .map(|entry| entry.workspace_name.as_str())
                .collect();
            projects.dedup();
            const NAMED: usize = 4;
            let shown = projects.len().min(NAMED);
            let more = match projects.len() - shown {
                0 => String::new(),
                rest => format!(" and {rest} more"),
            };
            println!(
                "{} agent(s) in {} ({}{}) — workbench agents --all to list them",
                elsewhere.len(),
                if projects.len() == 1 {
                    "another project".to_string()
                } else {
                    format!("{} other projects", projects.len())
                },
                projects[..shown].join(", "),
                more
            );
        }
    }

    println!("\naddress a peer by id or alias (provider name works when unique):");
    println!("  workbench transcript <id> --lines 200");
    println!("  workbench ask <id> \"question\"");
    if !elsewhere.is_empty() {
        println!("a peer in another project needs its full id — a bare name means this project");
    }
    Ok(())
}

pub fn cmd_transcript(target: String, lines: usize, all: bool) -> Result<()> {
    let ctx = caller_ctx()?;
    let directory = Directory::load()?;
    let entry = directory
        .resolve(&target, &ctx.scope())
        .map_err(|e| anyhow!(e))?;
    let agent = &entry.agent;
    let Some(path) = agent.transcript.as_ref().map(PathBuf::from) else {
        bail!(
            "{} ({}) has no transcript — it is not a transcript-capable agent",
            agent.id,
            agent.provider
        );
    };
    let text = std::fs::read_to_string(&path).map_err(|_| {
        anyhow!(
            "no transcript exported yet for {} — it may not have completed a turn",
            agent.id
        )
    })?;
    if all {
        print!("{text}");
    } else {
        println!("{}", comms::tail_lines(&text, lines));
    }
    Ok(())
}

/// Structured-handoff prompt: research on agent handoffs shows a structured
/// payload (done/remaining/decisions/gotchas) beats both raw transcripts and
/// free-form summaries, and that the live author narrates its own work far
/// better than a reader inferring from logs.
const HANDOFF_PROMPT: &str = "Another agent is preparing to take over or build on your work in this \
workspace. Produce a structured handoff summary:\n\
1. Objective — what you were asked to do, in one or two sentences\n\
2. Completed — what is done and where (files, branches, commits)\n\
3. Remaining — concrete next steps, in order\n\
4. Key decisions — choices you made and WHY (including approaches you tried and rejected)\n\
5. Gotchas — surprises, fragile spots, anything a successor would waste time rediscovering\n\
Be concrete: real paths, real names. Skip process narration.";

pub fn cmd_handoff(target: String, wait: bool, timeout_secs: u64) -> Result<()> {
    cmd_ask(target, HANDOFF_PROMPT.to_string(), wait, timeout_secs)
}

pub fn cmd_ask(target: String, message: String, wait: bool, timeout_secs: u64) -> Result<()> {
    let ctx = caller_ctx()?;
    let Some(from) = ctx.session.clone() else {
        bail!(
            "`workbench ask` must run inside a workbench agent session ({} is not set)",
            comms::ENV_SESSION
        );
    };
    let directory = Directory::load()?;
    let entry = directory
        .resolve(&target, &ctx.scope())
        .map_err(|e| anyhow!(e))?;
    let agent = &entry.agent;
    if !agent.supports_consult {
        bail!(
            "{} ({}) does not support consults; read its files or transcript instead",
            agent.id,
            agent.provider
        );
    }
    if message.trim().is_empty() {
        bail!("empty message");
    }

    let ticket = comms::new_ticket();
    let msg = InboxMessage::Ask {
        ticket: ticket.clone(),
        from,
        to: agent.id.clone(),
        to_workspace: Some(entry.workspace_id.clone()),
        message,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    // Written to the ASKER's inbox even when the target is elsewhere: the
    // reply is written back beside it, which is the directory this process is
    // about to poll. Routing by the message's own `to_workspace` keeps the
    // waiting side ignorant of where the answer comes from.
    comms::write_inbox(&ctx.workspace_id, &msg)?;
    let elsewhere = if entry.workspace_id == ctx.workspace_id {
        String::new()
    } else {
        format!(" in project {}", entry.workspace_name)
    };
    println!(
        "queued consult {ticket} for {} ({}){elsewhere} — delivered when it is idle",
        agent.id, agent.provider
    );
    if wait {
        wait_for_reply(&ctx.workspace_id, &ticket, timeout_secs)
    } else {
        println!("collect with: workbench replies {ticket} --wait");
        Ok(())
    }
}

pub fn cmd_replies(ticket: String, wait: bool, timeout_secs: u64) -> Result<()> {
    let ctx = caller_ctx()?;
    if wait {
        wait_for_reply(&ctx.workspace_id, &ticket, timeout_secs)
    } else {
        match read_reply(&ctx.workspace_id, &ticket)? {
            Some(reply) => print_reply(&reply),
            None => println!("no reply yet for {ticket} (try --wait)"),
        }
        Ok(())
    }
}

pub fn cmd_alias(name: String) -> Result<()> {
    let ctx = caller_ctx()?;
    let Some(from) = ctx.session.clone() else {
        bail!(
            "`workbench alias` must run inside a workbench agent session ({} is not set)",
            comms::ENV_SESSION
        );
    };
    let ticket = comms::new_ticket();
    comms::write_inbox(
        &ctx.workspace_id,
        &InboxMessage::Alias {
            ticket: ticket.clone(),
            from,
            alias: name.clone(),
        },
    )?;
    // Aliases apply within a second; wait briefly to report the outcome.
    match poll_reply(&ctx.workspace_id, &ticket, Duration::from_secs(5))? {
        Some(reply) if reply.status == "answered" => println!("alias set: {name}"),
        Some(reply) => bail!(
            "alias rejected: {}",
            reply.reason.unwrap_or_else(|| "unknown reason".into())
        ),
        None => println!("alias request queued (is the workbench TUI running?)"),
    }
    Ok(())
}

fn read_reply(workspace_id: &str, ticket: &str) -> Result<Option<Reply>> {
    let path = comms::reply_path(workspace_id, ticket)?;
    match std::fs::read(&path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(_) => Ok(None),
    }
}

fn poll_reply(workspace_id: &str, ticket: &str, timeout: Duration) -> Result<Option<Reply>> {
    let start = Instant::now();
    loop {
        if let Some(reply) = read_reply(workspace_id, ticket)? {
            return Ok(Some(reply));
        }
        if start.elapsed() > timeout {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn wait_for_reply(workspace_id: &str, ticket: &str, timeout_secs: u64) -> Result<()> {
    match poll_reply(workspace_id, ticket, Duration::from_secs(timeout_secs))? {
        Some(reply) => print_reply(&reply),
        None => bail!(
            "no reply to {ticket} within {timeout_secs}s — the peer may still be working; \
             check later with: workbench replies {ticket}"
        ),
    }
    Ok(())
}

fn print_reply(reply: &Reply) {
    match reply.status.as_str() {
        "answered" => {
            println!(
                "reply from {} (consult {}):\n",
                reply.to, reply.ticket
            );
            println!("{}", reply.reply.as_deref().unwrap_or(""));
        }
        status => {
            println!(
                "consult {} {}: {}",
                reply.ticket,
                status,
                reply.reason.as_deref().unwrap_or("")
            );
        }
    }
}

/// `workbench hook <event>` — called by the agent's own lifecycle hooks.
///
/// Two rules govern this verb. It must never fail the agent: a hook that exits
/// non-zero can interrupt a turn, so every error path here is swallowed and
/// the exit status is always success. And it must be quick — it runs inline
/// on events as frequent as every tool call, so it does one small read and one
/// atomic write, with no locking and no network.
pub fn cmd_hook(event: Option<&str>) {
    use std::io::Read;

    // The hook inherits its PTY's environment, which is how the event knows
    // whose it is. Outside a workbench-spawned agent there is nothing to
    // report against.
    let (Ok(workspace_id), Ok(session)) = (
        std::env::var(comms::ENV_WORKSPACE),
        std::env::var(comms::ENV_SESSION),
    ) else {
        return;
    };

    // Claude passes the event payload on stdin. Read it if it is there, but
    // never block waiting for a provider that sends nothing.
    let mut raw = String::new();
    let payload = if std::io::stdin().read_to_string(&mut raw).is_ok() && !raw.trim().is_empty() {
        serde_json::from_str::<serde_json::Value>(&raw).ok()
    } else {
        None
    };

    // Codex's hook command takes no arguments, so the event name arrives in
    // the payload instead; both providers put it in `hook_event_name`.
    let from_payload = payload
        .as_ref()
        .and_then(|p| p.get("hook_event_name"))
        .and_then(serde_json::Value::as_str);
    let Some(event) = event.or(from_payload) else {
        return;
    };

    if let Some(status) = crate::agent_status::interpret(event, payload.as_ref()) {
        let _ = crate::agent_status::record(&workspace_id, &session, &status);
    }
}

// ---------------------------------------------------------------------------
// Waiting on an agent (over the control socket)
// ---------------------------------------------------------------------------

/// What `wait` treats as "no longer working". An agent that stops to ask you
/// something has finished the turn as far as a script is concerned — and this
/// is the trap the default exists to avoid: `--state idle` alone would hang
/// forever on a permission prompt, because a blocked agent never becomes idle
/// until a human answers.
const SETTLED: &[&str] = &["idle", "blocked", "stopped"];

/// Exit code for "the state never arrived". Distinct from 1 so a script can
/// tell a timeout from a real failure — `wait || handle_timeout` is the whole
/// point of the command.
pub const EXIT_TIMEOUT: i32 = 3;

/// Block until an agent reaches one of `states`, then print what happened.
///
/// The ordering here is the only subtle part: subscribe *before* reading the
/// current state. The other way round, an agent that settles in the gap
/// between the read and the subscription is never reported, and the command
/// waits out its whole timeout on a condition that already came true.
pub fn cmd_wait(
    target: String,
    states: Option<String>,
    project: Option<String>,
    timeout_secs: u64,
    json_out: bool,
) -> Result<()> {
    use serde_json::{json, Value};

    let wanted: Vec<String> = match states.as_deref() {
        None => SETTLED.iter().map(|s| s.to_string()).collect(),
        Some(list) => {
            let states: Vec<String> = list
                .split(',')
                .map(|state| state.trim().to_lowercase())
                .filter(|state| !state.is_empty())
                .collect();
            if states.is_empty() {
                bail!("--state needs at least one state");
            }
            const KNOWN: &[&str] = &["idle", "working", "blocked", "stopped"];
            if let Some(unknown) = states.iter().find(|state| !KNOWN.contains(&state.as_str())) {
                bail!("unknown state `{unknown}` — expected one of {}", KNOWN.join(", "));
            }
            states
        }
    };

    let mut client = crate::control::Client::connect()?;

    // Subscribe first. See the note above: this order is what makes the
    // command race-free.
    client.subscribe()?;

    let agents = client.call("agents.list", json!({}))?;
    let agents = agents.as_array().cloned().unwrap_or_default();

    // The caller's own pane says which project it is in, which is what makes a
    // bare provider name usable — `wait codex` from inside a project means
    // that project's codex. `--project` says it outright, for a script run
    // from a plain shell where the environment cannot.
    let mut scope = crate::control::Scope::from_env();
    if let Some(name) = project.as_deref() {
        let projects = client.call("projects.list", json!({}))?;
        let projects = projects.as_array().cloned().unwrap_or_default();
        scope.project_id = Some(crate::control::resolve_project(&projects, name)?);
    }
    let agent = crate::control::resolve_agent(&agents, &target, &scope)?;

    let describe = |state: &str, waited: Duration| {
        if json_out {
            println!(
                "{}",
                json!({"agent": agent, "state": state, "waited_ms": waited.as_millis() as u64})
            );
        } else {
            println!("{agent} is {state} (after {:.1}s)", waited.as_secs_f64());
        }
    };

    let current = agents
        .iter()
        .find(|candidate| candidate.get("id").and_then(Value::as_str) == Some(agent.as_str()))
        .and_then(|candidate| candidate.get("status").and_then(Value::as_str))
        .unwrap_or("unknown")
        .to_string();
    if wanted.iter().any(|state| state == &current) {
        describe(&current, Duration::ZERO);
        return Ok(());
    }

    let started = Instant::now();
    let deadline = Duration::from_secs(timeout_secs);
    // Poll in slices rather than one long blocking read, so the deadline is
    // honoured even while events keep arriving for other agents.
    client.set_timeout(Some(Duration::from_millis(500)))?;

    while started.elapsed() < deadline {
        let Some(event) = client.next_event()? else {
            continue;
        };
        let name = event.get("event").and_then(Value::as_str).unwrap_or("");
        let data = event.get("data").cloned().unwrap_or(Value::Null);
        if data.get("agent").and_then(Value::as_str) != Some(agent.as_str()) {
            continue;
        }
        match name {
            "agent.status_changed" => {
                let to = data.get("to").and_then(Value::as_str).unwrap_or("unknown");
                if wanted.iter().any(|state| state == to) {
                    describe(to, started.elapsed());
                    return Ok(());
                }
            }
            // The agent went away. Waiting for a state it can no longer reach
            // would burn the whole timeout to report nothing useful.
            "agent.removed" => bail!("{agent} is gone"),
            _ => {}
        }
    }

    eprintln!(
        "{agent} did not reach {} within {timeout_secs}s",
        wanted.join(" or ")
    );
    std::process::exit(EXIT_TIMEOUT);
}


// ---- `workbench jobs` ------------------------------------------------------
//
// Everything but `run` is a file reader or writer against the repository, so
// it works with no TUI running — `report` in particular is called by an agent
// that may outlive the workbench that started it.

/// Where the project is: `--project` as a path or a name the running
/// workbench knows, else the pane's own workspace, else the nearest manifest
/// above the current directory. Says what it tried when nothing fits.
fn jobs_root(project: Option<&str>) -> Result<PathBuf> {
    if let Some(project) = project {
        let path = PathBuf::from(project);
        if path.is_dir() {
            return crate::jobs::find_root(&path.canonicalize()?)
                .ok_or_else(|| anyhow!("no {} in or above {project}", crate::jobs::MANIFEST));
        }
        let mut client = crate::control::Client::connect()
            .map_err(|err| anyhow!("`{project}` is not a directory, and {err}"))?;
        let projects = client.call("projects.list", serde_json::json!({}))?;
        let projects = projects.as_array().cloned().unwrap_or_default();
        let wanted = projects
            .iter()
            .find(|p| p.get("name").and_then(|n| n.as_str()).is_some_and(|n| n.eq_ignore_ascii_case(project)))
            .and_then(|p| p.get("path").and_then(|v| v.as_str()))
            .ok_or_else(|| anyhow!("workbench has no project named `{project}`"))?;
        return Ok(PathBuf::from(wanted));
    }
    if let Ok(workspace_id) = std::env::var(comms::ENV_WORKSPACE) {
        if let Ok(roster) = comms::load_roster(&workspace_id) {
            let path = PathBuf::from(&roster.workspace_path);
            if path.join(crate::jobs::MANIFEST).is_file() {
                return Ok(path);
            }
        }
    }
    let cwd = std::env::current_dir()?;
    crate::jobs::find_root(&cwd).ok_or_else(|| {
        anyhow!(
            "no {} in or above {} — run `workbench jobs init` there, or pass --project",
            crate::jobs::MANIFEST,
            cwd.display()
        )
    })
}

fn jobs_project(root: &std::path::Path) -> Result<crate::jobs::ProjectJobs> {
    let project = crate::jobs::load(root)
        .ok_or_else(|| anyhow!("no {} in {}", crate::jobs::MANIFEST, root.display()))?;
    if let Some(Err(error)) = &project.manifest {
        bail!("{}: {error}", crate::jobs::MANIFEST);
    }
    Ok(project)
}

fn describe_run(run: &crate::jobs::RunRecord) -> String {
    let mut line = format!(
        "{}  {:<10} {} on {}",
        run.started_utc.format("%Y-%m-%d %H:%M"),
        run.status.label(),
        run.by.user,
        run.by.host
    );
    if let Some(summary) = &run.summary {
        line.push_str("\n    ");
        line.push_str(summary.trim());
    }
    if let Some(artifacts) = &run.artifacts {
        line.push_str(&format!("\n    artifacts: {artifacts}"));
    }
    for lesson in &run.lessons {
        line.push_str(&format!("\n    lesson: {lesson}"));
    }
    line
}

pub fn cmd_jobs_list(project: Option<String>, json: bool) -> Result<()> {
    let root = jobs_root(project.as_deref())?;
    let project = jobs_project(&root)?;
    let now = chrono::Utc::now();
    if json {
        let rows: Vec<serde_json::Value> = project
            .jobs()
            .iter()
            .map(|job| {
                let last = project.last_run(&job.id);
                serde_json::json!({
                    "id": job.id,
                    "title": job.title,
                    "description": job.description,
                    "agent": job.agent,
                    "every": job.every.map(crate::jobs::manifest::describe_every),
                    "due": project.due(job, now),
                    "last": last,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    println!("{}  ({})", root.display(), crate::jobs::MANIFEST);
    if project.jobs().is_empty() {
        println!("no jobs");
    }
    for job in project.jobs() {
        let every = job
            .every
            .map(|e| format!("every {}", crate::jobs::manifest::describe_every(e)))
            .unwrap_or_else(|| "on demand".into());
        let due = if project.due(job, now) { "  DUE" } else { "" };
        println!("{:<28} {:<36} {every}{due}", job.id, job.title);
        match project.last_run(&job.id) {
            None => println!("{:<28} last: never", ""),
            Some(run) => println!(
                "{:<28} last: {} {} by {}",
                "",
                run.started_utc.format("%Y-%m-%d %H:%M"),
                run.status.label(),
                run.by.user
            ),
        }
    }
    Ok(())
}

pub fn cmd_jobs_history(id: String, limit: usize, json: bool, project: Option<String>) -> Result<()> {
    let root = jobs_root(project.as_deref())?;
    let project = jobs_project(&root)?;
    if project.job(&id).is_none() {
        bail!("no job `{id}` in {}", crate::jobs::MANIFEST);
    }
    let mut runs = crate::jobs::history::read(&root, &project.history_dir(), &id);
    runs.reverse();
    runs.truncate(limit);
    if json {
        println!("{}", serde_json::to_string_pretty(&runs)?);
        return Ok(());
    }
    if runs.is_empty() {
        println!("{id}: never run");
    }
    for run in &runs {
        println!("{}", describe_run(run));
    }
    if let Some(lessons) = project.lessons.get(&id) {
        println!("\n{}", lessons.trim_end());
    }
    Ok(())
}

pub fn cmd_jobs_run(id: String, project: Option<String>) -> Result<()> {
    use serde_json::json;
    let mut client = crate::control::Client::connect()?;
    // The TUI resolves the project by id or name; from a pane, the pane's own.
    let project = match project {
        Some(project) => project,
        None => std::env::var(comms::ENV_WORKSPACE).map_err(|_| {
            anyhow!("not inside a workbench pane: say which project with --project <name>")
        })?,
    };
    let reply = client.call("jobs.run", json!({"project": project, "job": id}))?;
    if reply.get("accepted").and_then(|v| v.as_bool()) == Some(true) {
        println!("asked workbench to run `{id}` in {project}");
    } else {
        println!("{reply}");
    }
    Ok(())
}

pub struct JobReport {
    pub status: String,
    pub summary: String,
    pub artifacts: Option<String>,
    pub lessons: Vec<String>,
    pub run: Option<String>,
    pub project: Option<String>,
}

pub fn cmd_jobs_report(report: JobReport) -> Result<()> {
    use crate::jobs::{history, RunStatus};
    let status = RunStatus::parse(&report.status)
        .filter(|s| !matches!(s, RunStatus::Running))
        .ok_or_else(|| anyhow!("--status must be completed, partial, blocked or failed"))?;
    if report.summary.trim().is_empty() {
        bail!("--summary must say what happened");
    }
    let run_id = report
        .run
        .or_else(|| std::env::var(crate::jobs::ENV_RUN).ok())
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| {
            anyhow!(
                "which run? pass --run <id>, or run this from the job's own pane where ${} is set",
                crate::jobs::ENV_RUN
            )
        })?;
    let root = jobs_root(report.project.as_deref())?;
    let project = jobs_project(&root)?;
    let history_dir = project.history_dir();
    // The job is whichever ledger holds the run; the environment is a hint.
    let job_id = std::env::var(crate::jobs::ENV_JOB)
        .ok()
        .filter(|id| history::find(&root, &history_dir, id, &run_id).is_some())
        .or_else(|| {
            project
                .jobs()
                .iter()
                .map(|job| job.id.clone())
                .find(|id| history::find(&root, &history_dir, id, &run_id).is_some())
        })
        .ok_or_else(|| anyhow!("no run `{run_id}` in any ledger under {}", history_dir.display()))?;
    let latest = history::find(&root, &history_dir, &job_id, &run_id).expect("found above");
    let mut closed = crate::jobs::close_record(&latest, status);
    closed.summary = Some(report.summary.trim().to_string());
    if let Some(artifacts) = report.artifacts.map(|a| a.trim().to_string()).filter(|a| !a.is_empty()) {
        closed.artifacts = Some(artifacts);
    }
    let lessons: Vec<String> = report
        .lessons
        .iter()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    closed.lessons.extend(lessons.iter().cloned());
    let path = history::append(&root, &history_dir, &closed)?;
    for lesson in &lessons {
        history::append_lesson(&root, &job_id, lesson)?;
    }
    println!(
        "recorded {} as {} in {}",
        run_id,
        status.label(),
        path.strip_prefix(&root).unwrap_or(&path).display()
    );
    if !lessons.is_empty() {
        println!(
            "added {} lesson(s) to {}",
            lessons.len(),
            crate::jobs::lessons_path(&root, &job_id)
                .strip_prefix(&root)
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        );
    }
    if latest.status != RunStatus::Running {
        println!(
            "note: the run was already {}; this report supersedes it",
            latest.status.label()
        );
    }
    Ok(())
}

pub fn cmd_jobs_init(path: Option<PathBuf>) -> Result<()> {
    let root = match path {
        Some(path) => path,
        None => std::env::current_dir()?,
    };
    let written = crate::jobs::init(&root)?;
    for path in &written {
        println!("wrote {}", path.strip_prefix(&root).unwrap_or(path).display());
    }
    println!(
        "next: edit {} (the example job is a placeholder), then open the project in workbench and press F4",
        crate::jobs::MANIFEST
    );
    Ok(())
}

#[cfg(test)]
mod jobs_tests {
    use super::*;
    use crate::jobs::{self, history, RunStatus};

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        jobs::init(dir.path()).unwrap();
        dir
    }

    fn running_line(root: &std::path::Path, run_id: &str) {
        let project = jobs::load(root).unwrap();
        let job = project.job("example-check").unwrap().clone();
        let record = jobs::start_record(root, &job, run_id, "claude", None);
        history::append(root, &project.history_dir(), &record).unwrap();
    }

    #[test]
    fn report_without_a_run_id_names_both_ways_to_give_one() {
        let dir = project();
        std::env::remove_var(jobs::ENV_RUN);
        let err = cmd_jobs_report(JobReport {
            status: "completed".into(),
            summary: "fine".into(),
            artifacts: None,
            lessons: vec![],
            run: None,
            project: Some(dir.path().display().to_string()),
        })
        .unwrap_err()
        .to_string();
        assert!(err.contains("--run") && err.contains(jobs::ENV_RUN), "{err}");
    }

    #[test]
    fn report_rejects_a_status_the_ledger_does_not_take_from_an_agent() {
        let dir = project();
        running_line(dir.path(), "r1");
        for status in ["running", "great", ""] {
            let err = cmd_jobs_report(JobReport {
                status: status.into(),
                summary: "fine".into(),
                artifacts: None,
                lessons: vec![],
                run: Some("r1".into()),
                project: Some(dir.path().display().to_string()),
            })
            .unwrap_err()
            .to_string();
            assert!(err.contains("--status"), "{status:?}: {err}");
        }
    }

    #[test]
    fn report_closes_the_run_and_files_the_lessons_and_history_reads_it_back() {
        let dir = project();
        let root = dir.path();
        running_line(root, "r1");
        cmd_jobs_report(JobReport {
            status: "partial".into(),
            summary: "  Two of three videos; the third timed out.  ".into(),
            artifacts: Some("ops/runs/r1".into()),
            lessons: vec!["Open comments twice".into(), " ".into()],
            run: Some("r1".into()),
            project: Some(root.display().to_string()),
        })
        .unwrap();
        let project = jobs::load(root).unwrap();
        let run = project.last_run("example-check").unwrap();
        assert_eq!(run.status, RunStatus::Partial);
        assert_eq!(run.summary.as_deref(), Some("Two of three videos; the third timed out."));
        assert_eq!(run.artifacts.as_deref(), Some("ops/runs/r1"));
        assert_eq!(run.lessons, vec!["Open comments twice".to_string()]);
        assert!(run.ended_utc.is_some());
        assert!(project.lessons["example-check"].contains("- 20"));
        assert!(project.lessons["example-check"].trim_end().ends_with("Open comments twice"));
        cmd_jobs_history("example-check".into(), 5, false, Some(root.display().to_string())).unwrap();
        cmd_jobs_list(Some(root.display().to_string()), true).unwrap();
        let err = cmd_jobs_history("nope".into(), 5, false, Some(root.display().to_string()))
            .unwrap_err()
            .to_string();
        assert!(err.contains("`nope`"), "{err}");
    }

    #[test]
    fn the_run_id_can_come_from_the_environment_and_names_the_job_itself() {
        let dir = project();
        let root = dir.path();
        running_line(root, "r-env");
        std::env::set_var(jobs::ENV_RUN, "r-env");
        std::env::set_var(jobs::ENV_JOB, "not-this-job");
        let result = cmd_jobs_report(JobReport {
            status: "done".into(),
            summary: "ok".into(),
            artifacts: None,
            lessons: vec![],
            run: None,
            project: Some(root.display().to_string()),
        });
        std::env::remove_var(jobs::ENV_RUN);
        std::env::remove_var(jobs::ENV_JOB);
        result.unwrap();
        let project = jobs::load(root).unwrap();
        assert_eq!(project.last_run("example-check").unwrap().status, RunStatus::Completed);
    }

    #[test]
    fn the_project_is_found_from_a_subdirectory_and_a_path_without_a_manifest_says_so() {
        let dir = project();
        let sub = dir.path().join("ops/deep");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(
            jobs_root(Some(sub.to_str().unwrap())).unwrap(),
            dir.path().canonicalize().unwrap()
        );
        let bare = tempfile::tempdir().unwrap();
        let err = jobs_root(Some(bare.path().to_str().unwrap())).unwrap_err().to_string();
        assert!(err.contains(jobs::MANIFEST), "{err}");
    }
}
