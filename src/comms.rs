//! Agent-to-agent communication layer.
//!
//! Everything is file-based, rooted in a per-workspace directory OUTSIDE the
//! repo (so nothing dirties git status or risks committing conversation
//! logs):
//!
//! ```text
//! <config_dir>/workbench/comms/<workspace-id>/
//!   agents.json          roster of sessions (refreshed by the TUI)
//!   transcripts/<provider>-<shortid>.md   exported on each idle transition
//!   inbox/<ticket>.json  messages written by the `workbench` CLI
//!   replies/<ticket>.json  consult outcomes written by the TUI
//! ```
//!
//! The TUI is the only writer of roster/transcripts/replies and the only
//! reader of the inbox; the CLI (run by agents inside their PTYs) does the
//! inverse. Self-identity travels via the `WORKBENCH_SESSION` /
//! `WORKBENCH_WORKSPACE` env vars injected at PTY spawn.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const ENV_SESSION: &str = "WORKBENCH_SESSION";
pub const ENV_WORKSPACE: &str = "WORKBENCH_WORKSPACE";

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

pub fn comms_root() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| anyhow!("could not locate config directory"))?
        .join("workbench")
        .join("comms");
    Ok(dir)
}

pub fn workspace_dir(workspace_id: &str) -> Result<PathBuf> {
    Ok(comms_root()?.join(workspace_id))
}

pub fn ensure_workspace_dirs(workspace_id: &str) -> Result<PathBuf> {
    let dir = workspace_dir(workspace_id)?;
    for sub in ["transcripts", "inbox", "replies"] {
        fs::create_dir_all(dir.join(sub))?;
    }
    Ok(dir)
}

