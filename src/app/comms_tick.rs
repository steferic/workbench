//! TUI-side driver for agent-to-agent comms (see `crate::comms`).
//!
//! Runs from the Tick handler. Responsibilities:
//! - export transcripts + refresh each workspace's `agents.json` roster
//! - ensure the standing instructions block in workspace instruction files
//! - poll the inbox for `workbench ask` / `workbench alias` messages
//! - deliver queued consults to their target session when it is idle
//! - capture the target's reply (transcript delta at its next idle) and
//!   write it to the replies directory
//!
//! All file writes go through atomic temp+rename; the larger ones are
//! offloaded to blocking threads.

use crate::app::{AppState, Toast, ToastLevel};
use crate::comms::{self, InboxMessage, Reply, Roster, RosterAgent};
use crate::models::SessionStatus;
use std::collections::HashSet;
use std::time::{Duration, Instant};
use uuid::Uuid;

const INBOX_POLL_INTERVAL: Duration = Duration::from_millis(1000);
const ROSTER_REFRESH_INTERVAL: Duration = Duration::from_millis(2000);
const CONSULT_TTL: Duration = Duration::from_secs(15 * 60);

/// A consult accepted from the inbox, waiting for delivery and then a reply.
#[derive(Debug)]
pub struct PendingConsult {
    pub ticket: String,
    pub workspace_id: Uuid,
    pub from_short: String,
    pub to_session: Uuid,
    pub to_short: String,
    pub question: String,
    pub delivered: bool,
    /// Transcript length of the target at delivery; the reply is everything
    /// after this once the target goes idle again.
    pub transcript_base: usize,
    /// The target's `last_activity` at delivery. The idle queue re-admits a
    /// session whose activity clock is simply stale, so without this a
    /// slow-to-start target would be "idle" again before it ever produced a
    /// reply and we'd capture nothing.
    pub activity_at_delivery: Option<Instant>,
    pub delivered_at: Option<Instant>,
    pub created: Instant,
}

/// If the target never produces attributable output, give up waiting for
/// activity and capture whatever the transcript holds.
const CAPTURE_FALLBACK: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub struct CommsState {
    pub last_inbox_poll: Instant,
    pub last_roster_refresh: Instant,
    /// Serialized roster per workspace, to skip no-op writes.
    pub roster_cache: std::collections::HashMap<Uuid, String>,
    pub pending: Vec<PendingConsult>,
    /// Workspaces whose instruction files were ensured this run.
    pub instructions_done: HashSet<Uuid>,
}

impl CommsState {
    pub fn new() -> Self {
        Self {
            last_inbox_poll: Instant::now(),
            last_roster_refresh: Instant::now(),
            roster_cache: std::collections::HashMap::new(),
            pending: Vec::new(),
            instructions_done: HashSet::new(),
        }
    }
}

fn toast(state: &mut AppState, msg: String, level: ToastLevel) {
    state
        .ui
        .toasts
        .push_back(Toast::new(msg, level, Duration::from_secs(4)));
    while state.ui.toasts.len() > 5 {
        state.ui.toasts.pop_front();
    }
}

/// Main entry point, called once per Tick with the sessions that just went
/// idle this tick.
pub fn tick(
    state: &mut AppState,
    action_tx: &tokio::sync::mpsc::UnboundedSender<crate::app::Action>,
    newly_idle: &[Uuid],
) {
    export_transcripts_for(state, newly_idle);
    capture_replies(state, newly_idle);
    deliver_pending(state, action_tx);
    poll_inbox(state);
    refresh_rosters(state);
    expire_stale(state);
}

// ---------------------------------------------------------------------------
// Transcript + roster export
// ---------------------------------------------------------------------------

fn export_transcripts_for(state: &mut AppState, session_ids: &[Uuid]) {
    for &sid in session_ids {
        export_transcript(state, sid);
    }
}

