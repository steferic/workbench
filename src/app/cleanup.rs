//! Shared lifecycle cleanup for individual sessions, workspaces, and shutdown.
use super::{Action, AppState, ToastLevel};
use crate::pty::PtyHandle;
use anyhow::Result;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

/// Keep every cleanup job until it finishes. Drop also joins, so unwinding
/// cannot discard the threads that own our last process handles.
#[derive(Default)]
pub struct CleanupJobs(Vec<CleanupJob>);

struct CleanupJob {
    sessions: HashSet<Uuid>,
    thread: std::thread::JoinHandle<Result<()>>,
}

impl CleanupJob {
    fn join(self) -> Result<()> {
        self.thread
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("session cleanup thread panicked")))
    }
}

impl CleanupJobs {
    pub fn spawn(
        &mut self,
        sessions: HashSet<Uuid>,
        job: impl FnOnce() -> Result<()> + Send + 'static,
    ) {
        let mut pending = Vec::new();
        for job in self.0.drain(..) {
            if job.thread.is_finished() {
                if let Err(err) = job.join() {
                    crate::logger::warn(format!("session cleanup failed: {err}"));
                }
            } else {
                pending.push(job);
            }
        }
        self.0 = pending;
        self.0.push(CleanupJob {
            sessions,
            thread: std::thread::spawn(job),
        });
    }

    fn take_for(&mut self, sessions: &HashSet<Uuid>) -> Vec<CleanupJob> {
        let (matching, remaining) = self
            .0
            .drain(..)
            .partition(|job| !job.sessions.is_disjoint(sessions));
        self.0 = remaining;
        matching
    }

    pub fn finish(&mut self) {
        for job in self.0.drain(..) {
            if let Err(err) = job.join() {
                crate::logger::warn(format!("session cleanup failed: {err}"));
            }
        }
    }
}

impl Drop for CleanupJobs {
    fn drop(&mut self) {
        self.finish();
    }
}

fn terminate(mut handle: PtyHandle, terminal: bool) -> Result<()> {
    if terminal {
        handle.interrupt_then_kill(Duration::from_millis(500))
    } else {
        handle.kill()
    }
}

pub fn stop_session(state: &mut AppState, id: Uuid) {
    let terminal = state
        .get_session(id)
        .is_some_and(|s| s.agent_type.is_terminal());
    if let Some(handle) = state.system.pty_handles.remove(&id) {
        state
            .system
            .cleanup_jobs
            .spawn([id].into_iter().collect(), move || {
                terminate(handle, terminal)
            });
    }
}

/// Retain session records and worktrees for resume, but wait for all active
/// and previously deleted sessions to stop before returning to the shell.
pub fn shutdown(state: &mut AppState) {
    state.system.forwarded.clear();
    crate::media::close(state);
    if let Ok(snapshot) = state.system.remote_state.lock() {
        snapshot.media_library.shutdown();
    }
    state.system.startup_queue.clear();
    let ids: Vec<_> = state.system.pty_handles.keys().copied().collect();
    for id in ids {
        stop_session(state, id);
    }
    state.system.cleanup_jobs.finish();
    for session in state.data.sessions.values_mut().flatten() {
        if session.status == crate::models::SessionStatus::Running {
            session.mark_stopped();
        }
        session.todo_queue.requeue_running();
    }
}

#[derive(Default)]
struct Removal {
    prior_jobs: Vec<CleanupJob>,
    status_files: Vec<PathBuf>,
    handles: Vec<(PtyHandle, bool)>,
    worktrees: HashSet<(PathBuf, PathBuf)>,
}

impl Removal {
    fn run(self) -> Result<()> {
        // Stop all handles concurrently; a workspace with many terminals
        // gets one grace period. Worktrees are touched only after they stop.
        let mut failures: Vec<_> = self
            .prior_jobs
            .into_iter()
            .filter_map(|job| job.join().err().map(|err| err.to_string()))
            .collect();
        failures.extend(std::thread::scope(|scope| {
            let jobs: Vec<_> = self
                .handles
                .into_iter()
                .map(|(handle, terminal)| scope.spawn(move || terminate(handle, terminal)))
                .collect();
            jobs.into_iter()
                .filter_map(|job| match job.join() {
                    Ok(Ok(())) => None,
                    Ok(Err(err)) => Some(err.to_string()),
                    Err(_) => Some("session cleanup thread panicked".into()),
                })
                .collect::<Vec<_>>()
        }));
        if !failures.is_empty() {
            anyhow::bail!("{}", failures.join("; "));
        }
        let mut failures = Vec::new();
        for path in self.status_files {
            if let Err(err) = std::fs::remove_file(&path) {
                if err.kind() != std::io::ErrorKind::NotFound {
                    failures.push(format!("{}: {err}", path.display()));
                }
            }
        }
        for (repo, tree) in self.worktrees {
            if let Err(err) = crate::git::remove_worktree(&repo, &tree, true) {
                failures.push(format!("{}: {err}", tree.display()));
            }
        }
        if !failures.is_empty() {
            anyhow::bail!("{}", failures.join("; "));
        }
        Ok(())
    }
}

