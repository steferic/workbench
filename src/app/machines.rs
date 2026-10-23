//! The Machines window: agents running in Workbench on your other Macs.
//!
//! The data comes from `remote::peers`, which reads each configured Mac's
//! phone view in the background. Everything you do here — reply, answer a
//! question, stop an agent — is the same request that Mac's phone page would
//! send, made on a thread so a slow network never holds up a keystroke.

use super::{Action, AppState, InputMode};
use crate::remote::peers;
use crate::remote::AgentView;
use tokio::sync::mpsc;

#[derive(Debug, Default)]
pub struct MachinesUi {
    /// `machine~agent` of the selected agent. An id rather than an index, so
    /// the selection stays put while the lists refresh under it.
    pub selected: Option<String>,
    pub typing: bool,
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
            ui.confirm_kill = false;
        }
        Action::MachinesCancel => {
            ui.typing = false;
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
