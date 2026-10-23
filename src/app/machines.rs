//! The Machines window: agents running in Workbench on your other Macs.
//!
//! The data comes from `remote::peers`, which reads each configured Mac's
//! phone view in the background. Everything you do here — reply, answer a
//! question, stop an agent — is the same request that Mac's phone page would
//! send, made on a thread so a slow network never holds up a keystroke.

use super::{Action, AppState, InputMode};
use crate::remote::peers;
use crate::remote::AgentView;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

#[derive(Debug, Default)]
pub struct MachinesUi {
    /// `machine~agent` of the selected agent. An id rather than an index, so
    /// the selection stays put while the lists refresh under it.
    pub selected: Option<String>,
    pub typing: bool,
    /// What is being typed is the paths of files to send, not a reply.
    pub file: bool,
    pub draft: String,
    pub confirm_kill: bool,
    /// Lines scrolled up from the end of the conversation.
    pub scroll: u16,
    /// The last thing a command came back with.
    pub message: Option<String>,
}

/// One remote agent as the window lists it.
#[derive(Debug, Clone)]
pub struct Row {
    pub machine: String,
    pub agent: AgentView,
    /// Its conversation has been read at least once.
    pub loaded: bool,
}

impl Row {
    pub fn key(&self) -> String {
        format!("{}{}{}", self.machine, peers::SEP, self.agent.id)
    }
}

/// Every remote agent, by machine, then project, then name.
pub fn rows() -> Vec<Row> {
    let Some(peers) = peers::get() else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for machine in peers.machines() {
        let Some(snapshot) = machine.snapshot else {
            continue;
        };
        let mut agents = snapshot.agents;
        agents.sort_by(|a, b| {
            (&a.project, a.alias.as_ref().unwrap_or(&a.id))
                .cmp(&(&b.project, b.alias.as_ref().unwrap_or(&b.id)))
        });
        rows.extend(agents.into_iter().map(|agent| Row {
            machine: machine.config.name.clone(),
            agent,
            loaded: false,
        }));
    }
    rows
}

/// The selected agent, with its conversation if it has arrived. Also asks
/// for that conversation to be kept current, so call it while drawing.
pub fn selected(state: &AppState) -> Option<Row> {
    let rows = rows();
    let row = match &state.ui.machines.selected {
        Some(key) => rows.iter().find(|r| &r.key() == key).cloned(),
        None => None,
    }
    .or_else(|| rows.first().cloned())?;
    let detail = peers::get()?.watch(&row.machine, &row.agent.id);
    Some(Row {
        loaded: detail.is_some(),
        agent: detail.unwrap_or(row.agent),
        ..row
    })
}

pub fn is_action(action: &Action) -> bool {
    matches!(
        action,
        Action::OpenMachines
            | Action::CloseMachines
            | Action::MachinesMove(_)
            | Action::MachinesScroll(_)
            | Action::MachinesStartTyping
            | Action::MachinesStartFile
            | Action::MachinesCancel
            | Action::MachinesInput(_)
            | Action::MachinesPaste(_)
            | Action::MachinesBackspace
            | Action::MachinesSend
            | Action::MachinesAnswer(_)
            | Action::MachinesAskKill
            | Action::MachinesConfirmKill
            | Action::MachinesDone(_)
    )
}

pub fn handle(state: &mut AppState, action: Action, action_tx: &mpsc::UnboundedSender<Action>) {
    let ui = &mut state.ui.machines;
    match action {
        Action::OpenMachines => {
            state.ui.input_mode = InputMode::MachinesWindow;
            ui.message = if peers::get().is_none() {
                Some(
                    "No other Macs are configured. Add a [[machines]] entry to user_config.toml."
                        .into(),
                )
            } else {
                None
            };
        }
        Action::CloseMachines => {
            if ui.typing || ui.confirm_kill {
                ui.typing = false;
                ui.file = false;
                ui.confirm_kill = false;
            } else {
                state.ui.input_mode = InputMode::Normal;
            }
        }
        Action::MachinesMove(up) => {
            let rows = rows();
            if rows.is_empty() {
                return;
            }
            let at = ui
                .selected
                .as_ref()
                .and_then(|key| rows.iter().position(|r| &r.key() == key))
                .unwrap_or(0);
            let next = if up {
                at.saturating_sub(1)
            } else {
                (at + 1).min(rows.len() - 1)
            };
            ui.selected = Some(rows[next].key());
            ui.scroll = 0;
            ui.message = None;
        }
        Action::MachinesScroll(up) => {
            ui.scroll = if up {
                ui.scroll.saturating_add(5)
            } else {
                ui.scroll.saturating_sub(5)
            };
        }
        Action::MachinesStartTyping => {
            ui.typing = true;
            ui.file = false;
            ui.confirm_kill = false;
        }
        Action::MachinesStartFile => {
            ui.typing = true;
            ui.file = true;
            ui.confirm_kill = false;
            ui.draft.clear();
        }
        Action::MachinesCancel => {
            ui.typing = false;
            ui.file = false;
            ui.confirm_kill = false;
        }
        Action::MachinesInput(c) => ui.draft.push(c),
        Action::MachinesPaste(text) => ui.draft.push_str(&text),
        Action::MachinesBackspace => {
            ui.draft.pop();
        }
        Action::MachinesSend => {
            let text = ui.draft.trim().to_string();
            if text.is_empty() {
                return;
            }
            let Some(row) = selected(state) else { return };
            let ui = &mut state.ui.machines;
            if ui.file {
                let paths = parse_paths(&text);
                if let Some(missing) = paths.iter().find(|p| !p.exists()) {
                    ui.message = Some(format!("There is no file at {}.", missing.display()));
                    return;
                }
                ui.draft.clear();
                ui.typing = false;
                ui.file = false;
                ui.message = Some(format!("Sending to {}…", row.machine));
                send_files(row, paths, action_tx);
                return;
            }
            ui.draft.clear();
            ui.typing = false;
            ui.scroll = 0;
            ui.message = Some(format!("Sending to {}…", row.machine));
            send(row, "/api/reply", text, Vec::new(), action_tx);
        }
        Action::MachinesAnswer(key) => {
            let Some(row) = selected(state) else { return };
            let Some(prompt) = row.agent.prompt.clone() else {
                return;
            };
            if !prompt.options.iter().any(|o| o.key == key) {
                return;
            }
            let headers = vec![("X-Prompt-Id".to_string(), prompt.id.clone())];
            state.ui.machines.message = Some(format!("Answering {key}…"));
            send(row, "/api/answer", key, headers, action_tx);
        }
        Action::MachinesAskKill => {
            if selected(state).is_some_and(|r| r.agent.status != "stopped") {
                let ui = &mut state.ui.machines;
                ui.confirm_kill = true;
                ui.typing = false;
            }
        }
        Action::MachinesConfirmKill => {
            state.ui.machines.confirm_kill = false;
            let Some(row) = selected(state) else { return };
            state.ui.machines.message = Some("Stopping…".into());
            send(row, "/api/kill", String::new(), Vec::new(), action_tx);
        }
        Action::MachinesDone(outcome) => {
            state.ui.machines.message = Some(match outcome {
                Ok(done) => done,
                Err(err) => err,
            });
        }
        _ => {}
    }
}

