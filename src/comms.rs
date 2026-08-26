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

use crate::resolve::{Candidate, Scope};
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

/// Mark as stopped every agent in a roster whose workspace is not open.
///
/// A roster is only refreshed while the TUI holds its workspace open, so one
/// left by a workspace since removed — or by a previous run with a different
/// set open — goes on claiming its agents are idle forever. That was
/// invisible while each agent read only its own workspace's roster; reading
/// all of them turns it into a directory of peers that cannot be reached.
///
/// Retiring rather than deleting, because the directory beside the roster
/// holds exported transcripts a peer may still want to read, and because a
/// workspace reopened later just has its roster rewritten from live state.
///
/// Takes its root as an argument purely so it can be tested against a
/// temporary one. Returns the number of rosters changed.
pub fn retire_closed_rosters(root: &Path, open: &std::collections::HashSet<String>) -> usize {
    let Ok(entries) = fs::read_dir(root) else {
        return 0;
    };
    let mut retired = 0;
    for entry in entries.flatten() {
        let workspace_id = entry.file_name().to_string_lossy().to_string();
        if open.contains(&workspace_id) {
            continue;
        }
        let path = entry.path().join("agents.json");
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        let Ok(mut roster) = serde_json::from_slice::<Roster>(&bytes) else {
            continue;
        };
        if roster.agents.iter().all(|agent| agent.status == "stopped") {
            continue;
        }
        for agent in &mut roster.agents {
            agent.status = "stopped".to_string();
        }
        let Ok(json) = serde_json::to_string_pretty(&roster) else {
            continue;
        };
        if let Err(err) = write_atomic(&path, json.as_bytes()) {
            crate::logger::warn(format!("failed to retire roster: {err}"));
            continue;
        }
        retired += 1;
    }
    retired
}

// ---------------------------------------------------------------------------
// The directory: every agent on this machine
// ---------------------------------------------------------------------------

/// One agent plus the workspace it belongs to.
///
/// The roster is per-workspace because that is how it is written, but an
/// address is not: a short id is 8 hex chars of a uuid and means one session
/// anywhere on the machine. Flattening the rosters is what lets a peer in
/// another project be addressed at all.
#[derive(Debug, Clone)]
pub struct DirectoryEntry {
    pub workspace_id: String,
    pub workspace_name: String,
    pub agent: RosterAgent,
}

impl DirectoryEntry {
    fn candidate(&self) -> Candidate<'_> {
        Candidate {
            id: &self.agent.id,
            alias: self.agent.alias.as_deref(),
            provider: &self.agent.provider,
            project_id: &self.workspace_id,
            project: &self.workspace_name,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Directory {
    pub entries: Vec<DirectoryEntry>,
}

impl Directory {
    /// Read every workspace roster under the comms root.
    ///
    /// An unreadable or half-written roster is skipped rather than fatal. One
    /// stale directory left by a workspace nobody has opened in months must
    /// not make every other agent on the machine unaddressable.
    pub fn load() -> Result<Self> {
        let root = comms_root()?;
        let mut entries = Vec::new();
        let dir = fs::read_dir(&root)
            .with_context(|| format!("no comms data at {}", root.display()))?;
        for workspace in dir.flatten() {
            let workspace_id = workspace.file_name().to_string_lossy().to_string();
            let Ok(roster) = load_roster(&workspace_id) else {
                continue;
            };
            for agent in roster.agents {
                entries.push(DirectoryEntry {
                    workspace_id: roster.workspace_id.clone(),
                    workspace_name: roster.workspace_name.clone(),
                    agent,
                });
            }
        }
        // Sorted so every listing and every error message that names
        // candidates comes out in the same order; `read_dir` gives none.
        entries.sort_by(|a, b| {
            a.workspace_name
                .cmp(&b.workspace_name)
                .then_with(|| a.agent.id.cmp(&b.agent.id))
        });
        Ok(Self { entries })
    }

    /// Live agents anywhere else, grouped by project (entries are already
    /// sorted, so equal project names are adjacent).
    pub fn outside<'a>(&'a self, workspace_id: &str) -> Vec<&'a DirectoryEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.workspace_id != workspace_id && entry.agent.status != "stopped")
            .collect()
    }

