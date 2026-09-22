//! The run ledger: one JSONL file per job, append-only, committed to git.
//!
//! Every line is a whole record; readers fold lines by run id and the last
//! wins. Nothing is edited in place, so `merge=union` lets two machines append
//! at once without a conflict — the same trick the projects this serves
//! already use for their own shared history files.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    /// Started and not yet reported.
    Running,
    // What the agent reports, in the vocabulary the job contracts use.
    Completed,
    Partial,
    Blocked,
    Failed,
    /// Its turn ended and no report came.
    Unreported,
    /// The session died or was deleted with the run still open.
    Aborted,
}

impl RunStatus {
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text.trim().to_lowercase().as_str() {
            "running" => Self::Running,
            "completed" | "complete" | "done" => Self::Completed,
            "partial" => Self::Partial,
            "blocked" => Self::Blocked,
            "failed" | "failure" => Self::Failed,
            "unreported" => Self::Unreported,
            "aborted" => Self::Aborted,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Partial => "partial",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
            Self::Unreported => "unreported",
            Self::Aborted => "aborted",
        }
    }

    pub fn is_open(self) -> bool {
        self == Self::Running
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub user: String,
    pub host: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    #[serde(default = "one")]
    pub schema_version: u32,
    pub run_id: String,
    pub job_id: String,
    pub status: RunStatus,
    pub started_utc: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub ended_utc: Option<chrono::DateTime<chrono::Utc>>,
    pub by: Actor,
    #[serde(default)]
    pub agent: String,
    /// The workbench session that ran it, when one did.
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub instructions_sha256_16: BTreeMap<String, String>,
    #[serde(default)]
    pub summary: Option<String>,
    /// Where the project's own artifacts for this run are, relative to the root.
    #[serde(default)]
    pub artifacts: Option<String>,
    #[serde(default)]
    pub lessons: Vec<String>,
}

fn one() -> u32 {
    1
}

pub fn path(root: &Path, history_dir: &Path, job_id: &str) -> PathBuf {
    root.join(history_dir).join(format!("{job_id}.jsonl"))
}

/// Every run of a job, folded, oldest start first. A line that will not
/// parse is skipped: one bad merge must not hide the rest of the history.
pub fn read(root: &Path, history_dir: &Path, job_id: &str) -> Vec<RunRecord> {
    let Ok(text) = std::fs::read_to_string(path(root, history_dir, job_id)) else {
        return Vec::new();
    };
    fold(&text)
}

pub fn fold(text: &str) -> Vec<RunRecord> {
    let mut by_id: BTreeMap<String, RunRecord> = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<RunRecord>(line) {
            by_id.insert(record.run_id.clone(), record);
        }
    }
    let mut runs: Vec<RunRecord> = by_id.into_values().collect();
    runs.sort_by(|a, b| {
        a.started_utc
            .cmp(&b.started_utc)
            .then(a.run_id.cmp(&b.run_id))
    });
    runs
}

/// Append one line. Creates the directory and file on first use.
pub fn append(root: &Path, history_dir: &Path, record: &RunRecord) -> anyhow::Result<PathBuf> {
    let path = path(root, history_dir, &record.job_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    let mut line = serde_json::to_string(record)?;
    line.push('\n');
    file.write_all(line.as_bytes())?;
    Ok(path)
}

/// The latest line for one run, if the ledger has it.
pub fn find(root: &Path, history_dir: &Path, job_id: &str, run_id: &str) -> Option<RunRecord> {
    read(root, history_dir, job_id)
        .into_iter()
        .find(|record| record.run_id == run_id)
}

/// Append a dated bullet to the job's lessons file, creating it with a
/// heading. One line per lesson keeps the file mergeable the same way.
pub fn append_lesson(root: &Path, job_id: &str, lesson: &str) -> anyhow::Result<PathBuf> {
    let path = super::lessons_path(root, job_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let fresh = !path.exists();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    if fresh {
        writeln!(
            file,
            "# Lessons: {job_id}\n\nOne dated line per lesson, appended by `workbench jobs report --lesson`.\nEvery run reads this first.\n"
        )?;
    }
    let one_line: String = lesson.split_whitespace().collect::<Vec<_>>().join(" ");
    writeln!(
        file,
        "- {}: {one_line}",
        chrono::Utc::now().format("%Y-%m-%d")
    )?;
    Ok(path)
}
