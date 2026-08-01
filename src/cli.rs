//! `workbench` CLI verbs used by agents (and humans) for agent-to-agent
//! comms. These are pure file readers/writers against the comms directory —
//! the running TUI does the live work (see `app::comms_tick`).

use crate::comms::{self, InboxMessage, Reply};
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

pub fn cmd_agents() -> Result<()> {
    let ctx = caller_ctx()?;
    let roster = comms::load_roster(&ctx.workspace_id)?;
    println!(
        "workspace: {} ({})  updated: {}",
        roster.workspace_name, roster.workspace_path, roster.updated_at
    );
    if roster.agents.is_empty() {
        println!("no agent sessions");
        return Ok(());
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
    println!("\naddress a peer by id or alias (provider name works when unique):");
    println!("  workbench transcript <id> --lines 200");
    println!("  workbench ask <id> \"question\"");
    Ok(())
}

pub fn cmd_transcript(target: String, lines: usize, all: bool) -> Result<()> {
    let ctx = caller_ctx()?;
    let roster = comms::load_roster(&ctx.workspace_id)?;
    let agent = comms::resolve_target(&roster, &target, ctx.session.as_deref())
        .map_err(|e| anyhow!(e))?;
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
    let roster = comms::load_roster(&ctx.workspace_id)?;
    let agent =
        comms::resolve_target(&roster, &target, Some(from.as_str())).map_err(|e| anyhow!(e))?;
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
        message,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    comms::write_inbox(&ctx.workspace_id, &msg)?;
    println!(
        "queued consult {ticket} for {} ({}) — delivered when it is idle",
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
