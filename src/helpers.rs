use crate::app::{Action, AppState};
use crate::models::{AgentType, JobLink, SessionStatus, TodoState};
use crate::pty::PtyManager;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use uuid::Uuid;

pub const ENV_HELPER: &str = "WORKBENCH_HELPER_RUN";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Queued,
    Running,
    Completed,
    Failed,
    Unreported,
    Aborted,
}

impl Status {
    fn finished(self) -> bool {
        !matches!(self, Self::Queued | Self::Running)
    }
}

#[derive(Serialize, Deserialize)]
struct Run {
    id: Uuid,
    parent: String,
    workspace: Uuid,
    provider: AgentType,
    prompt: String,
    status: Status,
    session: Option<Uuid>,
    parent_task: Option<Uuid>,
    parent_job: Option<JobLink>,
    worktree: Option<PathBuf>,
    todo: Option<Uuid>,
    summary: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Report {
    success: bool,
    summary: String,
}

fn directory() -> Result<PathBuf> {
    Ok(crate::comms::comms_root()?.join("helpers"))
}

fn path(dir: &Path, id: Uuid, tag: &str) -> PathBuf {
    dir.join(format!("{id}.{tag}"))
}

fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

fn read_run(dir: &Path, id: Uuid) -> Result<Run> {
    let waiting = path(&dir.join("pending"), id, "json");
    let claimed = path(dir, id, "json");
    match std::fs::read(&waiting) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => read(&claimed),
        Err(error) => Err(error.into()),
    }
}

fn write(path: &Path, value: &impl Serialize, create: bool) -> Result<()> {
    use std::io::Write;
    let mut temp =
        tempfile::NamedTempFile::new_in(path.parent().context("missing helper directory")?)?;
    serde_json::to_writer(&mut temp, value)?;
    temp.flush()?;
    if create {
        temp.persist_noclobber(path)?;
    } else {
        temp.persist(path)?;
    }
    Ok(())
}

#[derive(clap::Subcommand)]
pub enum Command {
    /// Start a separate Claude or Codex session in the current project
    Start {
        #[arg(value_parser = ["claude", "codex"])]
        provider: String,
        prompt: String,
    },
    /// Print a helper's result and its parent task or job links
    Status { run: Uuid },
    /// Wait for a specific run's final result
    Wait {
        run: Uuid,
        #[arg(long, default_value_t = 600)]
        timeout: u64,
    },
    /// Store the result from inside the helper session
    Report {
        summary: String,
        #[arg(long)]
        failed: bool,
    },
}

