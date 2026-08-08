//! What an agent is doing right now, reported by the agent itself.
//!
//! Workbench used to infer this from the shape of a session's output: silence
//! meant idle, bytes meant busy. That can never distinguish "thinking" from
//! "stopped, waiting for you to approve a tool" — and in a pane full of
//! agents, *which one is blocked on me* is the status that matters.
//!
//! Claude Code emits lifecycle hooks, so agents we spawn are given a hook that
//! runs `workbench hook <event>`. That process inherits `WORKBENCH_SESSION`
//! from its PTY, which is the whole correlation problem solved: the event
//! arrives already knowing whose it is. The hook writes one small file per
//! session; the TUI reads it on its tick.
//!
//! ```text
//! <config>/workbench/comms/<workspace-id>/status/<session-short-id>.json
//! ```
//!
//! Providers without a hook contract keep the old inference (see
//! `AppState::activity`), so this is additive: better where we have it,
//! unchanged where we don't.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::comms;

/// Why an agent stopped and needs the user. A closed set: the UI may act on
/// these, never on provider wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attention {
    /// A tool or permission gate is waiting on an approval.
    Permission,
    /// The agent asked something and stopped for the answer.
    Question,
    /// Anything else that blocks on the user typing.
    Input,
}

impl Attention {
    pub fn label(&self) -> &'static str {
        match self {
            Attention::Permission => "needs approval",
            Attention::Question => "asked a question",
            Attention::Input => "waiting for you",
        }
    }
}

/// The states a session can be in.
///
/// Deliberately four: a "starting" state would render as busy while the agent
/// sits at an empty prompt, and it is free to take work like any idle one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activity {
    /// Mid-turn: thinking or running tools.
    Working,
    /// Turn finished; the agent is waiting for the next instruction.
    Idle,
    /// Stopped *because* it needs the user.
    NeedsAttention(Attention),
    /// The agent process ended.
    Exited,
}

impl Activity {
    pub fn needs_attention(&self) -> Option<Attention> {
        match self {
            Activity::NeedsAttention(kind) => Some(*kind),
            _ => None,
        }
    }

    /// Whether the session is free to receive work (used for consult
    /// delivery). An agent blocked on a permission prompt is *not* free: it
    /// cannot read a new instruction until the human unblocks it.
    pub fn is_free(&self) -> bool {
        matches!(self, Activity::Idle)
    }
}

/// One session's reported state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStatus {
    pub activity: Activity,
    /// Human prose for the UI. Display only — never matched on.
    pub reason: String,
    pub at: DateTime<Utc>,
    /// The hook event this came from, for debugging.
    pub event: String,
    /// The journal file the agent says it is writing, when the provider's
    /// hooks name one (Claude sends `transcript_path` on every event). This is
    /// the agent in the present tense — it beats every heuristic in
    /// `locate`, and it is the only signal that survives `/clear`, which
    /// rotates the transcript to a new session id in the same process while
    /// the file the old heuristics point at just quietly stops growing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<String>,
}

impl AgentStatus {
    /// Hook data this old is treated as absent: the agent may have died
    /// without a `SessionEnd`, and a stale "working" is worse than falling
    /// back to inference.
    pub const FRESH_FOR: chrono::TimeDelta = chrono::TimeDelta::minutes(30);

    pub fn is_fresh(&self, now: DateTime<Utc>) -> bool {
        now - self.at < Self::FRESH_FOR
    }
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

pub fn status_dir(workspace_id: &str) -> Result<PathBuf> {
    Ok(comms::workspace_dir(workspace_id)?.join("status"))
}

pub fn status_path(workspace_id: &str, session_short_id: &str) -> Result<PathBuf> {
    Ok(status_dir(workspace_id)?.join(format!("{session_short_id}.json")))
}

/// Record a session's state, unless a newer one is already there.
///
/// Hooks are separate processes, so two events can land out of order — a
/// `PostToolUse` finishing after the `Stop` that followed it would otherwise
/// drag a finished agent back to "working".
pub fn record(workspace_id: &str, session_short_id: &str, status: &AgentStatus) -> Result<()> {
    let path = status_path(workspace_id, session_short_id)?;
    if let Some(existing) = read_status(&path) {
        if existing.at > status.at {
            return Ok(());
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    comms::write_atomic(&path, serde_json::to_string(status)?.as_bytes())
}

fn read_status(path: &std::path::Path) -> Option<AgentStatus> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Every session's reported state in a workspace, keyed by short session id.
pub fn load_all(workspace_id: &str) -> Vec<(String, AgentStatus)> {
    let Ok(dir) = status_dir(workspace_id) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                return None;
            }
            let short_id = path.file_stem()?.to_string_lossy().into_owned();
            Some((short_id, read_status(&path)?))
        })
        .collect()
}

