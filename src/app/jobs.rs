//! The Jobs window: a project's repeatable agent jobs, read from its own repo.
//!
//! The files are the truth (see `crate::jobs`); this module is the cursor,
//! the scan timer and what happens on Enter. Starting a job is starting an
//! ordinary agent session in the project — aliased after the job, linked to
//! its run, with the composed prompt queued so the existing dispatcher
//! delivers it the moment the agent is ready — plus one line in the ledger.
use super::{Action, AppState, FocusPanel, ToastLevel};
use crate::jobs::{self, JobDef, ProjectJobs, RunRecord, RunStatus, Stamp};
use crate::models::{AgentType, JobLink};
use crate::pty::PtyManager;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::mpsc;
use uuid::Uuid;

#[cfg(test)]
mod tests;

/// Which job in which project. The tab lists several projects at once.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct JobKey {
    pub workspace: Uuid,
    pub id: String,
}

/// The id an unreadable manifest is listed under, so it has a row to select
/// and the error somewhere to be read.
pub const MANIFEST_ERROR_ID: &str = "!manifest";

#[derive(Clone, Debug)]
pub struct Row {
    pub key: JobKey,
    pub project: String,
    /// `None` for the manifest-error row.
    pub job: Option<JobDef>,
    pub error: Option<String>,
    pub last: Option<RunRecord>,
    pub due: bool,
    /// A live session in this workbench with an open run of this job.
    pub open_session: Option<Uuid>,
}

/// The window's right-hand side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DetailTab {
    #[default]
    Overview,
    Runs,
    Lessons,
    Prompt,
}

impl DetailTab {
    pub const ALL: [DetailTab; 4] = [
        DetailTab::Overview,
        DetailTab::Runs,
        DetailTab::Lessons,
        DetailTab::Prompt,
    ];

    pub fn label(self) -> &'static str {
        match self {
            DetailTab::Overview => "Overview",
            DetailTab::Runs => "Runs",
            DetailTab::Lessons => "Lessons",
            DetailTab::Prompt => "Prompt",
        }
    }
}

/// Which side of the window the cursor is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowFocus {
    #[default]
    List,
    Detail,
}

#[derive(Default)]
pub struct JobsUi {
    /// Every project's jobs, or only the selected project's.
    pub all_projects: bool,
    pub selected: Option<JobKey>,
    pub offset: usize,
    pub message: Option<String>,
    pub focus: WindowFocus,
    pub tab: DetailTab,
    /// Which run the Runs tab has its cursor on, newest first.
    pub run_cursor: usize,
    /// How far the other detail tabs are scrolled.
    pub scroll: u16,
}

/// What a scan found for one project.
#[derive(Debug, Clone)]
pub enum Scanned {
    /// Nothing changed since the stamp it was given.
    Unchanged,
    /// No manifest (any more).
    Absent,
    Loaded(ProjectJobs),
}

/// Re-read the projects whose files changed, off the event loop.
pub fn scan(roots: Vec<(Uuid, PathBuf, Option<Stamp>)>) -> Vec<(Uuid, Scanned)> {
    roots
        .into_iter()
        .map(|(id, root, previous)| {
            if !root.join(jobs::MANIFEST).is_file() {
                return (id, Scanned::Absent);
            }
            if let Some(previous) = previous {
                // The stamp remembers which files it watched, so a manifest
                // that moved the history dir or named new instruction files
                // is itself a change it notices.
                if previous.refresh(&root) == previous {
                    return (id, Scanned::Unchanged);
                }
            }
            match jobs::load(&root) {
                Some(project) => (id, Scanned::Loaded(project)),
                None => (id, Scanned::Absent),
            }
        })
        .collect()
}

pub fn apply_scan(state: &mut AppState, results: Vec<(Uuid, Scanned)>) {
    for (id, scanned) in results {
        match scanned {
            Scanned::Unchanged => {}
            Scanned::Absent => {
                state.system.project_jobs.remove(&id);
            }
            Scanned::Loaded(project) => {
                state.system.project_jobs.insert(id, project);
            }
        }
    }
    reconcile(state);
}

