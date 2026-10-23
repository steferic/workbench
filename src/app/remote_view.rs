//! Another Mac in the main panes.
//!
//! Tab in the Workspaces pane swaps the three main panes over to another
//! Mac's Workbench: its projects on top, the selected project's agents below,
//! and the selected agent's live terminal on the right — keys typed there go
//! to that agent. Tab again moves on to the next Mac, and after the last one
//! back to this one.
//!
//! Nothing here touches this Mac's sessions. The lists come from
//! `remote::peers`, which reads each Mac's state every couple of seconds;
//! the terminal is a `vt100` parser rebuilt from the frames that Mac sends
//! (see `remote::screen`), so it draws with the same code as a local agent.

use super::{Action, AppState, FocusPanel};
use crate::remote::peers;
use crate::remote::{AgentView, ProjectView};

#[derive(Default)]
pub struct RemoteViewUi {
    /// The Mac on show; `None` is this one.
    pub machine: Option<String>,
    /// Ids on that Mac. Ids rather than indexes, so the selection stays put
    /// while the lists refresh under it.
    pub project: Option<String>,
    pub agent: Option<String>,
    mirror: Option<Mirror>,
}

/// The selected agent's terminal as last received.
struct Mirror {
    machine: String,
    agent: String,
    tag: String,
    parser: vt100::Parser,
}

pub fn active(state: &AppState) -> bool {
    state.ui.remote.machine.is_some()
}

/// Every project on that Mac, in its own order.
pub fn projects(machine: &str) -> Vec<ProjectView> {
    peers::get()
        .and_then(|p| p.machines().into_iter().find(|m| m.config.name == machine))
        .and_then(|m| m.snapshot)
        .map(|s| s.projects)
        .unwrap_or_default()
}

/// That Mac's agents in one project, by name.
pub fn agents(machine: &str, project: &str) -> Vec<AgentView> {
    let mut agents: Vec<AgentView> = peers::get()
        .and_then(|p| p.machines().into_iter().find(|m| m.config.name == machine))
        .and_then(|m| m.snapshot)
        .map(|s| s.agents)
        .unwrap_or_default()
        .into_iter()
        .filter(|a| a.project_id == project)
        .collect();
    agents.sort_by(|a, b| {
        (a.alias.is_none(), a.alias.as_ref(), &a.provider, &a.id).cmp(&(
            b.alias.is_none(),
            b.alias.as_ref(),
            &b.provider,
            &b.id,
        ))
    });
    agents
}

/// The selected project, or the first one.
pub fn selected_project(state: &AppState) -> Option<ProjectView> {
    let ui = &state.ui.remote;
    let projects = projects(ui.machine.as_deref()?);
    let wanted = ui.project.as_ref();
    projects
        .iter()
        .find(|p| Some(&p.id) == wanted)
        .or_else(|| projects.first())
        .cloned()
}

/// The selected agent in the selected project, or its first one.
pub fn selected_agent(state: &AppState) -> Option<AgentView> {
    let machine = state.ui.remote.machine.as_deref()?;
    let project = selected_project(state)?;
    let agents = agents(machine, &project.id);
    let wanted = state.ui.remote.agent.as_ref();
    agents
        .iter()
        .find(|a| Some(&a.id) == wanted)
        .or_else(|| agents.first())
        .cloned()
}

/// The selected agent's terminal, brought up to date with the latest frame.
/// Also keeps that frame coming, so call it while drawing.
pub fn mirror(state: &mut AppState) -> Option<&vt100::Parser> {
    let machine = state.ui.remote.machine.clone()?;
    let agent = selected_agent(state)?;
    let frame = peers::get()?.screen(&machine, &agent.id);
    let ui = &mut state.ui.remote;
    let same_agent = ui
        .mirror
        .as_ref()
        .is_some_and(|m| m.machine == machine && m.agent == agent.id);
    match frame {
        Some(frame) if !same_agent || ui.mirror.as_ref().is_some_and(|m| m.tag != frame.tag) => {
            ui.mirror = Some(Mirror {
                machine,
                agent: agent.id,
                tag: frame.tag.clone(),
                parser: frame.parser(),
            });
        }
        _ if !same_agent => ui.mirror = None,
        _ => {}
    }
    ui.mirror.as_ref().map(|m| &m.parser)
}

pub fn is_action(action: &Action) -> bool {
    matches!(
        action,
        Action::RemoteNextMachine
            | Action::RemoteMoveProject(_)
            | Action::RemoteMoveAgent(_)
            | Action::RemoteFocus(_)
            | Action::RemoteKeys(_)
    )
}

