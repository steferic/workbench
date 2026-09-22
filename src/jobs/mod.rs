//! Repeatable agent jobs a project declares in its own repository.
//!
//! A project opts in by committing `.workbench/jobs.toml`. Workbench reads it
//! to list the jobs in its own window (F4), sends a job's prompt to a fresh agent when asked, and
//! records every run as one JSON line in a ledger that is also committed —
//! so a teammate who pulls the repo sees what ran, when, by whom and how it
//! went, with no workbench of their own required.
//!
//! This module is pure file I/O and types: nothing here touches `AppState`,
//! which is what lets the `workbench jobs` CLI use it with no TUI running.
//! The window and its actions live in `app::jobs`.
//!
//! Two rules from the projects this was built for shape the design. The
//! ledger holds metadata only — who, when, which instruction versions, a
//! summary, a path — never captures or customer data, so it is safe to
//! commit. And nothing here ever runs a job or rewrites its instructions on
//! its own: `every` marks a row due, and a human presses Enter.

pub mod history;
pub mod manifest;
pub mod prompt;

pub use history::{Actor, RunRecord, RunStatus};
pub use manifest::{JobDef, Manifest, PromptSource};

use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests;

/// The manifest, relative to the repository root.
pub const MANIFEST: &str = ".workbench/jobs.toml";
/// Where the harness footer tells an agent to look for what earlier runs learned.
pub const LESSONS_DIR: &str = ".workbench/jobs/lessons";
/// Injected into a job's session so `workbench jobs report` needs no arguments.
pub const ENV_JOB: &str = "WORKBENCH_JOB";
pub const ENV_RUN: &str = "WORKBENCH_JOB_RUN";

/// Everything the Jobs tab shows for one project, read from its files.
#[derive(Debug, Clone, Default)]
pub struct ProjectJobs {
    /// The parsed manifest, or the one-line reason it could not be.
    pub manifest: Option<Result<Manifest, String>>,
    /// Folded run records per job id, oldest first.
    pub runs: HashMap<String, Vec<RunRecord>>,
    /// The lessons file per job id, when one exists.
    pub lessons: HashMap<String, String>,
    /// Each job's instruction hashes as the files are now, to set against
    /// the hashes its last run recorded.
    pub hashes: HashMap<String, BTreeMap<String, String>>,
    /// Each job's prompt body as it would be sent now (a `prompt_file`
    /// already read), or why it could not be.
    pub prompts: HashMap<String, Result<String, String>>,
    /// What the files looked like when read, so a rescan can skip a project
    /// nothing has touched.
    pub stamp: Stamp,
}

/// Modification times of the files a project's jobs live in: the manifest,
/// the ledgers, the lessons, and every instruction and prompt file the
/// manifest names — an edit to any of those is what the window reports.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stamp {
    pub history_dir: PathBuf,
    pub manifest: Option<std::time::SystemTime>,
    pub history: Vec<(PathBuf, std::time::SystemTime)>,
    pub lessons: Vec<(PathBuf, std::time::SystemTime)>,
    pub watched: Vec<(PathBuf, Option<std::time::SystemTime>)>,
}

impl Stamp {
    /// The same files, as they are now.
    pub fn refresh(&self, root: &Path) -> Stamp {
        let watched: Vec<PathBuf> = self.watched.iter().map(|(p, _)| p.clone()).collect();
        stamp(root, &self.history_dir, &watched)
    }
}

impl ProjectJobs {
    pub fn jobs(&self) -> &[JobDef] {
        match &self.manifest {
            Some(Ok(manifest)) => &manifest.jobs,
            _ => &[],
        }
    }

    pub fn job(&self, id: &str) -> Option<&JobDef> {
        self.jobs().iter().find(|job| job.id == id)
    }

    pub fn history_dir(&self) -> PathBuf {
        match &self.manifest {
            Some(Ok(manifest)) => manifest.history_dir.clone(),
            _ => PathBuf::from(manifest::DEFAULT_HISTORY_DIR),
        }
    }

    /// The latest record for a job, by start time.
    pub fn last_run(&self, id: &str) -> Option<&RunRecord> {
        self.runs.get(id).and_then(|runs| runs.last())
    }

    /// A job is due when its cadence has elapsed since its last run began —
    /// or has never run at all. On-demand jobs are never due.
    pub fn due(&self, job: &JobDef, now: chrono::DateTime<chrono::Utc>) -> bool {
        let Some(every) = job.every else {
            return false;
        };
        match self.last_run(&job.id) {
            None => true,
            Some(last) => {
                let elapsed = now - last.started_utc;
                elapsed.to_std().map(|e| e >= every).unwrap_or(false)
            }
        }
    }
}