pub fn all_rows(state: &AppState) -> Vec<Row> {
    let now = chrono::Utc::now();
    let mut rows = Vec::new();
    for workspace in &state.data.workspaces {
        let Some(project) = state.system.project_jobs.get(&workspace.id) else {
            continue;
        };
        match &project.manifest {
            Some(Err(error)) => rows.push(Row {
                key: JobKey {
                    workspace: workspace.id,
                    id: MANIFEST_ERROR_ID.into(),
                },
                project: workspace.name.clone(),
                job: None,
                error: Some(error.clone()),
                last: None,
                due: false,
                open_session: None,
            }),
            Some(Ok(_)) => {
                for job in project.jobs() {
                    let last = project.last_run(&job.id).cloned();
                    let open_session =
                        last.as_ref()
                            .filter(|run| run.status.is_open())
                            .and_then(|run| {
                                open_session_for(state, workspace.id, &job.id, &run.run_id)
                            });
                    rows.push(Row {
                        key: JobKey {
                            workspace: workspace.id,
                            id: job.id.clone(),
                        },
                        project: workspace.name.clone(),
                        due: project.due(job, now),
                        job: Some(job.clone()),
                        error: None,
                        last,
                        open_session,
                    });
                }
            }
            None => {}
        }
    }
    rows
}

/// The live session running this run, if it is still here.
fn open_session_for(state: &AppState, workspace: Uuid, job_id: &str, run_id: &str) -> Option<Uuid> {
    state
        .data
        .sessions
        .get(&workspace)?
        .iter()
        .find(|session| {
            session.status == crate::models::SessionStatus::Running
                && session
                    .job
                    .as_ref()
                    .is_some_and(|link| link.job_id == job_id && link.run_id == run_id)
        })
        .map(|session| session.id)
}

pub fn rows(state: &AppState) -> Vec<Row> {
    let project = state.selected_workspace().map(|w| w.id);
    all_rows(state)
        .into_iter()
        .filter(|row| state.ui.jobs.all_projects || Some(row.key.workspace) == project)
        .collect()
}

pub fn reconcile(state: &mut AppState) {
    let rows = rows(state);
    if !rows
        .iter()
        .any(|row| Some(&row.key) == state.ui.jobs.selected.as_ref())
    {
        state.ui.jobs.selected = rows.first().map(|row| row.key.clone());
        state.ui.jobs.offset = 0;
    }
}

/// The selected job's runs, newest first, as the Runs tab lists them.
pub fn selected_runs(state: &AppState) -> Vec<RunRecord> {
    let Some(row) = selected(state) else {
        return Vec::new();
    };
    let mut runs = state
        .system
        .project_jobs
        .get(&row.key.workspace)
        .and_then(|p| p.runs.get(&row.key.id))
        .cloned()
        .unwrap_or_default();
    runs.reverse();
    runs
}

pub fn move_selection(state: &mut AppState, up: bool) {
    state.ui.jobs.run_cursor = 0;
    state.ui.jobs.scroll = 0;
    let rows = rows(state);
    let index = rows
        .iter()
        .position(|r| Some(&r.key) == state.ui.jobs.selected.as_ref())
        .unwrap_or(0);
    let next = if up {
        index.saturating_sub(1)
    } else {
        (index + 1).min(rows.len().saturating_sub(1))
    };
    state.ui.jobs.selected = rows.get(next).map(|r| r.key.clone());
}

/// The row the cursor is on. With no cursor yet — the files were read while
/// another project was selected, and nothing has moved it since — it is the
/// first row, which is also the one the window highlights.
pub fn selected(state: &AppState) -> Option<Row> {
    let rows = rows(state);
    match &state.ui.jobs.selected {
        Some(key) => rows
            .iter()
            .find(|r| &r.key == key)
            .or(rows.first())
            .cloned(),
        None => rows.into_iter().next(),
    }
}