/// Plan from the owning workspace before removing any records. All deletion
/// paths share this so pins, queues, hooks, and worktrees cannot be skipped.
fn plan_removal(state: &mut AppState, ids: &[Uuid]) -> (Removal, HashSet<Uuid>) {
    let mut ids: HashSet<_> = ids.iter().copied().collect();
    // Viewing terminals share the owner's worktree. Close them with the
    // owner; deleting only a viewer must never remove the owner's files.
    for session in state.data.sessions.values().flatten() {
        if session
            .worktree_viewer_for
            .is_some_and(|owner| ids.contains(&owner))
        {
            ids.insert(session.id);
        }
    }
    let mut removal = Removal {
        prior_jobs: state.system.cleanup_jobs.take_for(&ids),
        ..Default::default()
    };
    let mut workspaces = HashSet::new();
    // Attempts can outlive a failed spawn, so their worktrees must not depend
    // on a session record still being present.
    for ws in &state.data.workspaces {
        for attempt in ws.parallel_tasks.iter().flat_map(|task| &task.attempts) {
            if ids.contains(&attempt.session_id) {
                removal
                    .worktrees
                    .insert((ws.path.clone(), attempt.worktree_path.clone()));
            }
        }
    }
    for id in ids {
        let Some(session) = state.get_session(id).cloned() else {
            continue;
        };
        workspaces.insert(session.workspace_id);
        if session.worktree_viewer_for.is_none() {
            if let Some(ws) = state.get_workspace(session.workspace_id) {
                let path = session.worktree_path.clone().or_else(|| {
                    ws.parallel_tasks
                        .iter()
                        .flat_map(|task| &task.attempts)
                        .find(|attempt| attempt.session_id == id)
                        .map(|attempt| attempt.worktree_path.clone())
                });
                if let Some(path) = path {
                    removal.worktrees.insert((ws.path.clone(), path));
                }
            }
        }
        if let Some(handle) = state.system.pty_handles.remove(&id) {
            removal
                .handles
                .push((handle, session.agent_type.is_terminal()));
        }
        if let Ok(path) =
            crate::agent_status::status_path(&session.workspace_id.to_string(), &session.short_id())
        {
            removal.status_files.push(path);
        }
        state.delete_session(id);
    }
    (removal, workspaces)
}