/// The directory whose manifest governs `from`: `from` itself or the nearest
/// ancestor holding one. This is how the CLI finds the project from a plain
/// shell, so it walks the way `git` does.
pub fn find_root(from: &Path) -> Option<PathBuf> {
    let mut dir = Some(from);
    while let Some(candidate) = dir {
        if candidate.join(MANIFEST).is_file() {
            return Some(candidate.to_path_buf());
        }
        dir = candidate.parent();
    }
    None
}

/// What the files currently say. `None` when the project has no manifest.
pub fn load(root: &Path) -> Option<ProjectJobs> {
    let manifest_path = root.join(MANIFEST);
    let text = std::fs::read_to_string(&manifest_path).ok()?;
    let manifest = manifest::parse(&text);
    let history_dir = manifest
        .as_ref()
        .map(|m| m.history_dir.clone())
        .unwrap_or_else(|_| PathBuf::from(manifest::DEFAULT_HISTORY_DIR));
    let mut project = ProjectJobs {
        manifest: Some(manifest),
        ..Default::default()
    };
    for job in project.jobs().to_vec() {
        let runs = history::read(root, &history_dir, &job.id);
        if !runs.is_empty() {
            project.runs.insert(job.id.clone(), runs);
        }
        if let Ok(text) = std::fs::read_to_string(lessons_path(root, &job.id)) {
            if !text.trim().is_empty() {
                project.lessons.insert(job.id.clone(), text);
            }
        }
        project
            .hashes
            .insert(job.id.clone(), instruction_hashes(root, &job));
        project.prompts.insert(
            job.id.clone(),
            prompt::body(root, &job).map_err(|e| format!("{e:#}")),
        );
    }
    let mut watched: Vec<PathBuf> = Vec::new();
    for job in project.jobs() {
        for path in &job.instructions {
            if !watched.contains(path) {
                watched.push(path.clone());
            }
        }
        if let PromptSource::File(path) = &job.prompt {
            if !watched.contains(path) {
                watched.push(path.clone());
            }
        }
    }
    project.stamp = stamp(root, &history_dir, &watched);
    Some(project)
}

/// Modification times of everything `load` reads, cheap enough for a timer.
pub fn stamp(root: &Path, history_dir: &Path, watched: &[PathBuf]) -> Stamp {
    let mtime = |path: &Path| std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let listing = |dir: &Path| {
        let mut entries: Vec<(PathBuf, std::time::SystemTime)> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                mtime(&path).map(|at| (path, at))
            })
            .collect();
        entries.sort();
        entries
    };
    Stamp {
        history_dir: history_dir.to_path_buf(),
        manifest: mtime(&root.join(MANIFEST)),
        history: listing(&root.join(history_dir)),
        lessons: listing(&root.join(LESSONS_DIR)),
        watched: watched
            .iter()
            .map(|path| (path.clone(), mtime(&root.join(path))))
            .collect(),
    }
}

pub fn lessons_path(root: &Path, job_id: &str) -> PathBuf {
    root.join(LESSONS_DIR).join(format!("{job_id}.md"))
}

/// `YYYYMMDDTHHMMSSZ-xxxx`: sortable, and unique enough across two machines
/// starting the same job in the same second.
pub fn new_run_id() -> String {
    use rand::Rng;
    let suffix: u16 = rand::thread_rng().gen();
    format!(
        "{}-{suffix:04x}",
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
    )
}

/// Who is running this: the git identity if the repo has one, else the
/// login, plus the machine — the two things a teammate reading the ledger
/// wants to know.
pub fn actor(root: &Path) -> Actor {
    let git_name = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "user.name"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|name| !name.is_empty());
    let user = git_name
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "unknown".into());
    let host = std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|host| !host.is_empty())
        .unwrap_or_else(|| "unknown".into());
    Actor { user, host }
}

/// The first 16 hex characters of each instruction file's SHA-256, keyed by
/// the path as the manifest wrote it. A missing file says so rather than
/// vanishing: a run that read nothing is worth telling apart later.
pub fn instruction_hashes(root: &Path, job: &JobDef) -> BTreeMap<String, String> {
    use sha2::{Digest, Sha256};
    let mut hashes = BTreeMap::new();
    let mut files: Vec<PathBuf> = job.instructions.clone();
    if let PromptSource::File(path) = &job.prompt {
        files.push(path.clone());
    }
    for path in files {
        let key = path.display().to_string();
        let digest = std::fs::read(root.join(&path))
            .map(|bytes| {
                let hex = format!("{:x}", Sha256::digest(&bytes));
                hex[..16].to_string()
            })
            .unwrap_or_else(|_| "missing".into());
        hashes.insert(key, digest);
    }
    hashes
}