/// The agent a job asks for, from the same table the create dialog uses.
///
/// A name the config does not know is refused rather than guessed: a job
/// that says `codex` must not quietly run under whatever is first.
pub fn agent_for(state: &AppState, wanted: Option<&str>) -> Result<AgentType, String> {
    let agents = &state.system.user_config.agents;
    let config = match wanted {
        Some(command) => agents
            .iter()
            .find(|a| a.enabled && a.command.eq_ignore_ascii_case(command))
            .ok_or_else(|| format!("no configured agent runs `{command}` (see h → Agents)"))?,
        None => agents
            .iter()
            .find(|a| a.enabled && a.command == "claude")
            .or_else(|| agents.iter().find(|a| a.enabled))
            .ok_or_else(|| "no agents are configured".to_string())?,
    };
    Ok(match config.command.as_str() {
        "claude" => AgentType::Claude,
        "gemini" => AgentType::Gemini,
        "codex" => AgentType::Codex,
        "grok" => AgentType::Grok,
        _ => AgentType::Custom {
            command: config.command.clone(),
            display_name: config.display_name.clone(),
            badge: config.badge.clone(),
        },
    })
}

pub fn handle(
    state: &mut AppState,
    action: Action,
    pty_manager: &PtyManager,
    pty_tx: &mpsc::Sender<Action>,
) {
    match action {
        Action::OpenJobs => {
            state.ui.input_mode = super::InputMode::JobsWindow;
            state.ui.jobs.message = None;
            state.ui.jobs.focus = WindowFocus::List;
            // Fresh files on opening: a ledger line written a moment ago is
            // exactly what you opened it to see.
            state.system.last_jobs_scan = None;
        }
        Action::CloseJobs => {
            if state.ui.input_mode == super::InputMode::JobsWindow {
                state.ui.input_mode = super::InputMode::Normal;
            }
        }
        Action::JobsSwitchFocus => {
            state.ui.jobs.focus = match state.ui.jobs.focus {
                WindowFocus::List => WindowFocus::Detail,
                WindowFocus::Detail => WindowFocus::List,
            };
        }
        Action::JobsTab(tab) => {
            state.ui.jobs.tab = tab;
            state.ui.jobs.focus = WindowFocus::Detail;
            state.ui.jobs.scroll = 0;
        }
        Action::JobsMove(up) => match state.ui.jobs.focus {
            WindowFocus::List => move_selection(state, up),
            WindowFocus::Detail => {
                if state.ui.jobs.tab == DetailTab::Runs {
                    let count = selected_runs(state).len();
                    let cursor = &mut state.ui.jobs.run_cursor;
                    *cursor = if up {
                        cursor.saturating_sub(1)
                    } else {
                        (*cursor + 1).min(count.saturating_sub(1))
                    };
                } else {
                    let scroll = &mut state.ui.jobs.scroll;
                    *scroll = if up {
                        scroll.saturating_sub(1)
                    } else {
                        scroll.saturating_add(1)
                    };
                }
            }
        },
        Action::SelectJob(key) => {
            if rows(state).iter().any(|r| r.key == key) {
                if state.ui.jobs.selected.as_ref() != Some(&key) {
                    state.ui.jobs.run_cursor = 0;
                    state.ui.jobs.scroll = 0;
                }
                state.ui.jobs.selected = Some(key);
                state.ui.jobs.focus = WindowFocus::List;
            }
        }
        Action::JobsScope => {
            state.ui.jobs.all_projects = !state.ui.jobs.all_projects;
            reconcile(state);
        }
        Action::JobsRefresh => {
            state.system.last_jobs_scan = None;
            state.ui.jobs.message = None;
        }
        Action::JobRun | Action::JobRunForce => {
            let Some(row) = selected(state) else {
                return;
            };
            if let Some(error) = row.error {
                state.ui.jobs.message = Some(error);
                return;
            }
            if matches!(action, Action::JobRun) {
                if let Some(session_id) = row.open_session {
                    state.set_active_session_id(Some(session_id));
                    state.ui.jobs.message = Some(format!(
                        "{} is already running here; R starts another",
                        row.job.as_ref().map(|j| j.title.as_str()).unwrap_or("it")
                    ));
                    return;
                }
            }
            start(state, row.key, pty_manager, pty_tx);
        }
        Action::JobImprove => {
            let Some(row) = selected(state) else {
                return;
            };
            let Some(job) = row.job else {
                state.ui.jobs.message = row.error;
                return;
            };
            let runs = state
                .system
                .project_jobs
                .get(&row.key.workspace)
                .and_then(|p| p.runs.get(&job.id))
                .map(|r| r.len())
                .unwrap_or(0);
            if runs == 0 {
                state.ui.jobs.message =
                    Some(format!("{} has no runs to learn from yet", job.title));
                return;
            }
            let prompt = jobs::prompt::improve(&job, runs.min(10));
            start_helper(
                state,
                row.key.workspace,
                &format!("improve:{}", job.id),
                prompt,
                pty_manager,
                pty_tx,
            );
        }
        Action::JobNew => {
            let Some(workspace) = state.selected_workspace().cloned() else {
                return;
            };
            if !workspace.path.join(jobs::MANIFEST).is_file() {
                match jobs::init(&workspace.path) {
                    Ok(written) => {
                        state.ui.jobs.message = Some(format!(
                            "Created {} and {} more files",
                            jobs::MANIFEST,
                            written.len().saturating_sub(1)
                        ));
                        state.system.last_jobs_scan = None;
                    }
                    Err(error) => {
                        state.ui.jobs.message =
                            Some(format!("Could not create {}: {error}", jobs::MANIFEST));
                        return;
                    }
                }
            }
            start_helper(
                state,
                workspace.id,
                "new-job",
                jobs::prompt::new_job(),
                pty_manager,
                pty_tx,
            );
        }
        Action::JobStart(key) => {
            if all_rows(state)
                .iter()
                .any(|r| r.key == key && r.job.is_some())
            {
                start(state, key, pty_manager, pty_tx);
            } else {
                state.ui.jobs.message = Some(format!("No job `{}` to run", key.id));
            }
        }
        Action::JobsScanned(results) => {
            state.system.jobs_scan_inflight = false;
            apply_scan(state, results);
        }
        _ => {}
    }
    reconcile(state);
}

