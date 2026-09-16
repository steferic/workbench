//! The server list and the phone share one project attribution path.
use super::{Action, AppState, SessionsTab};
use crate::ports::{DevServer, ServerKey};
use ratatui::layout::Rect;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub struct Row {
    pub project: Uuid,
    pub name: String,
    pub server: DevServer,
}

#[derive(Clone, Debug)]
pub struct Dialog {
    pub row: Row,
    pub confirming: bool,
}

#[derive(Default)]
pub struct ServerUi {
    pub resolved_roots: HashMap<PathBuf, PathBuf>,
    pub current_project: bool,
    pub selected: Option<ServerKey>,
    pub offset: usize,
    pub dialog: Option<Dialog>,
    pub stopping: HashSet<ServerKey>,
    pub message: Option<String>,
    pub scan_error: Option<String>,
    pub hits: Vec<(Rect, Action)>,
}

impl ServerUi {
    pub fn is_stopping(&self, server: &DevServer) -> bool {
        self.stopping
            .iter()
            .any(|key| key.pid == server.pid && key.start == server.start)
    }
}

pub fn workspace_roots(state: &AppState) -> Vec<(PathBuf, Uuid)> {
    let mut roots = Vec::new();
    for workspace in &state.data.workspaces {
        roots.push((workspace.path.clone(), workspace.id));
        for session in state.data.sessions.get(&workspace.id).into_iter().flatten() {
            if let Some(path) = &session.worktree_path {
                roots.push((path.clone(), workspace.id));
            }
        }
    }
    roots
}

pub fn roots(state: &AppState) -> Vec<(PathBuf, Uuid)> {
    workspace_roots(state)
        .into_iter()
        .map(|(path, id)| {
            let resolved = state
                .ui
                .servers
                .resolved_roots
                .get(&path)
                .cloned()
                .unwrap_or(path);
            (resolved, id)
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct Scan {
    pub servers: Vec<DevServer>,
    pub resolved_roots: HashMap<PathBuf, PathBuf>,
}

/// Resolve symlinks alongside lsof, outside the UI's render/event loop.
pub fn scan(roots: Vec<(PathBuf, Uuid)>) -> Result<Scan, String> {
    let servers = crate::ports::scan().map_err(|e| e.to_string())?;
    let resolved_roots = roots
        .into_iter()
        .filter_map(|(path, _)| path.canonicalize().ok().map(|resolved| (path, resolved)))
        .collect();
    Ok(Scan {
        servers,
        resolved_roots,
    })
}

pub fn all_rows(state: &AppState) -> Vec<Row> {
    let roots = roots(state);
    let mut rows: Vec<_> = crate::ports::owned_by(&state.system.dev_servers, &roots)
        .into_iter()
        .filter_map(|(server, project)| {
            let workspace = state.data.workspaces.iter().find(|w| w.id == project)?;
            Some(Row {
                project,
                name: workspace.name.clone(),
                server: server.clone(),
            })
        })
        .collect();
    rows.sort_by(|a, b| {
        (&a.name, a.server.port, a.server.pid).cmp(&(&b.name, b.server.port, b.server.pid))
    });
    rows
}

pub fn rows(state: &AppState) -> Vec<Row> {
    let project = state.selected_workspace().map(|w| w.id);
    all_rows(state)
        .into_iter()
        .filter(|row| !state.ui.servers.current_project || Some(row.project) == project)
        .collect()
}

pub fn reconcile(state: &mut AppState) {
    let rows = rows(state);
    if !rows
        .iter()
        .any(|row| Some(row.server.key()) == state.ui.servers.selected)
    {
        state.ui.servers.selected = rows.first().map(|row| row.server.key());
        state.ui.servers.offset = 0;
    }
}

pub fn move_selection(state: &mut AppState, up: bool) {
    let rows = rows(state);
    let index = rows
        .iter()
        .position(|r| Some(r.server.key()) == state.ui.servers.selected)
        .unwrap_or(0);
    let next = if up {
        index.saturating_sub(1)
    } else {
        (index + 1).min(rows.len().saturating_sub(1))
    };
    state.ui.servers.selected = rows.get(next).map(|r| r.server.key());
}

fn selected(state: &AppState) -> Option<Row> {
    if let Some(dialog) = &state.ui.servers.dialog {
        return Some(dialog.row.clone());
    }
    rows(state)
        .into_iter()
        .find(|r| Some(r.server.key()) == state.ui.servers.selected)
}

pub fn handle(state: &mut AppState, action: Action, tx: &UnboundedSender<Action>) {
    match action {
        Action::SetSessionsTab(tab) => {
            state.set_sessions_tab(tab);
            reconcile(state);
        }
        Action::SelectServer(key) => {
            if rows(state).iter().any(|r| r.server.key() == key) {
                state.ui.servers.selected = Some(key);
            }
        }
        Action::ServerScope => {
            state.ui.servers.current_project = !state.ui.servers.current_project;
            reconcile(state);
        }
        Action::ServerRefresh => {
            state.system.last_port_scan = None;
        }
        Action::ServerDetails | Action::ServerAskStop => {
            if let Some(row) = selected(state) {
                let confirming = matches!(action, Action::ServerAskStop);
                if state.ui.servers.is_stopping(&row.server) {
                    return;
                }
                state.ui.servers.dialog = Some(Dialog { row, confirming });
                state.ui.pressed_link = None;
            }
        }
        Action::ServerOpen => {
            if let Some(row) = selected(state) {
                if let Err(error) = crate::links::open(&row.server.url()) {
                    state.ui.servers.message = Some(error.to_string());
                }
            }
        }
        Action::ServerClose => {
            state.ui.servers.dialog = None;
        }
        Action::ServerConfirmStop => {
            let Some(dialog) = state.ui.servers.dialog.take().filter(|d| d.confirming) else {
                return;
            };
            let key = dialog.row.server.key();
            if state.ui.servers.is_stopping(&dialog.row.server) {
                return;
            }
            state.ui.servers.stopping.insert(key);
            state.ui.servers.message = Some(format!("Stopping :{}…", key.port));
            let tx = tx.clone();
            state.system.cleanup_jobs.spawn(HashSet::new(), move || {
                let result = crate::ports::stop(&dialog.row.server).map_err(|e| e.to_string());
                let _ = tx.send(Action::ServerStopped(key, result));
                Ok(())
            });
        }
        Action::ServerStopped(key, result) => {
            state.ui.servers.stopping.remove(&key);
            state.ui.servers.message = Some(match result {
                Ok(()) => format!("Stopped :{} (PID {})", key.port, key.pid),
                Err(e) => format!("Could not stop :{}: {e}", key.port),
            });
            state.system.last_port_scan = None;
        }
        _ => {}
    }
    if state.sessions_tab() == SessionsTab::Servers {
        reconcile(state);
    }
}