/// What a job's ledger adds up to, for the window's overview.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    pub total: usize,
    pub running: usize,
    pub completed: usize,
    pub partial: usize,
    pub blocked: usize,
    pub failed: usize,
    pub unreported: usize,
    pub aborted: usize,
    /// Completed over every run the agent reported on (completed, partial,
    /// blocked, failed), as a percentage. None until one is reported.
    pub success_pct: Option<u8>,
    /// Mean wall time of reported runs, in seconds.
    pub mean_seconds: Option<u64>,
    pub last_7_days: usize,
    pub people: Vec<String>,
    pub lessons: usize,
}

pub fn stats(runs: &[RunRecord], now: chrono::DateTime<chrono::Utc>) -> Stats {
    let mut s = Stats {
        total: runs.len(),
        ..Default::default()
    };
    let mut durations = Vec::new();
    for run in runs {
        match run.status {
            RunStatus::Running => s.running += 1,
            RunStatus::Completed => s.completed += 1,
            RunStatus::Partial => s.partial += 1,
            RunStatus::Blocked => s.blocked += 1,
            RunStatus::Failed => s.failed += 1,
            RunStatus::Unreported => s.unreported += 1,
            RunStatus::Aborted => s.aborted += 1,
        }
        let reported = matches!(
            run.status,
            RunStatus::Completed | RunStatus::Partial | RunStatus::Blocked | RunStatus::Failed
        );
        if reported {
            if let Some(ended) = run.ended_utc {
                if let Ok(d) = (ended - run.started_utc).to_std() {
                    durations.push(d.as_secs());
                }
            }
        }
        if now - run.started_utc <= chrono::TimeDelta::days(7) {
            s.last_7_days += 1;
        }
        if !s.people.contains(&run.by.user) {
            s.people.push(run.by.user.clone());
        }
        s.lessons += run.lessons.len();
    }
    let reported = s.completed + s.partial + s.blocked + s.failed;
    if reported > 0 {
        s.success_pct = Some(((s.completed * 100) / reported) as u8);
    }
    if !durations.is_empty() {
        s.mean_seconds = Some(durations.iter().sum::<u64>() / durations.len() as u64);
    }
    s
}

/// The record for a run that is starting now.
pub fn start_record(
    root: &Path,
    job: &JobDef,
    run_id: &str,
    agent: &str,
    session: Option<String>,
) -> RunRecord {
    RunRecord {
        schema_version: 1,
        run_id: run_id.to_string(),
        job_id: job.id.clone(),
        status: RunStatus::Running,
        started_utc: chrono::Utc::now(),
        ended_utc: None,
        by: actor(root),
        agent: agent.to_string(),
        session,
        instructions_sha256_16: instruction_hashes(root, job),
        summary: None,
        artifacts: None,
        lessons: Vec::new(),
    }
}

/// Close a run with a later line. The latest record is cloned so nothing
/// recorded at launch is lost; only what changed is written over.
pub fn close_record(latest: &RunRecord, status: RunStatus) -> RunRecord {
    RunRecord {
        status,
        ended_utc: Some(chrono::Utc::now()),
        ..latest.clone()
    }
}

/// The files `init` writes, so its caller can list them.
pub fn init(root: &Path) -> Result<Vec<PathBuf>> {
    let manifest_path = root.join(MANIFEST);
    if manifest_path.exists() {
        bail!("{} already exists", manifest_path.display());
    }
    let mut written = Vec::new();
    let write = |path: PathBuf, text: &str, written: &mut Vec<PathBuf>| -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
        written.push(path);
        Ok(())
    };
    write(manifest_path, manifest::TEMPLATE, &mut written)?;
    write(
        root.join(".workbench/jobs/README.md"),
        prompt::CONTRACT,
        &mut written,
    )?;
    write(
        root.join(manifest::DEFAULT_HISTORY_DIR).join(".gitkeep"),
        "",
        &mut written,
    )?;
    write(root.join(LESSONS_DIR).join(".gitkeep"), "", &mut written)?;

    // Union merge: two machines appending to one ledger must never conflict.
    let attributes = root.join(".gitattributes");
    let line = format!("{}/*.jsonl merge=union", manifest::DEFAULT_HISTORY_DIR);
    let existing = std::fs::read_to_string(&attributes).unwrap_or_default();
    if !existing.lines().any(|l| l.trim() == line) {
        let mut text = existing;
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&line);
        text.push('\n');
        std::fs::write(&attributes, text)?;
        written.push(attributes);
    }
    Ok(written)
}