fn close_window(state: &mut AppState) {
    if state.ui.input_mode == super::InputMode::JobsWindow {
        state.ui.input_mode = super::InputMode::Normal;
    }
    state.ui.focus = FocusPanel::SessionList;
}

/// Whether an action belongs to the Jobs window, so the handler routes it
/// here and a click on one of its targets leaves the focus alone.
pub fn is_jobs_action(action: &Action) -> bool {
    matches!(
        action,
        Action::OpenJobs
            | Action::CloseJobs
            | Action::SelectJob(_)
            | Action::JobsScope
            | Action::JobsRefresh
            | Action::JobsSwitchFocus
            | Action::JobsMove(_)
            | Action::JobsTab(_)
            | Action::JobRun
            | Action::JobRunForce
            | Action::JobImprove
            | Action::JobNew
            | Action::JobStart(_)
            | Action::JobsScanned(_)
    )
}

/// What the overview shows for the selected job: the ledger's totals, and
/// which instruction files have changed since the last run read them.
pub struct Overview {
    pub stats: jobs::Stats,
    /// Each instruction file with its hash now and at the last run.
    pub instructions: Vec<(String, String, Option<String>)>,
    pub changed_since_last_run: usize,
}

pub fn overview(state: &AppState, row: &Row) -> Overview {
    let project = state.system.project_jobs.get(&row.key.workspace);
    let runs = project
        .and_then(|p| p.runs.get(&row.key.id))
        .map(|r| r.as_slice())
        .unwrap_or(&[]);
    let stats = jobs::stats(runs, chrono::Utc::now());
    let now: std::collections::BTreeMap<String, String> = project
        .and_then(|p| p.hashes.get(&row.key.id))
        .cloned()
        .unwrap_or_default();
    let then = runs.last().map(|r| &r.instructions_sha256_16);
    let instructions: Vec<(String, String, Option<String>)> = now
        .into_iter()
        .map(|(path, hash)| {
            let before = then.and_then(|t| t.get(&path)).cloned();
            (path, hash, before)
        })
        .collect();
    let changed_since_last_run = instructions
        .iter()
        .filter(|(_, hash, before)| before.as_ref().is_some_and(|b| b != hash))
        .count();
    Overview {
        stats,
        instructions,
        changed_since_last_run,
    }
}