    /// Resolve an address to one agent, anywhere on the machine.
    ///
    /// Stopped sessions are dropped before the ladder runs: their roster row
    /// is a tombstone, and there is no PTY left to type into. `Reach` is
    /// deliberately the strict one — a bare name means "in my project", and
    /// only a full id or an alias reaches out of it, because everything that
    /// resolves through here goes on to spend a peer's model turn.
    pub fn resolve(&self, target: &str, scope: &Scope) -> Result<&DirectoryEntry, String> {
        let live: Vec<&DirectoryEntry> = self
            .entries
            .iter()
            .filter(|entry| entry.agent.status != "stopped")
            .collect();
        let candidates: Vec<Candidate> = live.iter().map(|entry| entry.candidate()).collect();
        let index = crate::resolve::pick(
            &candidates,
            target,
            scope,
            crate::resolve::Reach::ExplicitAcrossProjects,
        )?;
        Ok(live[index])
    }
}

// ---------------------------------------------------------------------------
// Inbox messages + replies
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InboxMessage {
    /// A consult: deliver `message` to session `to` when idle; capture reply.
    ///
    /// The message is written to the ASKER's inbox, never the target's, so the
    /// reply lands in the workspace the waiting CLI is already polling. That
    /// makes `to_workspace` the only record of where the consult is bound, and
    /// it defaults to the asker's own for messages written before consults
    /// could cross a project at all.
    Ask {
        ticket: String,
        from: String,
        to: String,
        #[serde(default)]
        to_workspace: Option<String>,
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

- `workbench agents` — list agent sessions here (id, provider, alias, branch, idle/busy); `--all` also lists agents in other projects
- `workbench transcript <id|alias> --lines 200` — read a peer's recent conversation (exported each time it goes idle)
- `workbench ask <id|alias> "question" --wait` — deliver a question to a live peer and collect its answer (or collect later: `workbench replies <ticket> --wait`)
- `workbench handoff <id|alias> --wait` — ask a peer for a structured summary of its work (done/remaining/decisions/gotchas) before taking over or building on it
- `workbench alias <name>` — set your own alias
- `workbench wait <id|alias>` — block until a peer stops working (add `--json` for a parseable line, `--state idle` to insist it is not merely blocked)

Address peers by id or alias. A provider name like `codex` also works when
only one such agent runs in this project — and you are never a match for
your own provider, so `codex` from a codex agent means the other one. What
is still ambiguous is refused with the candidates named, never guessed at.

### Reaching an agent in another project

Every agent on this machine is addressable, not just the ones in this
project — `workbench agents --all` lists them, and any verb above accepts
one. But only a full id or an alias crosses a project boundary: a bare name
like `codex` always means this project's codex, so that a habit formed here
cannot silently reach into an unrelated repo. Asking for a name this
project does not have names the outside candidates and their ids rather
than guessing at one.

Weigh a cross-project consult harder than a local one. The peer is working
in a different repo and cannot see yours, so ask about what it knows — a
decision it made, an interface it owns, a convention in its codebase — and
put the context it needs INTO the question rather than assuming shared
ground. It is told which project you are asking from, and will tell you
when the question needs a repo it cannot see.

A consult costs the peer a full model turn — consult when the user asks you
to or you are genuinely blocked, not by default.

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
- WAITING on a peer to finish: `workbench wait <id|alias>`, which returns the
  moment it stops working. Do not poll `workbench agents` in a loop, and do
  not sit in `sleep` guessing how long a turn takes.

If the user asks for something known to be counterproductive — e.g. agents
debating in rounds until they agree (models sycophantically converge on
wrong answers), several agents editing the same branch/files (conflicting
writes), or relaying work through a peer that you could do directly (token
cost without benefit) — do not silently comply: briefly tell the user why it
tends to fail, propose the closest effective alternative, and proceed with
the original request only if they confirm.

### Keep your task list current

Workbench mirrors your own task list in its Tasks pane — that pane is how
the user watches progress across several agents at once, and how they add,
reword or drop work without interrupting you.

So whenever a request has more than one step, write the list down as you go
(`TaskCreate`/`TaskUpdate`, or `TodoWrite` if that is what you have) and
keep the states honest: exactly one task in progress, completed the moment
it is done. An empty pane reads as "this agent is doing nothing". Do not
manufacture tasks for genuinely single-step work — a one-line answer or a
one-file edit needs no list.
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

/// The untracked sidecar that carries the block when the conventional file
/// is version-controlled. `AGENTS.md` -> `AGENTS.local.md`; anything already
/// ending in `.local.md` is its own sidecar.
fn local_sidecar_name(name: &str) -> String {
    match name.strip_suffix(".md") {
        Some(stem) if !stem.ends_with(".local") => format!("{stem}.local.md"),
        _ => name.to_string(),
    }
}

/// Is this path version-controlled in the workspace?
fn is_git_tracked(workspace_path: &Path, name: &str) -> bool {
    if !workspace_path.join(".git").exists() {
        return false;
    }
    std::process::Command::new("git")
        .arg("-C")
        .arg(workspace_path)
        .args(["ls-files", "--error-unmatch", "--"])
        .arg(name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Make sure a workspace's instruction files carry the workbench block.
///
/// Placement rules, in order:
///  - the file is GIT-TRACKED: never touch it. `AGENTS.md` is increasingly a
///    project-owned, committed convention file, and this block is guidance
///    about the local dev environment (a TUI, peer sessions, `$WORKBENCH_SESSION`)
///    that no teammate or CI checkout should inherit. Worse, editing a tracked
///    file means any agent running `git add -A` silently commits machine
///    instructions into the shared repo. The block goes to an untracked
///    sidecar (`AGENTS.local.md`) instead, so agents here still get it.
///  - the file is untracked (or absent): create/refresh the fenced section in
///    place. Files we create are added to `.git/info/exclude` so they never
///    show up as untracked noise.
pub fn ensure_workspace_instructions(workspace_path: &Path) -> Result<()> {
    let block = instructions_block();
    for name in ["CLAUDE.local.md", "AGENTS.md"] {
        let mut target = name.to_string();
        if is_git_tracked(workspace_path, name) {
            let sidecar = local_sidecar_name(name);
            if sidecar == name {
                continue; // tracked and has no distinct sidecar: leave it alone
            }
            target = sidecar;
            if is_git_tracked(workspace_path, &target) {
                continue; // the sidecar is tracked too; nothing safe to write
            }
        }
        let path = workspace_path.join(&target);
        let existed = path.exists();
        upsert_instructions_file(&path, &block)?;
        if !existed {
            add_git_exclude(workspace_path, &target);
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
    use std::path::Path;

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

    fn git_init(dir: &Path) {
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@t"],
            vec!["config", "user.name", "t"],
        ] {
            let _ = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }

    fn git_commit_file(dir: &Path, name: &str, body: &str) {
        fs::write(dir.join(name), body).unwrap();
        for args in [vec!["add", name], vec!["commit", "-qm", "add"]] {
            let _ = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }

    #[test]
    fn sidecar_naming() {
        assert_eq!(local_sidecar_name("AGENTS.md"), "AGENTS.local.md");
        assert_eq!(local_sidecar_name("CLAUDE.local.md"), "CLAUDE.local.md");
    }

    /// A project-owned, committed AGENTS.md must never be rewritten: the
    /// block is local-environment guidance, and editing a tracked file
    /// invites `git add -A` to commit it into the shared repo.
    #[test]
    fn tracked_instruction_file_is_never_modified() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git_init(root);
        let original = "# AGENTS\n\nProject-owned guidance.\n";
        git_commit_file(root, "AGENTS.md", original);

        ensure_workspace_instructions(root).unwrap();

        assert_eq!(
            fs::read_to_string(root.join("AGENTS.md")).unwrap(),
            original,
            "tracked AGENTS.md must be left byte-identical"
        );
        let sidecar = fs::read_to_string(root.join("AGENTS.local.md")).unwrap();
        assert!(
            sidecar.contains(BLOCK_BEGIN),
            "the block must still reach agents via the untracked sidecar"
        );
        let exclude =
            fs::read_to_string(root.join(".git/info/exclude")).unwrap_or_default();
        assert!(exclude.lines().any(|l| l.trim() == "AGENTS.local.md"));
        assert!(
            !exclude.lines().any(|l| l.trim() == "AGENTS.md"),
            "a tracked file must never be added to the exclude list"
        );
    }

    /// The ordinary case is unchanged: absent/untracked files are created,
    /// refreshed in place, and excluded.
    #[test]
    fn untracked_instruction_file_is_created_and_excluded() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git_init(root);

        ensure_workspace_instructions(root).unwrap();

        let agents = fs::read_to_string(root.join("AGENTS.md")).unwrap();
        assert!(agents.contains(BLOCK_BEGIN));
        assert!(!root.join("AGENTS.local.md").exists());
        let exclude =
            fs::read_to_string(root.join(".git/info/exclude")).unwrap_or_default();
        assert!(exclude.lines().any(|l| l.trim() == "AGENTS.md"));
    }

    /// Content outside the fence — a project's own sections — survives a
    /// refresh untouched.
    #[test]
    fn upsert_preserves_content_around_the_fence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("AGENTS.md");
        fs::write(
            &path,
            format!("before\n\n{}\nstale\n{}\n\nafter\n", BLOCK_BEGIN, BLOCK_END),
        )
        .unwrap();

        upsert_instructions_file(&path, &instructions_block()).unwrap();

        let out = fs::read_to_string(&path).unwrap();
        assert!(out.starts_with("before"), "content before the fence is kept");
        assert!(out.trim_end().ends_with("after"), "content after the fence is kept");
        assert!(!out.contains("stale"), "the fenced section is replaced");
    }

    fn entry(id: &str, provider: &str, workspace: &str, status: &str) -> DirectoryEntry {
        DirectoryEntry {
            workspace_id: format!("{workspace}-id"),
            workspace_name: workspace.into(),
            agent: agent(id, provider, None, status),
        }
    }

    fn scope(workspace: &str) -> Scope {
        Scope {
            project_id: Some(format!("{workspace}-id")),
            exclude: None,
        }
    }

    /// The point of the directory: an id addresses a session anywhere on the
    /// machine, not just one in the caller's own project.
    #[test]
    fn the_directory_resolves_across_workspaces_by_id() {
        let directory = Directory {
            entries: vec![
                entry("ab12cd34", "claude", "workbench", "idle"),
                entry("ef56ab78", "codex", "canvas", "idle"),
            ],
        };

        let found = directory.resolve("ef56ab78", &scope("workbench")).unwrap();
        assert_eq!(found.agent.id, "ef56ab78");
        assert_eq!(found.workspace_name, "canvas");

        // A bare provider name still means "mine", and says where the other is.
        let err = directory.resolve("codex", &scope("workbench")).unwrap_err();
        assert!(err.contains("ef56ab78") && err.contains("canvas"), "{err}");
    }

    /// A stopped session has no PTY to type into; its roster row is a
    /// tombstone and must not be addressable, here or in another project.
    #[test]
    fn a_stopped_session_is_not_addressable() {
        let directory = Directory {
            entries: vec![
                entry("ab12cd34", "claude", "workbench", "stopped"),
                entry("ef56ab78", "codex", "canvas", "stopped"),
            ],
        };
        assert!(directory.resolve("ab12cd34", &scope("workbench")).is_err());
        assert!(directory.resolve("ef56ab78", &scope("workbench")).is_err());
        assert!(directory.outside("workbench-id").is_empty());
    }

    /// A consult written before consults could cross a project still parses:
    /// it simply names no target workspace, and the TUI falls back to the
    /// asker's own.
    #[test]
    fn an_ask_without_a_target_workspace_still_parses() {
        let raw = r#"{"kind":"ask","ticket":"c1","from":"aaaa1111","to":"bbbb2222",
                      "message":"hi","created_at":"now"}"#;
        let msg: InboxMessage = serde_json::from_str(raw).unwrap();
        match msg {
            InboxMessage::Ask { to_workspace, to, .. } => {
                assert_eq!(to, "bbbb2222");
                assert!(to_workspace.is_none());
            }
            _ => panic!("expected an ask"),
        }
    }

    /// Rosters outlive the workspaces that wrote them, and a stale one claims
    /// its agents are idle forever. Only the open ones survive a retirement.
    #[test]
    fn retiring_stops_the_agents_of_closed_workspaces() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for (ws, status) in [("open-ws", "idle"), ("closed-ws", "idle")] {
            fs::create_dir_all(root.join(ws)).unwrap();
            let roster = Roster {
                workspace_id: ws.into(),
                workspace_name: ws.into(),
                workspace_path: format!("/tmp/{ws}"),
                updated_at: "now".into(),
                agents: vec![agent("aaaa1111", "claude", None, status)],
            };
            fs::write(
                root.join(ws).join("agents.json"),
                serde_json::to_string(&roster).unwrap(),
            )
            .unwrap();
        }
        // A directory with no roster at all must not trip it up.
        fs::create_dir_all(root.join("no-roster")).unwrap();

        let open: std::collections::HashSet<String> = ["open-ws".to_string()].into_iter().collect();
        assert_eq!(retire_closed_rosters(root, &open), 1);

        let read = |ws: &str| -> Roster {
            serde_json::from_slice(&fs::read(root.join(ws).join("agents.json")).unwrap()).unwrap()
        };
        assert_eq!(read("open-ws").agents[0].status, "idle", "an open workspace is left alone");
        assert_eq!(read("closed-ws").agents[0].status, "stopped");

        // Idempotent: a second pass finds nothing left to change.
        assert_eq!(retire_closed_rosters(root, &open), 0);
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