/// Atomic write (temp file + rename) so readers never see partial content.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Roster
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RosterAgent {
    /// First 8 hex chars of the session uuid — the canonical address.
    pub id: String,
    pub provider: String,
    #[serde(default)]
    pub alias: Option<String>,
    /// Worktree branch if isolated, otherwise the workspace itself.
    pub branch: String,
    /// Directory the agent runs in (worktree path or workspace path).
    pub cwd: String,
    /// "idle" | "busy" | "stopped"
    pub status: String,
    /// Absolute path of the exported transcript, if this agent has one.
    #[serde(default)]
    pub transcript: Option<String>,
    /// Whether `workbench ask` can target this agent (transcript-capable).
    pub supports_consult: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Roster {
    pub workspace_id: String,
    pub workspace_name: String,
    pub workspace_path: String,
    pub updated_at: String,
    pub agents: Vec<RosterAgent>,
}

pub fn roster_path(workspace_id: &str) -> Result<PathBuf> {
    Ok(workspace_dir(workspace_id)?.join("agents.json"))
}

pub fn load_roster(workspace_id: &str) -> Result<Roster> {
    let path = roster_path(workspace_id)?;
    let bytes = fs::read(&path).with_context(|| {
        format!(
            "no roster at {} — is workbench running with this workspace open?",
            path.display()
        )
    })?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Locate the workspace whose path contains `cwd` (used when the CLI runs
/// outside a workbench PTY and `WORKBENCH_WORKSPACE` is unset). Picks the
/// longest matching workspace path so nested workspaces resolve correctly.
pub fn find_workspace_for_cwd(cwd: &Path) -> Result<String> {
    let root = comms_root()?;
    let mut best: Option<(usize, String)> = None;
    for entry in fs::read_dir(&root).with_context(|| format!("no comms data at {}", root.display()))? {
        let entry = entry?;
        let ws_id = entry.file_name().to_string_lossy().to_string();
        let Ok(roster) = load_roster(&ws_id) else {
            continue;
        };
        let ws_path = PathBuf::from(&roster.workspace_path);
        if cwd.starts_with(&ws_path) {
            let len = ws_path.as_os_str().len();
            if best.as_ref().map(|(l, _)| len > *l).unwrap_or(true) {
                best = Some((len, ws_id.clone()));
            }
        }
        // Worktree cwds live outside the workspace path — match agent cwds too.
        for agent in &roster.agents {
            if cwd.starts_with(Path::new(&agent.cwd)) {
                let len = agent.cwd.len();
                if best.as_ref().map(|(l, _)| len > *l).unwrap_or(true) {
                    best = Some((len, ws_id.clone()));
                }
            }
        }
    }
    best.map(|(_, id)| id).ok_or_else(|| {
        anyhow!("current directory is not inside any workbench workspace (set {ENV_WORKSPACE} or cd into one)")
    })
}

// ---------------------------------------------------------------------------
// Target resolution
// ---------------------------------------------------------------------------

/// Resolve a target query against the roster. Rules, in order:
/// exact short id → exact alias → provider name (only if exactly one running
/// match). Ambiguity or no match is an error whose message includes the
/// candidates, so a calling agent can pick deliberately. `self_id` (if known)
/// is excluded from provider matching and rejected as an explicit target.
pub fn resolve_target<'a>(
    roster: &'a Roster,
    query: &str,
    self_id: Option<&str>,
) -> Result<&'a RosterAgent, String> {
    let q = query.to_lowercase();
    let live: Vec<&RosterAgent> = roster
        .agents
        .iter()
        .filter(|a| a.status != "stopped")
        .collect();

    if let Some(agent) = live.iter().find(|a| a.id.to_lowercase() == q) {
        if Some(agent.id.as_str()) == self_id {
            return Err("target is yourself".to_string());
        }
        return Ok(agent);
    }

    if let Some(agent) = live
        .iter()
        .find(|a| a.alias.as_deref().map(str::to_lowercase) == Some(q.clone()))
    {
        if Some(agent.id.as_str()) == self_id {
            return Err("target is yourself".to_string());
        }
        return Ok(agent);
    }

    let by_provider: Vec<&&RosterAgent> = live
        .iter()
        .filter(|a| a.provider.to_lowercase() == q && Some(a.id.as_str()) != self_id)
        .collect();

    match by_provider.len() {
        1 => Ok(by_provider[0]),
        0 => Err(format!(
            "no agent matches '{query}'. Running agents:\n{}",
            describe_agents(&live)
        )),
        _ => Err(format!(
            "'{query}' is ambiguous — address by id or alias:\n{}",
            describe_agents(&by_provider.iter().map(|a| **a).collect::<Vec<_>>())
        )),
    }
}

pub fn describe_agents(agents: &[&RosterAgent]) -> String {
    agents
        .iter()
        .map(|a| {
            format!(
                "  {}  {}{}  [{}]  {}",
                a.id,
                a.provider,
                a.alias
                    .as_deref()
                    .map(|al| format!(" ({al})"))
                    .unwrap_or_default(),
                a.branch,
                a.status,
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Inbox messages + replies
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InboxMessage {
    /// A consult: deliver `message` to session `to` when idle; capture reply.
    Ask {
        ticket: String,
        from: String,
        to: String,
        message: String,
        created_at: String,
    },
    /// Self-service alias assignment from `workbench alias`.
    Alias {
        ticket: String,
        from: String,
        alias: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reply {
    pub ticket: String,
    /// "answered" | "refused" | "timeout" | "error"
    pub status: String,
    pub from: String,
    pub to: String,
    pub question: String,
    #[serde(default)]
    pub reply: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

pub fn new_ticket() -> String {
    use rand::Rng;
    let n: u32 = rand::thread_rng().gen_range(0x1000..0xFFFF_FFFF);
    format!("c{n:x}")
}

pub fn write_inbox(workspace_id: &str, msg: &InboxMessage) -> Result<PathBuf> {
    let ticket = match msg {
        InboxMessage::Ask { ticket, .. } => ticket,
        InboxMessage::Alias { ticket, .. } => ticket,
    };
    let dir = ensure_workspace_dirs(workspace_id)?;
    let path = dir.join("inbox").join(format!("{ticket}.json"));
    write_atomic(&path, serde_json::to_string_pretty(msg)?.as_bytes())?;
    Ok(path)
}

pub fn reply_path(workspace_id: &str, ticket: &str) -> Result<PathBuf> {
    Ok(workspace_dir(workspace_id)?
        .join("replies")
        .join(format!("{ticket}.json")))
}

pub fn write_reply(workspace_id: &str, reply: &Reply) -> Result<()> {
    let dir = ensure_workspace_dirs(workspace_id)?;
    let path = dir.join("replies").join(format!("{}.json", reply.ticket));
    write_atomic(&path, serde_json::to_string_pretty(reply)?.as_bytes())
}

// ---------------------------------------------------------------------------
// Transcript helpers
// ---------------------------------------------------------------------------

pub fn transcript_path(workspace_id: &str, provider: &str, short_id: &str) -> Result<PathBuf> {
    let name = format!("{}-{}.md", provider.to_lowercase().replace(' ', "-"), short_id);
    Ok(workspace_dir(workspace_id)?.join("transcripts").join(name))
}

/// Last `n` lines of `text` (whole text if it has fewer).
pub fn tail_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

// ---------------------------------------------------------------------------
// Standing instructions block
// ---------------------------------------------------------------------------

const BLOCK_BEGIN: &str = "<!-- workbench:begin -->";
const BLOCK_END: &str = "<!-- workbench:end -->";

pub fn instructions_block() -> String {
    format!(
        r#"{BLOCK_BEGIN}
## Workbench multi-agent workspace

You are running inside the workbench TUI, possibly alongside other coding
agents (Claude, Codex, ...). Your session id is in `$WORKBENCH_SESSION`.
The `workbench` CLI lets you discover and communicate with peers:

- `workbench agents` — list agent sessions here (id, provider, alias, branch, idle/busy)
- `workbench transcript <id|alias> --lines 200` — read a peer's recent conversation (exported each time it goes idle)
- `workbench ask <id|alias> "question" --wait` — deliver a question to a live peer and collect its answer (or collect later: `workbench replies <ticket> --wait`)
- `workbench handoff <id|alias> --wait` — ask a peer for a structured summary of its work (done/remaining/decisions/gotchas) before taking over or building on it
- `workbench alias <name>` — set your own alias

Address peers by id or alias; a provider name like `codex` only works when
exactly one such agent is running. A consult costs the peer a full model
turn — consult when the user asks you to or you are genuinely blocked, not
by default.

### Choosing the right collaboration pattern

Match the user's request to what is known to work; if it fits, just do it:
- REVIEWING a peer's work: review its git branch/diff yourself with fresh
  eyes (branches are in `agents` output). Do NOT ask the author to summarize
  or defend its own work first — self-reports hide exactly the bugs the
  author missed.
- TAKING OVER or building on a peer's work: `workbench handoff` the live
  author (its self-summary carries decisions a transcript reader must
  guess); read `transcript` only when the author is stopped or unresponsive.
- CONSULTING: one broad question beats many narrow ones; a cross-provider
  opinion (claude<->codex) is worth more than a same-provider one.

If the user asks for something known to be counterproductive — e.g. agents
debating in rounds until they agree (models sycophantically converge on
wrong answers), several agents editing the same branch/files (conflicting
writes), or relaying work through a peer that you could do directly (token
cost without benefit) — do not silently comply: briefly tell the user why it
tends to fail, propose the closest effective alternative, and proceed with
the original request only if they confirm.
{BLOCK_END}"#
    )
}

/// Insert or refresh the fenced workbench block in `path`. Creates the file
/// if missing and returns true if it was newly created.
pub fn upsert_instructions_file(path: &Path, block: &str) -> Result<bool> {
    let existing = fs::read_to_string(path).ok();
    let created = existing.is_none();
    let updated = match existing {
        Some(content) => {
            if let (Some(start), Some(end)) = (content.find(BLOCK_BEGIN), content.find(BLOCK_END)) {
                let end = end + BLOCK_END.len();
                if &content[start..end] == block {
                    return Ok(false); // already current
                }
                format!("{}{}{}", &content[..start], block, &content[end..])
            } else {
                let sep = if content.ends_with('\n') { "\n" } else { "\n\n" };
                format!("{content}{sep}{block}\n")
            }
        }
        None => format!("{block}\n"),
    };
    write_atomic(path, updated.as_bytes())?;
    Ok(created)
}

/// Make sure a workspace's instruction files carry the workbench block.
/// Files we create are added to `.git/info/exclude` so they never show up
/// as untracked noise; pre-existing (possibly tracked) files just get the
/// fenced section refreshed.
pub fn ensure_workspace_instructions(workspace_path: &Path) -> Result<()> {
    let block = instructions_block();
    for name in ["CLAUDE.local.md", "AGENTS.md"] {
        let path = workspace_path.join(name);
        let existed = path.exists();
        upsert_instructions_file(&path, &block)?;
        if !existed {
            add_git_exclude(workspace_path, name);
        }
    }
    Ok(())
}

fn add_git_exclude(workspace_path: &Path, name: &str) {
    let exclude = workspace_path.join(".git").join("info").join("exclude");
    let Some(parent) = exclude.parent() else {
        return;
    };
    if !workspace_path.join(".git").exists() {
        return;
    }
    let _ = fs::create_dir_all(parent);
    let current = fs::read_to_string(&exclude).unwrap_or_default();
    if current.lines().any(|l| l.trim() == name) {
        return;
    }
    let sep = if current.is_empty() || current.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let _ = fs::write(&exclude, format!("{current}{sep}{name}\n"));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str, provider: &str, alias: Option<&str>, status: &str) -> RosterAgent {
        RosterAgent {
            id: id.into(),
            provider: provider.into(),
            alias: alias.map(Into::into),
            branch: "main".into(),
            cwd: "/tmp/w".into(),
            status: status.into(),
            transcript: None,
            supports_consult: true,
        }
    }

    fn roster(agents: Vec<RosterAgent>) -> Roster {
        Roster {
            workspace_id: "ws".into(),
            workspace_name: "w".into(),
            workspace_path: "/tmp/w".into(),
            updated_at: "now".into(),
            agents,
        }
    }

    #[test]
    fn resolves_by_short_id_alias_and_unique_provider() {
        let r = roster(vec![
            agent("ab12cd34", "claude", None, "idle"),
            agent("ef56ab78", "codex", Some("parser"), "busy"),
        ]);

        assert_eq!(resolve_target(&r, "ef56ab78", None).unwrap().id, "ef56ab78");
        assert_eq!(resolve_target(&r, "parser", None).unwrap().id, "ef56ab78");
        assert_eq!(resolve_target(&r, "codex", None).unwrap().id, "ef56ab78");
        assert_eq!(resolve_target(&r, "Claude", None).unwrap().id, "ab12cd34");
    }

    #[test]
    fn ambiguous_provider_errors_with_candidates() {
        let r = roster(vec![
            agent("aaaa1111", "codex", None, "idle"),
            agent("bbbb2222", "codex", Some("reviewer"), "busy"),
        ]);
        let err = resolve_target(&r, "codex", None).unwrap_err();
        assert!(err.contains("ambiguous"));
        assert!(err.contains("aaaa1111") && err.contains("bbbb2222"));
        // Alias still resolves despite provider ambiguity.
        assert_eq!(resolve_target(&r, "reviewer", None).unwrap().id, "bbbb2222");
    }

    #[test]
    fn provider_match_excludes_self_and_stopped() {
        let r = roster(vec![
            agent("aaaa1111", "claude", None, "idle"),
            agent("bbbb2222", "claude", None, "busy"),
            agent("cccc3333", "codex", None, "stopped"),
        ]);
        // With self excluded, "claude" becomes unambiguous.
        assert_eq!(
            resolve_target(&r, "claude", Some("aaaa1111")).unwrap().id,
            "bbbb2222"
        );
        // Stopped agents never match.
        assert!(resolve_target(&r, "codex", None).is_err());
        // Explicitly addressing yourself is rejected.
        assert!(resolve_target(&r, "aaaa1111", Some("aaaa1111")).is_err());
    }

    #[test]
    fn tail_lines_slices_from_the_end() {
        let text = "a\nb\nc\nd";
        assert_eq!(tail_lines(text, 2), "c\nd");
        assert_eq!(tail_lines(text, 10), text);
    }

    #[test]
    fn upsert_instructions_creates_replaces_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CLAUDE.local.md");

        let created = upsert_instructions_file(&path, &instructions_block()).unwrap();
        assert!(created);
        let first = std::fs::read_to_string(&path).unwrap();
        assert!(first.contains("workbench agents"));

        // Idempotent: unchanged block writes nothing new.
        let created = upsert_instructions_file(&path, &instructions_block()).unwrap();
        assert!(!created);

        // User content around the block survives a block update.
        std::fs::write(&path, format!("# mine\n\n{}\ntrailing\n", instructions_block())).unwrap();
        upsert_instructions_file(&path, "<!-- workbench:begin -->\nnew\n<!-- workbench:end -->")
            .unwrap();
        let updated = std::fs::read_to_string(&path).unwrap();
        assert!(updated.starts_with("# mine"));
        assert!(updated.contains("\nnew\n"));
        assert!(updated.contains("trailing"));
        assert!(!updated.contains("workbench agents"));
    }
}