/// Start a run: a ledger line, then a session linked to it with the prompt
/// queued. The ledger goes first so a session that fails to spawn still
/// leaves a trace; it is closed as aborted right away in that case.
fn start(
    state: &mut AppState,
    key: JobKey,
    pty_manager: &PtyManager,
    pty_tx: &mpsc::Sender<Action>,
) {
    let Some(workspace) = state.get_workspace(key.workspace).cloned() else {
        return;
    };
    let Some(project) = state.system.project_jobs.get(&key.workspace) else {
        return;
    };
    let Some(job) = project.job(&key.id).cloned() else {
        return;
    };
    let history_dir = project.history_dir();
    let agent_type = match agent_for(state, job.agent.as_deref()) {
        Ok(agent) => agent,
        Err(error) => {
            state.ui.jobs.message = Some(error);
            return;
        }
    };
    let run_id = jobs::new_run_id();
    let prompt = match jobs::prompt::compose(&workspace.path, &job, &run_id) {
        Ok(prompt) => prompt,
        Err(error) => {
            state.ui.jobs.message = Some(format!("{}: {error:#}", job.title));
            return;
        }
    };
    let session_id = super::handlers::session::create_job_session(
        state,
        key.workspace,
        agent_type.clone(),
        job.skip_permissions,
        &job.id,
        &run_id,
        pty_manager,
        pty_tx,
    );
    let session_short = session_id.map(crate::models::Session::short_id_of);
    let mut record = jobs::start_record(
        &workspace.path,
        &job,
        &run_id,
        agent_type.command(),
        session_short,
    );
    let Some(session_id) = session_id else {
        record = jobs::close_record(&record, RunStatus::Aborted);
        let _ = jobs::history::append(&workspace.path, &history_dir, &record);
        state.ui.jobs.message = Some(format!("Could not start an agent for {}", job.title));
        return;
    };
    if let Err(error) = jobs::history::append(&workspace.path, &history_dir, &record) {
        state.ui.jobs.message = Some(format!(
            "Started, but could not write the ledger: {error:#}"
        ));
    }
    if let Some(project) = state.system.project_jobs.get_mut(&key.workspace) {
        project.runs.entry(job.id.clone()).or_default().push(record);
    }
    if let Some(session) = state.get_session_mut(session_id) {
        session.alias = Some(job.id.clone());
        session.job = Some(JobLink {
            job_id: job.id.clone(),
            run_id: run_id.clone(),
        });
        session.todo_queue.add(prompt);
    }
    // The window closes so the output pane, which it was covering, shows the
    // agent start; F4 brings it back with the cursor where it was.
    close_window(state);
    state.ui.jobs.selected = Some(key);
    super::handlers::save_state(state, "failed to save a job session");
    toast(state, format!("Started {}", job.title));
    crate::logger::info(format!("job {} started as run {run_id}", job.id));
}