pub fn cli(command: Command) -> Result<()> {
    let dir = directory()?;
    match command {
        Command::Start { provider, prompt } => {
            let parent = std::env::var(crate::comms::ENV_SESSION)
                .context("start a helper from inside a Workbench agent")?;
            if std::env::var_os(ENV_HELPER).is_some() {
                bail!("a helper run cannot launch another helper");
            }
            if prompt.trim().is_empty() {
                bail!("provide a helper prompt");
            }
            let mut client = crate::control::Client::connect()?;
            client.set_timeout(Some(Duration::from_secs(3)))?;
            let agent = client.call("agent.get", serde_json::json!({"agent": parent}))?;
            if agent["manager"] == true || agent["status"] == "stopped" {
                bail!("the current session cannot dispatch a helper");
            }
            let workspace = agent["project_id"]
                .as_str()
                .context("parent has no workspace")?
                .parse()?;
            let provider = if provider == "claude" {
                AgentType::Claude
            } else {
                AgentType::Codex
            };
            let id = Uuid::new_v4();
            let run = Run {
                id,
                parent,
                workspace,
                provider,
                prompt,
                status: Status::Queued,
                session: None,
                todo: None,
                parent_task: None,
                parent_job: None,
                worktree: None,
                summary: None,
            };
            std::fs::create_dir_all(dir.join("pending"))?;
            write(&path(&dir.join("pending"), id, "json"), &run, true)?;
            println!("{id}");
        }
        Command::Status { run } => {
            let record = read_run(&dir, run)?;
            println!("{}", serde_json::to_string_pretty(&record)?);
        }
        Command::Wait { run, timeout } => {
            let started = Instant::now();
            loop {
                let record = read_run(&dir, run)?;
                if record.status.finished() {
                    println!("{}", serde_json::to_string_pretty(&record)?);
                    if record.status != Status::Completed {
                        bail!("helper run ended with status {:?}", record.status);
                    }
                    break;
                }
                if started.elapsed() >= Duration::from_secs(timeout) {
                    bail!("timed out waiting for helper {run}");
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }
        Command::Report { summary, failed } => {
            if summary.trim().is_empty() {
                bail!("provide a summary");
            }
            let id = std::env::var(ENV_HELPER)
                .context("report must run inside a Workbench helper")?
                .parse()?;
            let record = read_run(&dir, id)?;
            let caller = std::env::var(crate::comms::ENV_SESSION)?;
            if record.status != Status::Running
                || record
                    .session
                    .map(crate::models::Session::short_id_of)
                    .as_deref()
                    != Some(&caller)
            {
                bail!("this session does not own a running helper");
            }
            write(
                &path(&dir, id, "report"),
                &Report {
                    success: !failed,
                    summary,
                },
                true,
            )?;
        }
    }
    Ok(())
}

#[derive(Default)]
pub struct Runtime {
    dir: Option<PathBuf>,
    active: HashMap<Uuid, Run>,
    last_tick: Option<Instant>,
}

impl Runtime {
    pub fn start() -> Self {
        let mut runtime = Self {
            dir: directory().ok(),
            ..Self::default()
        };
        runtime.recover();
        runtime
    }

    pub fn recover(&mut self) {
        let Some(dir) = self.dir.as_deref() else {
            return;
        };
        if let Ok(entries) = std::fs::read_dir(dir.join("pending")) {
            for entry in entries.flatten() {
                if entry.path().extension().is_some_and(|ext| ext == "json") {
                    let _ = std::fs::rename(entry.path(), dir.join(entry.file_name()));
                }
            }
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            if entry.path().extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let Ok(mut run) = read::<Run>(&entry.path()) else {
                continue;
            };
            match run.status {
                Status::Running => {
                    if let Ok(report) = read::<Report>(&path(dir, run.id, "report")) {
                        let status = if report.success {
                            Status::Completed
                        } else {
                            Status::Failed
                        };
                        finish(dir, &mut run, status, &report.summary);
                    } else {
                        finish(
                            dir,
                            &mut run,
                            Status::Aborted,
                            "Workbench exited before the helper reported",
                        );
                    }
                }
                Status::Queued => {
                    finish(
                        dir,
                        &mut run,
                        Status::Aborted,
                        "Workbench exited while claiming the helper",
                    );
                }
                _ => {}
            }
        }
    }

    pub fn tick(state: &mut AppState, manager: &PtyManager, pty_tx: &mpsc::Sender<Action>) {
        let tracker = &mut state.system.helpers;
        if tracker
            .last_tick
            .is_some_and(|last| last.elapsed() < Duration::from_millis(400))
        {
            return;
        }
        tracker.last_tick = Some(Instant::now());
        let Some(dir) = tracker.dir.clone() else {
            return;
        };
        let mut active = std::mem::take(&mut tracker.active);
        active.retain(|_, run| {
            if let Ok(report) = read::<Report>(&path(&dir, run.id, "report")) {
                let status = if report.success {
                    Status::Completed
                } else {
                    Status::Failed
                };
                return !finish(&dir, run, status, &report.summary);
            }
            let session = run.session.and_then(|id| state.get_session(id));
            match session {
                Some(session) if session.status == SessionStatus::Running => {
                    let todo = session
                        .todo_queue
                        .items
                        .iter()
                        .find(|item| Some(item.id) == run.todo);
                    match todo.map(|item| item.state) {
                        Some(TodoState::Done) => !finish(
                            &dir,
                            run,
                            Status::Unreported,
                            "The helper turn ended without a report",
                        ),
                        None => !finish(&dir, run, Status::Aborted, "The helper task was removed"),
                        _ => true,
                    }
                }
                _ => !finish(&dir, run, Status::Aborted, "The helper session stopped"),
            }
        });
        if let Ok(requests) = std::fs::read_dir(dir.join("pending")) {
            for entry in requests
                .flatten()
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            {
                let Ok(mut run) = read::<Run>(&entry.path()) else {
                    continue;
                };
                // Claim before spawning. Retrying a failed cleanup never
                // launches a second helper with the same ID.
                if std::fs::rename(entry.path(), path(&dir, run.id, "json")).is_err() {
                    continue;
                }
                if active.values().any(|other| other.parent == run.parent) {
                    finish(
                        &dir,
                        &mut run,
                        Status::Aborted,
                        "This agent already has a running helper",
                    );
                    continue;
                }
                if let Err(error) = start(state, manager, pty_tx, &dir, &mut run) {
                    finish(&dir, &mut run, Status::Aborted, &error.to_string());
                    continue;
                }
                active.insert(run.id, run);
            }
        }
        state.system.helpers.active = active;
    }
}

fn start(
    state: &mut AppState,
    manager: &PtyManager,
    pty_tx: &mpsc::Sender<Action>,
    dir: &Path,
    run: &mut Run,
) -> Result<()> {
    let parent = crate::remote::session_for(state, &run.parent)
        .and_then(|id| state.get_session(id))
        .filter(|s| {
            s.workspace_id == run.workspace
                && s.status == SessionStatus::Running
                && !s.agent_type.is_manager()
                && s.helper_run.is_none()
        })
        .context("the parent session is no longer available")?;
    run.parent_task = parent.todo_queue.running().map(|todo| todo.id);
    run.parent_job = parent.job.clone();
    run.worktree = parent.worktree_path.clone();
    let session = crate::app::handlers::session::spawn_in_workspace(
        state,
        run.workspace,
        run.provider.clone(),
        false,
        vec![(ENV_HELPER.into(), run.id.to_string())],
        manager,
        pty_tx,
        Some((run.id, run.worktree.as_deref())),
    )
    .context("could not start the helper session")?;
    run.session = Some(session);
    run.status = Status::Running;
    let prompt = format!(
        "{}\n\nWhen finished, record your result with `workbench helper report \"your summary\"`. Add `--failed` if the requested work could not be completed.",
        run.prompt
    );
    let helper = state
        .get_session_mut(session)
        .context("helper session disappeared")?;
    helper.alias = Some(format!("helper-{}", &run.id.to_string()[..8]));
    run.todo = Some(helper.todo_queue.add(prompt));
    if let Err(error) = write(&path(dir, run.id, "json"), run, false) {
        crate::app::cleanup::stop_session(state, session);
        if let Some(helper) = state.get_session_mut(session) {
            helper.todo_queue = Default::default();
            helper.mark_stopped();
        }
        return Err(error);
    }
    crate::app::handlers::save_state(state, "failed to save the helper session");
    Ok(())
}

fn finish(dir: &Path, run: &mut Run, status: Status, summary: &str) -> bool {
    run.status = status;
    run.summary = Some(summary.to_string());
    match write(&path(dir, run.id, "json"), run, false) {
        Ok(()) => true,
        Err(err) => {
            crate::logger::warn(format!("could not record helper {}: {err}", run.id));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_survives_runtime_recovery_with_the_original_links() {
        let dir = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4();
        let task = Uuid::new_v4();
        let record = Run {
            id,
            parent: "parent".into(),
            workspace: Uuid::new_v4(),
            provider: AgentType::Codex,
            prompt: "Review the diff".into(),
            status: Status::Running,
            session: Some(Uuid::new_v4()),
            parent_task: Some(task),
            parent_job: Some(JobLink {
                job_id: "review".into(),
                run_id: "original".into(),
            }),
            worktree: None,
            todo: Some(Uuid::new_v4()),
            summary: None,
        };
        write(&path(dir.path(), id, "json"), &record, true).unwrap();
        write(
            &path(dir.path(), id, "report"),
            &Report {
                success: true,
                summary: "Two findings".into(),
            },
            true,
        )
        .unwrap();
        let mut runtime = Runtime {
            dir: Some(dir.path().to_path_buf()),
            ..Runtime::default()
        };
        runtime.recover();
        let result = read_run(dir.path(), id).unwrap();
        assert_eq!(result.status, Status::Completed);
        assert_eq!(result.parent_task, Some(task));
        assert_eq!(result.parent_job, record.parent_job);
        assert_eq!(result.summary.as_deref(), Some("Two findings"));
    }
}