/// Send files to the selected agent's Mac and tell that agent where they
/// are, on a thread. Progress comes back as `MachinesDone` messages too.
fn send_files(row: Row, paths: Vec<PathBuf>, action_tx: &mpsc::UnboundedSender<Action>) {
    use crate::remote::transfer::{self, human_bytes};
    let tx = action_tx.clone();
    std::thread::spawn(move || {
        let outcome = (|| -> anyhow::Result<String> {
            let remote = transfer::connect(&row.machine)?;
            let mut shown: Option<Instant> = None;
            let sent = remote.send_files(&paths, &mut |p| {
                if shown.is_none_or(|at| at.elapsed() >= Duration::from_millis(500)) {
                    shown = Some(Instant::now());
                    let _ = tx.send(Action::MachinesDone(Ok(format!(
                        "Sending {}: {} of {}…",
                        p.name,
                        human_bytes(p.sent),
                        human_bytes(p.total)
                    ))));
                }
            })?;
            remote.tell(&row.agent.id, &transfer::notice(&sent, None))?;
            let count = sent.paths.len();
            Ok(format!(
                "Sent {count} file{} ({}) to {}, and told the agent.",
                if count == 1 { "" } else { "s" },
                human_bytes(sent.bytes),
                row.machine
            ))
        })();
        let _ = tx.send(Action::MachinesDone(
            outcome.map_err(|err| format!("{err:#}")),
        ));
    });
}

/// What was typed or dropped: one path as it is, or several, the way a
/// terminal pastes dropped files — separated by spaces, with a space inside
/// a name escaped or the whole name quoted. `~/` is your home.
fn parse_paths(text: &str) -> Vec<PathBuf> {
    let home = |raw: &str| match (raw.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(raw),
    };
    let whole = text.trim().trim_matches(['"', '\'']);
    if home(whole).exists() {
        return vec![home(whole)];
    }
    let mut paths = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = text.trim().chars();
    while let Some(c) = chars.next() {
        match (c, quote) {
            ('\\', None) => current.extend(chars.next()),
            ('"' | '\'', None) => quote = Some(c),
            (c, Some(q)) if c == q => quote = None,
            (' ', None) => {
                if !current.is_empty() {
                    paths.push(home(&std::mem::take(&mut current)));
                }
            }
            (c, _) => current.push(c),
        }
    }
    if !current.is_empty() {
        paths.push(home(&current));
    }
    paths
}

/// Make the request on its own thread and report back with `MachinesDone`.
fn send(
    row: Row,
    path: &'static str,
    text: String,
    headers: Vec<(String, String)>,
    action_tx: &mpsc::UnboundedSender<Action>,
) {
    let tx = action_tx.clone();
    std::thread::spawn(move || {
        let (code, body) = peers::forward(&row.machine, path, &row.agent.id, &text, &headers);
        let outcome = match code {
            200 => Ok(match path {
                "/api/kill" => format!("Stopped on {}.", row.machine),
                "/api/answer" => "Answered.".to_string(),
                _ => format!("Sent to {}.", row.machine),
            }),
            // An older Workbench there has no way to stop an agent remotely.
            404 if path == "/api/kill" && body == "not found" => Err(format!(
                "{} runs an older Workbench without remote stop; update it there.",
                row.machine
            )),
            _ => Err(format!("{}: {body}", row.machine)),
        };
        let _ = tx.send(Action::MachinesDone(outcome));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_files_split_on_spaces_that_are_not_escaped_or_quoted() {
        assert_eq!(
            parse_paths(r"/a/ship\ final.glb '/b/hull tex.png' /c/x"),
            vec![
                PathBuf::from("/a/ship final.glb"),
                PathBuf::from("/b/hull tex.png"),
                PathBuf::from("/c/x"),
            ]
        );
        let dir = tempfile::tempdir().unwrap();
        let spaced = dir.path().join("a b.glb");
        std::fs::write(&spaced, "x").unwrap();
        assert_eq!(
            parse_paths(&format!("  {}  ", spaced.display())),
            vec![spaced],
            "a path that exists as typed is taken whole"
        );
    }
}