pub fn handle(state: &mut AppState, action: Action) {
    match action {
        Action::RemoteNextMachine => {
            let names: Vec<String> = peers::get()
                .map(|p| p.machines().into_iter().map(|m| m.config.name).collect())
                .unwrap_or_default();
            let ui = &mut state.ui.remote;
            ui.machine = match &ui.machine {
                None => names.first().cloned(),
                Some(current) => names
                    .iter()
                    .position(|n| n == current)
                    .and_then(|i| names.get(i + 1))
                    .cloned(),
            };
            ui.project = None;
            ui.agent = None;
            ui.mirror = None;
            state.ui.focus = FocusPanel::WorkspaceList;
        }
        Action::RemoteMoveProject(up) => {
            let Some(machine) = state.ui.remote.machine.clone() else {
                return;
            };
            let projects = projects(&machine);
            let current = selected_project(state).map(|p| p.id);
            if let Some(next) = step(&projects, current.as_ref(), up, |p| &p.id) {
                state.ui.remote.project = Some(next.id.clone());
                state.ui.remote.agent = None;
            }
        }
        Action::RemoteMoveAgent(up) => {
            let (Some(machine), Some(project)) =
                (state.ui.remote.machine.clone(), selected_project(state))
            else {
                return;
            };
            let agents = agents(&machine, &project.id);
            let current = selected_agent(state).map(|a| a.id);
            if let Some(next) = step(&agents, current.as_ref(), up, |a| &a.id) {
                state.ui.remote.agent = Some(next.id.clone());
            }
        }
        Action::RemoteFocus(panel) => state.ui.focus = panel,
        Action::RemoteKeys(bytes) => {
            let (Some(machine), Some(agent)) =
                (state.ui.remote.machine.clone(), selected_agent(state))
            else {
                return;
            };
            if agent.status == "stopped" {
                return;
            }
            if let Some(peers) = peers::get() {
                peers.send_keys(&machine, &agent.id, bytes);
            }
        }
        _ => {}
    }
}

/// The row above or below `current` in `rows`, clamped at the ends.
fn step<'a, T>(
    rows: &'a [T],
    current: Option<&String>,
    up: bool,
    id: impl Fn(&T) -> &String,
) -> Option<&'a T> {
    if rows.is_empty() {
        return None;
    }
    let at = current
        .and_then(|c| rows.iter().position(|r| id(r) == c))
        .unwrap_or(0);
    let next = if up {
        at.saturating_sub(1)
    } else {
        (at + 1).min(rows.len() - 1)
    };
    rows.get(next)
}

/// The bytes a key sends to a terminal, as the TUI sends them to a local
/// agent. `None` for a key that has no bytes.
pub fn key_bytes(key: &crossterm::event::KeyEvent) -> Option<Vec<u8>> {
    use crossterm::event::{KeyCode, KeyModifiers};
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let cmd = key.modifiers.contains(KeyModifiers::SUPER);
    Some(match key.code {
        KeyCode::Char(c) if ctrl => vec![(c as u8) & 0x1f],
        KeyCode::Char(c) if alt => {
            let mut bytes = vec![0x1b];
            bytes.extend(c.to_string().into_bytes());
            bytes
        }
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Esc => vec![0x1b],
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace if alt => vec![0x1b, 0x7f],
        KeyCode::Backspace if cmd => vec![0x15],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Delete if alt => vec![0x1b, b'd'],
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Up if alt => b"\x1b[1;3A".to_vec(),
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down if alt => b"\x1b[1;3B".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right if alt => vec![0x1b, b'f'],
        KeyCode::Right if cmd => vec![0x05],
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left if alt => vec![0x1b, b'b'],
        KeyCode::Left if cmd => vec![0x01],
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => vec![0x01],
        KeyCode::End => vec![0x05],
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::F(n) => match n {
            1 => b"\x1bOP".to_vec(),
            2 => b"\x1bOQ".to_vec(),
            3 => b"\x1bOR".to_vec(),
            4 => b"\x1bOS".to_vec(),
            5 => b"\x1b[15~".to_vec(),
            6 => b"\x1b[17~".to_vec(),
            7 => b"\x1b[18~".to_vec(),
            8 => b"\x1b[19~".to_vec(),
            9 => b"\x1b[20~".to_vec(),
            10 => b"\x1b[21~".to_vec(),
            11 => b"\x1b[23~".to_vec(),
            12 => b"\x1b[24~".to_vec(),
            _ => return None,
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn keys_turn_into_the_bytes_a_local_agent_gets() {
        let key = |code, mods| key_bytes(&KeyEvent::new(code, mods));
        assert_eq!(
            key(KeyCode::Char('a'), KeyModifiers::NONE),
            Some(b"a".to_vec())
        );
        assert_eq!(
            key(KeyCode::Char('é'), KeyModifiers::NONE),
            Some("é".as_bytes().to_vec())
        );
        assert_eq!(
            key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            Some(vec![3])
        );
        assert_eq!(
            key(KeyCode::Enter, KeyModifiers::NONE),
            Some(b"\r".to_vec())
        );
        assert_eq!(key(KeyCode::Esc, KeyModifiers::NONE), Some(vec![0x1b]));
        assert_eq!(
            key(KeyCode::Up, KeyModifiers::NONE),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            key(KeyCode::BackTab, KeyModifiers::SHIFT),
            Some(b"\x1b[Z".to_vec())
        );
        assert_eq!(key(KeyCode::Null, KeyModifiers::NONE), None);
    }

    #[test]
    fn moving_stops_at_the_ends_and_starts_from_the_first_row() {
        let rows = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        fn id(r: &String) -> &String {
            r
        }
        assert_eq!(step(&rows, None, false, id), Some(&"b".to_string()));
        assert_eq!(
            step(&rows, Some(&"c".into()), false, id),
            Some(&"c".to_string())
        );
        assert_eq!(
            step(&rows, Some(&"a".into()), true, id),
            Some(&"a".to_string())
        );
        assert_eq!(
            step(&rows, Some(&"gone".into()), true, id),
            Some(&"a".to_string())
        );
        assert_eq!(step::<String>(&[], None, true, id), None);
    }
}