/// An agent for the tab's own errands (improve, new): a session with a prompt
/// queued and a telling alias, and no ledger line — it is not a run.
fn start_helper(
    state: &mut AppState,
    workspace: Uuid,
    alias: &str,
    prompt: String,
    pty_manager: &PtyManager,
    pty_tx: &mpsc::Sender<Action>,
) {
    let agent_type = match agent_for(state, None) {
        Ok(agent) => agent,
        Err(error) => {
            state.ui.jobs.message = Some(error);
            return;
        }
    };
    let session_id = super::handlers::session::create_job_session(
        state,
        workspace,
        agent_type,
        false,
        "",
        "",
        pty_manager,
        pty_tx,
    );
    let Some(session_id) = session_id else {
        state.ui.jobs.message = Some("Could not start an agent".into());
        return;
    };
    if let Some(session) = state.get_session_mut(session_id) {
        session.alias = Some(alias.to_string());
        session.todo_queue.add(prompt);
    }
    close_window(state);
    super::handlers::save_state(state, "failed to save a helper session");
    toast(state, format!("Started {alias}"));
}

/// Close a session's run, if it has one still open, with a later ledger line.
///
/// The ledger on disk decides whether the run is open, not memory: the agent
/// reports by appending a line, and the scan that would tell us about it
/// runs on a timer. Reading memory here could write `unreported` after a
/// `completed` the agent had already written — and the last line wins.
pub fn close_run(state: &mut AppState, session_id: Uuid, status: RunStatus) {
    let Some(link) = state.get_session(session_id).and_then(|s| s.job.clone()) else {
        return;
    };
    let Some(workspace_id) = state.workspace_id_for_session(session_id) else {
        return;
    };
    let Some(root) = state.get_workspace(workspace_id).map(|w| w.path.clone()) else {
        return;
    };
    let history_dir = state
        .system
        .project_jobs
        .get(&workspace_id)
        .map(|p| p.history_dir())
        .or_else(|| jobs::load(&root).map(|p| p.history_dir()))
        .unwrap_or_else(|| PathBuf::from(jobs::manifest::DEFAULT_HISTORY_DIR));
    let Some(latest) = jobs::history::find(&root, &history_dir, &link.job_id, &link.run_id) else {
        return;
    };
    if !latest.status.is_open() {
        return;
    }
    let closed = jobs::close_record(&latest, status);
    match jobs::history::append(&root, &history_dir, &closed) {
        Ok(_) => {
            if let Some(runs) = state
                .system
                .project_jobs
                .get_mut(&workspace_id)
                .and_then(|p| p.runs.get_mut(&link.job_id))
            {
                if let Some(run) = runs.iter_mut().find(|r| r.run_id == link.run_id) {
                    *run = closed;
                }
            }
            crate::logger::info(format!(
                "job run {} closed as {}",
                link.run_id,
                status.label()
            ));
        }
        Err(error) => crate::logger::warn(format!(
            "could not close job run {}: {error:#}",
            link.run_id
        )),
    }
}

fn toast(state: &mut AppState, message: String) {
    state.ui.toasts.push_back(super::Toast::new(
        message,
        ToastLevel::Info,
        std::time::Duration::from_secs(3),
    ));
    while state.ui.toasts.len() > 5 {
        state.ui.toasts.pop_front();
    }
}

/// Roots and stamps for the scanner: every workspace, with what it last saw.
pub fn scan_inputs(state: &AppState) -> Vec<(Uuid, PathBuf, Option<Stamp>)> {
    state
        .data
        .workspaces
        .iter()
        .filter(|w| !w.global)
        .map(|w| {
            (
                w.id,
                w.path.clone(),
                state
                    .system
                    .project_jobs
                    .get(&w.id)
                    .map(|p| p.stamp.clone()),
            )
        })
        .collect()
}

/// The jobs each project has, for the phone and the CLI.
pub fn by_project(state: &AppState) -> HashMap<Uuid, Vec<Row>> {
    let mut map: HashMap<Uuid, Vec<Row>> = HashMap::new();
    for row in all_rows(state) {
        map.entry(row.key.workspace).or_default().push(row);
    }
    map
}