fn export_transcript(state: &AppState, session_id: Uuid) {
    let Some(session) = state.get_session(session_id) else {
        return;
    };
    let Some(transcript) = state.system.transcript_buffers.get(&session_id) else {
        return;
    };
    let provider = session.agent_type.display_name().to_lowercase();
    let Ok(path) = comms::transcript_path(
        &session.workspace_id.to_string(),
        &provider,
        &session.short_id(),
    ) else {
        return;
    };

    let mut text = String::with_capacity(transcript.len() * 64);
    text.push_str(&format!(
        "# {} {} — workbench transcript (exported at last idle; the session may have progressed)\n\n",
        provider,
        session.short_id()
    ));
    for i in 0..transcript.len() {
        if let Some(line) = transcript.line(i) {
            text.push_str(line);
        }
        text.push('\n');
    }

    tokio::task::spawn_blocking(move || {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(err) = comms::write_atomic(&path, text.as_bytes()) {
            crate::logger::warn(format!("failed to export transcript: {err}"));
        }
    });
}

fn refresh_rosters(state: &mut AppState) {
    if state.system.comms.last_roster_refresh.elapsed() < ROSTER_REFRESH_INTERVAL {
        return;
    }
    state.system.comms.last_roster_refresh = Instant::now();

    let workspaces: Vec<(Uuid, String, std::path::PathBuf)> = state
        .data
        .workspaces
        .iter()
        .map(|w| (w.id, w.name.clone(), w.path.clone()))
        .collect();

    for (ws_id, ws_name, ws_path) in workspaces {
        let mut agents: Vec<RosterAgent> = Vec::new();
        if let Some(sessions) = state.data.sessions.get(&ws_id) {
            for s in sessions {
                if !s.agent_type.is_agent() {
                    continue;
                }
                let provider = s.agent_type.display_name().to_lowercase();
                let status = match s.status {
                    SessionStatus::Running => {
                        if state.data.idle_queue.contains(&s.id) {
                            "idle"
                        } else {
                            "busy"
                        }
                    }
                    _ => "stopped",
                };
                let cwd = s
                    .worktree_path
                    .clone()
                    .unwrap_or_else(|| ws_path.clone())
                    .to_string_lossy()
                    .to_string();
                let transcript = comms::transcript_path(&ws_id.to_string(), &provider, &s.short_id())
                    .ok()
                    .filter(|p| p.exists() || s.agent_type.is_redraw_style())
                    .map(|p| p.to_string_lossy().to_string());
                agents.push(RosterAgent {
                    id: s.short_id(),
                    provider,
                    alias: s.alias.clone(),
                    branch: s
                        .worktree_branch
                        .clone()
                        .unwrap_or_else(|| "workspace".to_string()),
                    cwd,
                    status: status.to_string(),
                    transcript,
                    supports_consult: s.agent_type.is_redraw_style(),
                });
            }
        }

        let roster = Roster {
            workspace_id: ws_id.to_string(),
            workspace_name: ws_name,
            workspace_path: ws_path.to_string_lossy().to_string(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            agents,
        };
        // Compare everything except the timestamp so unchanged rosters skip IO.
        let fingerprint = serde_json::to_string(&roster.agents).unwrap_or_default();
        if state.system.comms.roster_cache.get(&ws_id) == Some(&fingerprint) {
            continue;
        }
        state.system.comms.roster_cache.insert(ws_id, fingerprint);

        let ensure_instructions = state.system.comms.instructions_done.insert(ws_id)
            && !roster.agents.is_empty();
        let ws_id_str = ws_id.to_string();
        tokio::task::spawn_blocking(move || {
            match comms::ensure_workspace_dirs(&ws_id_str) {
                Ok(_) => {
                    if let (Ok(path), Ok(json)) = (
                        comms::roster_path(&ws_id_str),
                        serde_json::to_string_pretty(&roster),
                    ) {
                        if let Err(err) = comms::write_atomic(&path, json.as_bytes()) {
                            crate::logger::warn(format!("failed to write roster: {err}"));
                        }
                    }
                }
                Err(err) => crate::logger::warn(format!("failed to create comms dirs: {err}")),
            }
            if ensure_instructions {
                if let Err(err) = comms::ensure_workspace_instructions(&ws_path) {
                    crate::logger::warn(format!("failed to write workspace instructions: {err}"));
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Inbox
// ---------------------------------------------------------------------------

fn poll_inbox(state: &mut AppState) {
    if state.system.comms.last_inbox_poll.elapsed() < INBOX_POLL_INTERVAL {
        return;
    }
    state.system.comms.last_inbox_poll = Instant::now();

    let workspace_ids: Vec<Uuid> = state.data.workspaces.iter().map(|w| w.id).collect();
    for ws_id in workspace_ids {
        let Ok(dir) = comms::workspace_dir(&ws_id.to_string()) else {
            continue;
        };
        let inbox = dir.join("inbox");
        let Ok(entries) = std::fs::read_dir(&inbox) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let msg: Option<InboxMessage> = std::fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok());
            let _ = std::fs::remove_file(&path);
            match msg {
                Some(InboxMessage::Ask {
                    ticket,
                    from,
                    to,
                    message,
                    ..
                }) => ingest_ask(state, ws_id, ticket, from, to, message),
                Some(InboxMessage::Alias { ticket, from, alias }) => {
                    ingest_alias(state, ws_id, ticket, from, alias)
                }
                None => {
                    crate::logger::warn(format!(
                        "unreadable comms message {} — removed",
                        path.display()
                    ));
                }
            }
        }
    }
}

fn find_session_by_short(state: &AppState, ws_id: Uuid, short: &str) -> Option<Uuid> {
    state.data.sessions.get(&ws_id).and_then(|sessions| {
        sessions
            .iter()
            .find(|s| s.short_id().eq_ignore_ascii_case(short))
            .map(|s| s.id)
    })
}

fn refuse(state: &AppState, ws_id: Uuid, ticket: &str, from: &str, to: &str, q: &str, reason: String) {
    let reply = Reply {
        ticket: ticket.to_string(),
        status: "refused".to_string(),
        from: from.to_string(),
        to: to.to_string(),
        question: q.to_string(),
        reply: None,
        reason: Some(reason),
    };
    let _ = state; // reads nothing else; kept for signature symmetry
    if let Err(err) = comms::write_reply(&ws_id.to_string(), &reply) {
        crate::logger::warn(format!("failed to write refusal: {err}"));
    }
}

fn ingest_ask(
    state: &mut AppState,
    ws_id: Uuid,
    ticket: String,
    from: String,
    to: String,
    message: String,
) {
    let Some(target) = find_session_by_short(state, ws_id, &to) else {
        refuse(state, ws_id, &ticket, &from, &to, &message, format!("no session {to} in this workspace"));
        return;
    };
    let (running, consultable) = state
        .get_session(target)
        .map(|s| {
            (
                s.status == SessionStatus::Running,
                s.agent_type.is_redraw_style(),
            )
        })
        .unwrap_or((false, false));
    if !running {
        refuse(state, ws_id, &ticket, &from, &to, &message, format!("session {to} is not running"));
        return;
    }
    if !consultable {
        refuse(
            state, ws_id, &ticket, &from, &to, &message,
            format!("session {to} does not support consults (no transcript); read its terminal or files instead"),
        );
        return;
    }
    // Cycle guard: refuse if the target is itself waiting on a consult it
    // sent to the asker (A→B while B→A would deadlock on idle-gating).
    let cycle = state.system.comms.pending.iter().any(|p| {
        p.workspace_id == ws_id && p.from_short.eq_ignore_ascii_case(&to) && p.to_short.eq_ignore_ascii_case(&from)
    });
    if cycle {
        refuse(
            state, ws_id, &ticket, &from, &to, &message,
            format!("consult cycle: {to} is already waiting on a consult to {from}; answer it first"),
        );
        return;
    }
    // One outstanding consult per asker keeps amplification bounded.
    if state
        .system
        .comms
        .pending
        .iter()
        .any(|p| p.workspace_id == ws_id && p.from_short.eq_ignore_ascii_case(&from))
    {
        refuse(
            state, ws_id, &ticket, &from, &to, &message,
            "you already have an outstanding consult; collect its reply first".to_string(),
        );
        return;
    }

    toast(
        state,
        format!("Consult {ticket}: {from} → {to} queued"),
        ToastLevel::Info,
    );
    state.system.comms.pending.push(PendingConsult {
        ticket,
        workspace_id: ws_id,
        from_short: from,
        to_session: target,
        to_short: to,
        question: message,
        delivered: false,
        transcript_base: 0,
        activity_at_delivery: None,
        delivered_at: None,
        created: Instant::now(),
    });
}

fn ingest_alias(state: &mut AppState, ws_id: Uuid, ticket: String, from: String, alias: String) {
    let ok_chars = alias
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    let valid = !alias.is_empty() && alias.len() <= 24 && ok_chars;
    let taken = state
        .data
        .sessions
        .get(&ws_id)
        .map(|ss| {
            ss.iter().any(|s| {
                s.alias.as_deref().map(str::to_lowercase) == Some(alias.to_lowercase())
                    && !s.short_id().eq_ignore_ascii_case(&from)
            })
        })
        .unwrap_or(false);
    let Some(session_id) = find_session_by_short(state, ws_id, &from) else {
        refuse(state, ws_id, &ticket, &from, &from, "", "unknown session".into());
        return;
    };

    if !valid || taken {
        let reason = if taken {
            format!("alias '{alias}' is already taken in this workspace")
        } else {
            "alias must be 1-24 chars of [a-zA-Z0-9_-]".to_string()
        };
        refuse(state, ws_id, &ticket, &from, &from, "", reason);
        return;
    }

    if let Some(session) = state.get_session_mut(session_id) {
        session.alias = Some(alias.clone());
    }
    crate::app::handlers::save_state(state, "failed to save alias");
    // Push the roster out immediately so `workbench ask <alias>` works right
    // after `workbench alias` returns instead of racing the 2s refresh.
    state.system.comms.roster_cache.remove(&ws_id);
    state.system.comms.last_roster_refresh = Instant::now() - ROSTER_REFRESH_INTERVAL;
    toast(state, format!("{from} is now “{alias}”"), ToastLevel::Info);
    let reply = Reply {
        ticket: ticket.clone(),
        status: "answered".into(),
        from: from.clone(),
        to: from,
        question: String::new(),
        reply: Some(format!("alias set to {alias}")),
        reason: None,
    };
    if let Err(err) = comms::write_reply(&ws_id.to_string(), &reply) {
        crate::logger::warn(format!("failed to write alias reply: {err}"));
    }
}

// ---------------------------------------------------------------------------
// Delivery + reply capture
// ---------------------------------------------------------------------------

fn deliver_pending(
    state: &mut AppState,
    action_tx: &tokio::sync::mpsc::UnboundedSender<crate::app::Action>,
) {
    let deliverable: Vec<usize> = state
        .system
        .comms
        .pending
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.delivered && state.data.idle_queue.contains(&p.to_session))
        .map(|(i, _)| i)
        .collect();

    for idx in deliverable {
        let (target, base, framed, ticket, from, to_short) = {
            let p = &state.system.comms.pending[idx];
            let base = state
                .system
                .transcript_buffers
                .get(&p.to_session)
                .map(|t| t.len())
                .unwrap_or(0);
            let framed = format!(
                "[workbench consult {} from agent {}] {}\n(Reply normally — your full response will be relayed back to {} when you finish. Do not use `workbench ask` to answer this.)",
                p.ticket, p.from_short, p.question, p.from_short
            );
            (
                p.to_session,
                base,
                framed,
                p.ticket.clone(),
                p.from_short.clone(),
                p.to_short.clone(),
            )
        };

        // Bracketed paste so multiline questions don't submit early.
        let mut payload = Vec::with_capacity(framed.len() + 12);
        payload.extend_from_slice(b"\x1b[200~");
        payload.extend_from_slice(framed.as_bytes());
        payload.extend_from_slice(b"\x1b[201~");
        let _ = action_tx.send(crate::app::Action::SendInput(target, payload));
        let _ = action_tx.send(crate::app::Action::SendInput(target, vec![b'\r']));

        // Sending input marks activity via the echo path only after output
        // arrives; drop the target from the idle queue now so a second
        // pending consult can't double-deliver this tick.
        state.data.idle_queue.retain(|&id| id != target);

        let activity = state.data.last_activity.get(&target).copied();
        let p = &mut state.system.comms.pending[idx];
        p.delivered = true;
        p.transcript_base = base;
        p.activity_at_delivery = activity;
        p.delivered_at = Some(Instant::now());
        toast(
            state,
            format!("Consult {ticket}: {from} → {to_short} delivered"),
            ToastLevel::Info,
        );
    }
}

fn capture_replies(state: &mut AppState, newly_idle: &[Uuid]) {
    let mut finished: Vec<usize> = Vec::new();
    for (i, p) in state.system.comms.pending.iter().enumerate() {
        if !p.delivered {
            continue;
        }
        let idle_now =
            newly_idle.contains(&p.to_session) || state.data.idle_queue.contains(&p.to_session);
        if !idle_now {
            continue;
        }
        let worked_since_delivery =
            state.data.last_activity.get(&p.to_session).copied() != p.activity_at_delivery;
        let fallback_elapsed = p
            .delivered_at
            .map(|t| t.elapsed() > CAPTURE_FALLBACK)
            .unwrap_or(true);
        if worked_since_delivery || fallback_elapsed {
            finished.push(i);
        }
    }

    for &i in finished.iter().rev() {
        let p = state.system.comms.pending.remove(i);
        let reply_text = state
            .system
            .transcript_buffers
            .get(&p.to_session)
            .map(|t| {
                let start = p.transcript_base.min(t.len());
                let mut out = String::new();
                for idx in start..t.len() {
                    if let Some(line) = t.line(idx) {
                        out.push_str(line);
                    }
                    out.push('\n');
                }
                out.trim().to_string()
            })
            .unwrap_or_default();

        let reply = Reply {
            ticket: p.ticket.clone(),
            status: "answered".into(),
            from: p.from_short.clone(),
            to: p.to_short.clone(),
            question: p.question.clone(),
            reply: Some(reply_text),
            reason: None,
        };
        let ws = p.workspace_id.to_string();
        tokio::task::spawn_blocking(move || {
            if let Err(err) = comms::write_reply(&ws, &reply) {
                crate::logger::warn(format!("failed to write consult reply: {err}"));
            }
        });
        toast(
            state,
            format!("Consult {}: {} answered", p.ticket, p.to_short),
            ToastLevel::Success,
        );
    }
}

fn expire_stale(state: &mut AppState) {
    let expired: Vec<PendingConsult> = {
        let pending = &mut state.system.comms.pending;
        let mut out = Vec::new();
        let mut i = 0;
        while i < pending.len() {
            if pending[i].created.elapsed() > CONSULT_TTL {
                out.push(pending.remove(i));
            } else {
                i += 1;
            }
        }
        out
    };
    for p in expired {
        let reply = Reply {
            ticket: p.ticket.clone(),
            status: "timeout".into(),
            from: p.from_short,
            to: p.to_short,
            question: p.question,
            reply: None,
            reason: Some("consult expired before the target went idle".into()),
        };
        if let Err(err) = comms::write_reply(&p.workspace_id.to_string(), &reply) {
            crate::logger::warn(format!("failed to write timeout reply: {err}"));
        }
    }
}