pub fn remove_sessions(state: &mut AppState, ids: &[Uuid], tx: &mpsc::UnboundedSender<Action>) {
    let (removal, workspaces) = plan_removal(state, ids);
    crate::media::tick(state);
    let tx = tx.clone();
    state
        .system
        .cleanup_jobs
        .spawn(ids.iter().copied().collect(), move || {
            let result = removal.run();
            if let Err(err) = &result {
                crate::logger::warn(format!("failed to clean up deleted sessions: {err}"));
                let _ = tx.send(Action::ShowToast(
                    format!("Session cleanup failed: {err}"),
                    ToastLevel::Error,
                ));
            }
            result
        });
    for id in workspaces {
        super::comms_tick::publish_workspace_roster(state, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AgentType, ParallelTask, ParallelTaskAttempt, Session, Workspace};
    use std::path::Path;
    use std::process::Command;
    use std::time::Instant;

    fn git(repo: &Path, args: &[&str]) -> String {
        let result = Command::new("git")
            .current_dir(repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8_lossy(&result.stdout).into_owned()
    }

    #[test]
    fn removal_cleans_the_owning_worktree_hooks_and_viewers() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let tree = temp.path().join("tree");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(
            &repo,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-qm",
                "initial",
            ],
        );
        crate::git::create_worktree(&repo, "cleanup-test", &tree).unwrap();
        let mut state = AppState::default();
        // Deliberately select another project. The owner, not selection,
        // determines which repository is allowed to lose a worktree.
        state
            .data
            .workspaces
            .push(Workspace::new("other".into(), temp.path().join("other")));
        let ws = Workspace::new("owner".into(), repo.clone());
        let mut owner = Session::new(ws.id, AgentType::Claude, false);
        owner.worktree_path = Some(tree.clone());
        let mut viewer = Session::new(ws.id, AgentType::Terminal("viewer".into()), false);
        viewer.worktree_path = Some(tree.clone());
        viewer.worktree_viewer_for = Some(owner.id);
        let owner_id = owner.id;
        let viewer_id = viewer.id;
        state.data.workspaces.push(ws);
        state.add_session(owner);
        state.add_session(viewer);
        state.data.last_activity.insert(owner_id, Instant::now());
        state.data.idle_queue.push(owner_id);
        let (mut removal, _) = plan_removal(&mut state, &[owner_id]);
        assert!(state.get_session(owner_id).is_none());
        assert!(state.get_session(viewer_id).is_none());
        assert!(!state.data.last_activity.contains_key(&owner_id));
        assert!(!state.data.idle_queue.contains(&owner_id));
        assert_eq!(removal.status_files.len(), 2);
        assert_eq!(removal.worktrees.len(), 1);
        // Redirect just the fixture's hook files into its temporary directory.
        let hook = temp.path().join("status.json");
        std::fs::write(&hook, b"{}").unwrap();
        removal.status_files = vec![hook.clone()];
        removal.run().unwrap();
        assert!(!tree.exists());
        assert!(!hook.exists());
        assert!(git(&repo, &["branch", "--list", "cleanup-test"])
            .trim()
            .is_empty());
        assert!(repo.join(".git").exists());
    }

    #[test]
    fn removing_a_viewer_keeps_its_owners_worktree() {
        let mut state = AppState::default();
        let ws = Workspace::new("owner".into(), PathBuf::from("/unused"));
        let owner = Session::new(ws.id, AgentType::Claude, false);
        let mut viewer = Session::new(ws.id, AgentType::Terminal("viewer".into()), false);
        viewer.worktree_viewer_for = Some(owner.id);
        viewer.worktree_path = Some(PathBuf::from("/unused/tree"));
        let (owner_id, viewer_id) = (owner.id, viewer.id);
        state.data.workspaces.push(ws);
        state.add_session(owner);
        state.add_session(viewer);
        let (removal, _) = plan_removal(&mut state, &[viewer_id]);
        assert!(removal.worktrees.is_empty());
        assert!(state.get_session(owner_id).is_some());
    }

    #[test]
    fn an_attempt_without_a_session_still_has_its_worktree_removed() {
        let mut state = AppState::default();
        let mut ws = Workspace::new("owner".into(), PathBuf::from("/unused"));
        let mut task = ParallelTask::new(ws.id, "test".into(), "main".into(), "head".into(), false);
        let id = Uuid::new_v4();
        task.add_attempt(ParallelTaskAttempt::new(
            task.id,
            id,
            AgentType::Claude,
            "branch".into(),
            PathBuf::from("/unused/tree"),
        ));
        ws.parallel_tasks.push(task);
        state.data.workspaces.push(ws);
        let (removal, _) = plan_removal(&mut state, &[id]);
        assert_eq!(removal.worktrees.len(), 1);
    }
    #[test]
    fn deleting_a_previously_stopped_session_waits_for_its_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let hook = temp.path().join("status.json");
        std::fs::write(&hook, b"{}").unwrap();
        let mut state = AppState::default();
        let id = Uuid::new_v4();
        let (release, wait) = std::sync::mpsc::channel();
        state
            .system
            .cleanup_jobs
            .spawn([id].into_iter().collect(), move || {
                wait.recv().unwrap();
                Ok(())
            });
        let (mut removal, _) = plan_removal(&mut state, &[id]);
        assert_eq!(removal.prior_jobs.len(), 1);
        removal.status_files.push(hook.clone());
        let (finished, done) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            finished.send(removal.run()).unwrap();
        });
        let still_waiting = done.recv_timeout(Duration::from_millis(50)).is_err();
        let still_exists = hook.exists();
        release.send(()).unwrap();
        worker.join().unwrap();
        assert!(
            still_waiting && still_exists,
            "files were removed before process cleanup finished"
        );
        done.recv().unwrap().unwrap();
        assert!(!hook.exists());
    }
}