/// Drop a session's state file (session deleted, or its agent replaced).
pub fn forget(workspace_id: &str, session_short_id: &str) {
    if let Ok(path) = status_path(workspace_id, session_short_id) {
        let _ = fs::remove_file(path);
    }
}

// ---------------------------------------------------------------------------
// Provider hook wiring
// ---------------------------------------------------------------------------

/// The hook events workbench installs. Verified to fire on Claude Code
/// 2.1.220; unknown event names are not worth the risk of a settings parse
/// error at spawn.
///
/// `SubagentStop` is deliberately absent: it fires *after* the `Stop` at the
/// end of a turn, so hooking it would flip a freshly idle session back to
/// working.
pub const CLAUDE_HOOK_EVENTS: [&str; 7] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Notification",
    "Stop",
    "SessionEnd",
];

/// The hook events Codex is given. Verified on Codex 0.145.0.
///
/// Codex has no session-end event — a dead process is something workbench can
/// see for itself — and `SubagentStop` is skipped for the same reason as
/// Claude's: it can arrive after the parent's `Stop`.
pub const CODEX_HOOK_EVENTS: [&str; 7] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "PreCompact",
    "Stop",
];

/// Turn a hook event into a state.
///
/// The event vocabulary overlaps across providers by design — both Claude and
/// Codex send `Stop`, `PreToolUse` and friends — so one mapping serves both.
/// `payload` is the JSON the provider writes to the hook's stdin; every field
/// read here is optional, so a payload we cannot parse still yields the
/// event's base state.
pub fn interpret(event: &str, payload: Option<&serde_json::Value>) -> Option<AgentStatus> {
    let field = |name: &str| {
        payload
            .and_then(|p| p.get(name))
            .and_then(serde_json::Value::as_str)
    };

    let (activity, reason) = match event {
        // Started, but with nothing asked of it yet: free, not busy.
        "SessionStart" => (Activity::Idle, "session started".to_string()),
        "UserPromptSubmit" => (Activity::Working, "working on your prompt".to_string()),
        "PreToolUse" | "PostToolUse" => {
            let tool = field("tool_name").unwrap_or("a tool");
            (Activity::Working, format!("running {tool}"))
        }
        // The only event that reports a *blocked* agent. Claude sends the
        // prose it would have shown you, e.g. "Claude needs your permission
        // to use Bash" or "Claude is waiting for your input".
        "Notification" => {
            let message = field("message").unwrap_or("waiting for you");
            (
                Activity::NeedsAttention(classify_notification(message)),
                message.to_string(),
            )
        }
        // Codex's own permission gate — a stronger signal than Claude's
        // prose, since it says outright that approval is what is wanted.
        "PermissionRequest" => (
            Activity::NeedsAttention(Attention::Permission),
            field("tool_name")
                .map(|tool| format!("wants to run {tool}"))
                .unwrap_or_else(|| "needs approval".to_string()),
        ),
        "PreCompact" | "PostCompact" | "SubagentStart" => {
            (Activity::Working, "compacting context".to_string())
        }
        "Stop" => {
            // A Stop raised *by* a stop hook means the turn is continuing,
            // not finishing.
            let re_entered = payload
                .and_then(|p| p.get("stop_hook_active"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            if re_entered {
                (Activity::Working, "continuing after a stop hook".to_string())
            } else {
                (Activity::Idle, "finished its turn".to_string())
            }
        }
        "SessionEnd" => (Activity::Exited, "session ended".to_string()),
        _ => return None,
    };

    Some(AgentStatus {
        activity,
        reason,
        at: Utc::now(),
        event: event.to_string(),
        transcript: field("transcript_path").map(str::to_string),
    })
}

/// Which kind of block a notification describes. Wording is matched loosely
/// and falls back to the generic "waiting for you", so a reworded message
/// degrades to a weaker label rather than a wrong state.
fn classify_notification(message: &str) -> Attention {
    let m = message.to_lowercase();
    if m.contains("permission") || m.contains("approve") || m.contains("approval") {
        Attention::Permission
    } else if m.contains('?') || m.contains("question") {
        Attention::Question
    } else {
        Attention::Input
    }
}

/// The `-c` overrides that point Codex's hooks at our wrapper script.
///
/// Codex takes hook config as TOML on the command line, so nothing is written
/// to the user's `~/.codex/config.toml` — which matters, since that file
/// commonly already chains other tools through `notify`.
pub fn codex_hook_args(script: &Path) -> Vec<String> {
    let script = script.to_string_lossy();
    CODEX_HOOK_EVENTS
        .iter()
        .flat_map(|event| {
            [
                "-c".to_string(),
                // `command` must be a bare string: Codex rejects an array
                // outright, and silently declines to run a string carrying
                // arguments. Hence the wrapper.
                format!(r#"hooks.{event}=[{{hooks=[{{type="command",command="{script}",timeout=30}}]}}]"#),
            ]
        })
        .collect()
}

/// Where the Codex wrapper can live.
///
/// Codex runs a hook command by splitting it, so a path containing whitespace
/// never executes — and it says nothing when that happens. On macOS the
/// natural home (`~/Library/Application Support/workbench`) has a space in it,
/// so fall back to a dotted directory, and give up rather than install a hook
/// that will silently never run.
fn codex_hook_dir(config_dir: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    let candidates = [
        config_dir.map(|dir| dir.join("workbench").join("hooks")),
        home.map(|dir| dir.join(".workbench").join("hooks")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|path| !path.to_string_lossy().contains(char::is_whitespace))
}

/// Write (or refresh) the argument-free script Codex's hooks invoke, and
/// return its path.
///
/// One script serves every event: the payload names it in `hook_event_name`,
/// and `workbench hook` reads it from there.
pub fn ensure_codex_hook_script(workbench_bin: &str) -> Result<PathBuf> {
    let dir = codex_hook_dir(dirs::config_dir().as_deref(), dirs::home_dir().as_deref())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no whitespace-free directory for the hook script; Codex would never run it"
            )
        })?;
    fs::create_dir_all(&dir)?;
    let path = dir.join("codex-hook.sh");

    let body = format!(
        "#!/bin/sh\n\
         # Generated by workbench. Codex hook commands take no arguments, so\n\
         # this passes the payload through and lets the event name come from it.\n\
         exec {} hook\n",
        shell_quote(workbench_bin)
    );
    if fs::read_to_string(&path).ok().as_deref() != Some(body.as_str()) {
        comms::write_atomic(&path, body.as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(path)
}

/// Whether an agent command can be asked to report its status.
///
/// Only Claude has a hook contract wired up so far, and only if its build
/// accepts `--settings` — checked once and cached, because a Claude old enough
/// to reject the flag would refuse to start at all, and a session that will
/// not start is a far worse trade than a session without status.
pub fn supports_status_hooks(command: &str) -> bool {
    use std::sync::OnceLock;
    static CLAUDE: OnceLock<bool> = OnceLock::new();
    match command {
        "claude" => *CLAUDE.get_or_init(|| help_mentions_settings(command)),
        // Gated further at the spawn site: Codex needs its hooks trusted.
        "codex" => true,
        _ => false,
    }
}

fn help_mentions_settings(command: &str) -> bool {
    std::process::Command::new(command)
        .arg("--help")
        .output()
        .map(|out| {
            let help = String::from_utf8_lossy(&out.stdout);
            help.contains("--settings")
        })
        .unwrap_or(false)
}

/// The `--settings` JSON that wires every hook event to `workbench hook`.
///
/// Passed at spawn instead of written into `~/.claude/settings.json`: Claude
/// *merges* what `--settings` provides with the user's own settings, so their
/// existing hooks keep running and workbench leaves no trace behind.
pub fn claude_hook_settings(workbench_bin: &str) -> String {
    let hooks: serde_json::Map<String, serde_json::Value> = CLAUDE_HOOK_EVENTS
        .iter()
        .map(|event| {
            let command = format!("{} hook {event}", shell_quote(workbench_bin));
            (
                (*event).to_string(),
                serde_json::json!([{
                    "matcher": "",
                    "hooks": [{"type": "command", "command": command}]
                }]),
            )
        })
        .collect();
    serde_json::json!({ "hooks": hooks }).to_string()
}

/// Hook commands are run through a shell, so a path with spaces needs quoting.
fn shell_quote(path: &str) -> String {
    format!("'{}'", path.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(json: serde_json::Value) -> Option<serde_json::Value> {
        Some(json)
    }

    #[test]
    fn a_turn_runs_from_prompt_through_tools_to_idle() {
        let states = ["UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop"]
            .map(|e| interpret(e, None).unwrap().activity);
        assert_eq!(
            states,
            [
                Activity::Working,
                Activity::Working,
                Activity::Working,
                Activity::Idle
            ]
        );
    }

    #[test]
    fn a_session_that_just_started_is_idle_not_busy() {
        // It is sitting at an empty prompt: free for work, and a spinner
        // would claim otherwise for as long as nobody typed.
        let started = interpret("SessionStart", None).unwrap();
        assert_eq!(started.activity, Activity::Idle);
        assert!(started.activity.is_free());
    }

    #[test]
    fn a_notification_is_the_signal_that_the_agent_is_blocked() {
        let permission = interpret(
            "Notification",
            payload(serde_json::json!({
                "message": "Claude needs your permission to use Bash"
            }))
            .as_ref(),
        )
        .unwrap();
        assert_eq!(
            permission.activity,
            Activity::NeedsAttention(Attention::Permission)
        );
        // The prose is carried through for display.
        assert!(permission.reason.contains("permission to use Bash"));

        let waiting = interpret(
            "Notification",
            payload(serde_json::json!({"message": "Claude is waiting for your input"})).as_ref(),
        )
        .unwrap();
        assert_eq!(waiting.activity, Activity::NeedsAttention(Attention::Input));
    }

    #[test]
    fn an_unparseable_payload_still_yields_the_events_state() {
        // Hooks must never be a source of failure; a missing payload just
        // costs detail.
        assert_eq!(
            interpret("PreToolUse", None).unwrap().activity,
            Activity::Working
        );
        assert_eq!(
            interpret("Notification", None).unwrap().activity,
            Activity::NeedsAttention(Attention::Input)
        );
        assert!(interpret("SomeFutureEvent", None).is_none());
    }

    #[test]
    fn tool_names_reach_the_reason_line() {
        let status = interpret(
            "PreToolUse",
            payload(serde_json::json!({"tool_name": "Edit"})).as_ref(),
        )
        .unwrap();
        assert_eq!(status.reason, "running Edit");
    }

    #[test]
    fn the_codex_wrapper_avoids_paths_codex_cannot_run() {
        // Verified against Codex 0.145.0: a hook command containing a space
        // never executes and nothing is reported, so macOS's
        // "Application Support" must not be used.
        let mac_config = PathBuf::from("/Users/x/Library/Application Support");
        let home = PathBuf::from("/Users/x");
        assert_eq!(
            codex_hook_dir(Some(&mac_config), Some(&home)),
            Some(PathBuf::from("/Users/x/.workbench/hooks"))
        );

        // A whitespace-free config dir (Linux) is used directly.
        let linux_config = PathBuf::from("/home/x/.config");
        assert_eq!(
            codex_hook_dir(Some(&linux_config), Some(&home)),
            Some(PathBuf::from("/home/x/.config/workbench/hooks"))
        );

        // Nowhere safe: no hook is better than one that silently never runs.
        let spaced_home = PathBuf::from("/Users/my name");
        assert_eq!(codex_hook_dir(Some(&mac_config), Some(&spaced_home)), None);
    }

    #[test]
    fn codex_permission_requests_are_the_clearest_attention_signal() {
        let asked = interpret(
            "PermissionRequest",
            payload(serde_json::json!({"tool_name": "shell"})).as_ref(),
        )
        .unwrap();
        assert_eq!(
            asked.activity,
            Activity::NeedsAttention(Attention::Permission)
        );
        assert_eq!(asked.reason, "wants to run shell");
    }

    #[test]
    fn a_stop_raised_by_a_stop_hook_is_not_the_end_of_the_turn() {
        let finished = interpret(
            "Stop",
            payload(serde_json::json!({"stop_hook_active": false})).as_ref(),
        )
        .unwrap();
        assert_eq!(finished.activity, Activity::Idle);

        // Codex re-enters Stop while a stop hook drives more work; calling
        // that idle would advertise the agent as free mid-turn.
        let continuing = interpret(
            "Stop",
            payload(serde_json::json!({"stop_hook_active": true})).as_ref(),
        )
        .unwrap();
        assert_eq!(continuing.activity, Activity::Working);
    }

    #[test]
    fn codex_hook_args_are_bare_command_strings() {
        let args = codex_hook_args(Path::new("/cfg/workbench/hooks/codex-hook.sh"));
        assert_eq!(args.len(), CODEX_HOOK_EVENTS.len() * 2);
        assert_eq!(args[0], "-c");
        // Codex rejects an array outright and silently declines a string that
        // carries arguments, so the command must be exactly the script path.
        assert_eq!(
            args[1],
            r#"hooks.SessionStart=[{hooks=[{type="command",command="/cfg/workbench/hooks/codex-hook.sh",timeout=30}]}]"#
        );
        assert!(args.iter().any(|a| a.starts_with("hooks.PermissionRequest=")));
        assert!(!args.iter().any(|a| a.contains("SessionEnd")));
    }

    #[test]
    fn only_verified_events_are_installed() {
        let settings = claude_hook_settings("/usr/local/bin/workbench");
        let parsed: serde_json::Value = serde_json::from_str(&settings).unwrap();
        let hooks = parsed["hooks"].as_object().unwrap();

        assert_eq!(hooks.len(), CLAUDE_HOOK_EVENTS.len());
        assert!(
            !hooks.contains_key("SubagentStop"),
            "SubagentStop fires after Stop and would undo a freshly idle row"
        );
        assert_eq!(
            hooks["Stop"][0]["hooks"][0]["command"],
            "'/usr/local/bin/workbench' hook Stop"
        );
    }

    #[test]
    fn an_agent_binary_we_cannot_run_reports_no_settings_support() {
        // Rather than spawn Claude with a flag it may reject, an unusable or
        // missing binary simply means no hooks.
        assert!(!help_mentions_settings("workbench-no-such-binary-xyz"));
    }

    #[test]
    fn providers_without_a_hook_contract_are_left_alone() {
        // Codex's `notify` would clobber whatever the user already has
        // configured, so it stays on output-timing inference for now.
        for command in ["gemini", "grok", "opencode", "bash"] {
            assert!(!supports_status_hooks(command), "{command}");
        }
    }

    #[test]
    fn a_binary_path_with_spaces_survives_the_shell() {
        let settings = claude_hook_settings("/Applications/My Tools/workbench");
        let parsed: serde_json::Value = serde_json::from_str(&settings).unwrap();
        assert_eq!(
            parsed["hooks"]["Stop"][0]["hooks"][0]["command"],
            "'/Applications/My Tools/workbench' hook Stop"
        );
    }

    #[test]
    fn an_older_event_cannot_drag_a_finished_agent_back_to_working() {
        let dir = tempfile::tempdir().unwrap();
        // `record` writes under the comms root, so point HOME-derived config
        // at a temp dir by writing through the same path helpers.
        let workspace = format!("test-{}", uuid::Uuid::new_v4());
        let stop = AgentStatus {
            activity: Activity::Idle,
            reason: "finished its turn".into(),
            at: Utc::now(),
            event: "Stop".into(),
            transcript: None,
        };
        let late_tool = AgentStatus {
            activity: Activity::Working,
            reason: "running Bash".into(),
            at: stop.at - chrono::TimeDelta::seconds(1),
            event: "PostToolUse".into(),
            transcript: None,
        };

        record(&workspace, "abc12345", &stop).unwrap();
        record(&workspace, "abc12345", &late_tool).unwrap();

        let loaded = load_all(&workspace);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].0, "abc12345");
        assert_eq!(
            loaded[0].1.activity,
            Activity::Idle,
            "the later Stop must win over an out-of-order tool event"
        );

        forget(&workspace, "abc12345");
        assert!(load_all(&workspace).is_empty());
        drop(dir);
    }

    #[test]
    fn stale_hook_data_stops_being_believed() {
        let now = Utc::now();
        let fresh = AgentStatus {
            activity: Activity::Working,
            reason: String::new(),
            at: now - chrono::TimeDelta::minutes(5),
            event: "PreToolUse".into(),
            transcript: None,
        };
        let stale = AgentStatus {
            at: now - chrono::TimeDelta::hours(2),
            ..fresh.clone()
        };
        assert!(fresh.is_fresh(now));
        assert!(!stale.is_fresh(now));
    }
}
