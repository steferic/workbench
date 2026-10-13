//! Deleting old transcripts once their words are counted.
//!
//! The transcripts are most of what Claude Code and Codex keep on disk, and
//! after the word count has read them the only reason left to keep one is to
//! resume it. So a session is deleted only when all of these hold:
//!
//! - nothing was written to it for the given number of days;
//! - the scan read all of it (or it is outside your projects, so there is
//!   nothing to count);
//! - no Workbench agent resumes it;
//! - no process has it open.
//!
//! A Claude session is its `<id>.jsonl` together with the `<id>/` folder of
//! sub-agent transcripts and tool results beside it, and the two go together.
//! Nothing else under `~/.claude` or `~/.codex` is touched — not `memory/`,
//! not Codex's own databases.

use super::scan::{claude_projects_dir, codex_sessions_dir, Projects};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// One deletable session: a transcript and whatever belongs to it.
#[derive(Debug)]
pub struct Session {
    pub provider: &'static str,
    pub paths: Vec<PathBuf>,
    pub bytes: u64,
}

#[derive(Debug, Default)]
pub struct Plan {
    pub delete: Vec<Session>,
    /// Old enough, but kept, and why.
    pub kept_resumed: usize,
    pub kept_unread: usize,
    pub kept_open: usize,
}

impl Plan {
    pub fn bytes(&self, provider: &str) -> (usize, u64) {
        let sessions = self.delete.iter().filter(|s| s.provider == provider);
        sessions.fold((0, 0), |(n, b), s| (n + 1, b + s.bytes))
    }
}

/// Work out what may go. Run a scan first, so "read all of it" is current.
pub fn plan(db: &Path, older_than: Duration) -> Result<Plan> {
    let connection = super::open(db)?;
    let offsets: HashMap<PathBuf, u64> = connection
        .prepare("SELECT path, offset FROM files")?
        .query_map([], |row| {
            Ok((
                PathBuf::from(row.get::<_, String>(0)?),
                row.get::<_, i64>(1)? as u64,
            ))
        })?
        .filter_map(Result::ok)
        .collect();
    let context = Context {
        cutoff: SystemTime::now() - older_than,
        offsets,
        resumed: resumed_sessions(),
        open: open_transcripts(),
        projects: Projects::load(),
    };
    Ok(plan_in(
        &claude_projects_dir(),
        &codex_sessions_dir(),
        &context,
    ))
}

/// Delete what the plan names. Returns how many sessions and bytes went.
pub fn delete(plan: &Plan) -> (usize, u64) {
    let (mut sessions, mut bytes) = (0, 0);
    for session in &plan.delete {
        let mut ok = true;
        for path in &session.paths {
            let result = if path.is_dir() {
                std::fs::remove_dir_all(path)
            } else {
                std::fs::remove_file(path)
            };
            if let Err(err) = result {
                crate::logger::warn(format!("prune: {}: {err}", path.display()));
                ok = false;
            }
        }
        if ok {
            sessions += 1;
            bytes += session.bytes;
        }
        // Codex files sessions by `YYYY/MM/DD`; a day, month or year left
        // empty goes too. `remove_dir` refuses a folder that is not empty.
        if session.provider == "openai" {
            for dir in session.paths[0].ancestors().skip(1).take(3) {
                if std::fs::remove_dir(dir).is_err() {
                    break;
                }
            }
        }
    }
    (sessions, bytes)
}

struct Context {
    cutoff: SystemTime,
    offsets: HashMap<PathBuf, u64>,
    /// Session ids and transcript paths Workbench agents resume.
    resumed: HashSet<String>,
    open: HashSet<PathBuf>,
    projects: Projects,
}

fn plan_in(claude: &Path, codex: &Path, context: &Context) -> Plan {
    let mut plan = Plan::default();
    for dir in children(claude).into_iter().filter(|p| p.is_dir()) {
        let counted = context
            .projects
            .wants_claude_dir(&dir.file_name().unwrap_or_default().to_string_lossy());
        for transcript in children(&dir) {
            if transcript.extension().is_none_or(|e| e != "jsonl") || !transcript.is_file() {
                continue;
            }
            let mut paths = vec![transcript.clone()];
            let beside = transcript.with_extension("");
            if beside.is_dir() {
                paths.push(beside);
            }
            judge(&mut plan, context, "anthropic", &transcript, paths, counted);
        }
    }
    let mut rollouts = Vec::new();
    files_below(codex, &mut rollouts);
    for rollout in rollouts {
        if rollout.extension().is_some_and(|e| e == "jsonl") {
            judge(
                &mut plan,
                context,
                "openai",
                &rollout.clone(),
                vec![rollout],
                true,
            );
        }
    }
    plan
}

fn judge(
    plan: &mut Plan,
    context: &Context,
    provider: &'static str,
    transcript: &Path,
    paths: Vec<PathBuf>,
    counted: bool,
) {
    let mut files = Vec::new();
    for path in &paths {
        if path.is_dir() {
            files_below(path, &mut files);
        } else {
            files.push(path.clone());
        }
    }
    let newest = files
        .iter()
        .filter_map(|f| f.metadata().and_then(|m| m.modified()).ok())
        .max();
    if newest.is_none_or(|at| at > context.cutoff) {
        return;
    }
    let name = transcript.to_string_lossy();
    let stem = transcript.file_stem().unwrap_or_default().to_string_lossy();
    if context.resumed.contains(name.as_ref())
        || context.resumed.iter().any(|id| stem.contains(id.as_str()))
    {
        plan.kept_resumed += 1;
        return;
    }
    if files.iter().any(|f| context.open.contains(f)) {
        plan.kept_open += 1;
        return;
    }
    let unread = counted
        && files
            .iter()
            .filter(|f| f.extension().is_some_and(|e| e == "jsonl"))
            .any(|f| !fully_read(f, context.offsets.get(f).copied()));
    if unread {
        plan.kept_unread += 1;
        return;
    }
    let bytes = files
        .iter()
        .filter_map(|f| f.metadata().ok())
        .map(|m| m.len())
        .sum();
    plan.delete.push(Session {
        provider,
        paths,
        bytes,
    });
}

/// The scan stops before a line with no newline, since an agent may still be
/// writing it. A file that ends that way for good — a crash mid-write — is
/// still read in full.
fn fully_read(path: &Path, offset: Option<u64>) -> bool {
    let Some(offset) = offset else { return false };
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let size = file.metadata().map(|m| m.len()).unwrap_or(u64::MAX);
    if offset >= size {
        return true;
    }
    if size - offset > 1 << 20 || file.seek(SeekFrom::Start(offset)).is_err() {
        return false;
    }
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).is_ok() && !tail.contains(&b'\n')
}

/// Every session a Workbench agent can resume, stopped ones included: a
/// stopped agent comes back with its conversation only if this file is there.
fn resumed_sessions() -> HashSet<String> {
    let Some(text) = dirs::config_dir()
        .map(|dir| dir.join("workbench").join("state.json"))
        .and_then(|path| std::fs::read_to_string(path).ok())
    else {
        return HashSet::new();
    };
    let state: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    let mut out = HashSet::new();
    for sessions in state["sessions"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
    {
        for session in sessions.as_array().into_iter().flatten() {
            for key in ["provider_session_id", "journal_path"] {
                if let Some(value) = session[key].as_str().filter(|v| !v.is_empty()) {
                    out.insert(value.to_string());
                }
            }
        }
    }
    out
}

/// Transcripts some process holds open, from one `lsof` of the machine.
fn open_transcripts() -> HashSet<PathBuf> {
    let Ok(output) = std::process::Command::new("lsof")
        .args(["-n", "-Fn"])
        .output()
    else {
        return HashSet::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix('n'))
        .filter(|path| path.ends_with(".jsonl"))
        .map(PathBuf::from)
        .collect()
}

fn children(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

fn files_below(dir: &Path, out: &mut Vec<PathBuf>) {
    for path in children(dir) {
        if path.is_dir() {
            files_below(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn old(path: &Path) {
        let then = SystemTime::now() - Duration::from_secs(40 * 86400);
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(then)
            .unwrap();
    }

    #[test]
    fn only_old_counted_unclaimed_sessions_are_deleted() {
        let root = tempfile::tempdir().unwrap();
        let claude = root.path().join("claude");
        let codex = root.path().join("codex");
        let project = claude.join("-h-Code-app");
        std::fs::create_dir_all(project.join("gone/subagents")).unwrap();
        std::fs::create_dir_all(project.join("memory")).unwrap();
        std::fs::create_dir_all(codex.join("2026/09/01")).unwrap();

        let file = |path: PathBuf, body: &str, age_old: bool| {
            std::fs::write(&path, body).unwrap();
            if age_old {
                old(&path);
            }
            path
        };
        let gone = file(project.join("gone.jsonl"), "{}\n", true);
        let sub = file(project.join("gone/subagents/a.jsonl"), "{}\n", true);
        let memory = file(project.join("memory/MEMORY.md"), "keep", true);
        let recent = file(project.join("recent.jsonl"), "{}\n", false);
        let resumed = file(project.join("abc-123.jsonl"), "{}\n", true);
        let unread = file(project.join("unread.jsonl"), "{}\n{}\n", true);
        let rollout = file(codex.join("2026/09/01/rollout-x-01a1.jsonl"), "{}\n", true);
        let open = file(codex.join("2026/09/01/rollout-y-02b2.jsonl"), "{}\n", true);

        let offsets = [&gone, &sub, &resumed, &rollout, &open, &recent]
            .into_iter()
            .map(|p| (p.clone(), 3))
            .chain([(unread.clone(), 3)])
            .collect();
        let context = Context {
            cutoff: SystemTime::now() - Duration::from_secs(14 * 86400),
            offsets,
            resumed: ["abc-123".to_string()].into(),
            open: [open.clone()].into(),
            projects: Projects::new(
                vec![("/h/Code/app".into(), "app".into(), false)],
                Path::new("/h"),
            ),
        };
        let plan = plan_in(&claude, &codex, &context);

        let mut deleted: Vec<_> = plan.delete.iter().flat_map(|s| s.paths.clone()).collect();
        deleted.sort();
        let mut expected = vec![gone.clone(), project.join("gone"), rollout.clone()];
        expected.sort();
        assert_eq!(deleted, expected);
        assert_eq!(
            (plan.kept_resumed, plan.kept_open, plan.kept_unread),
            (1, 1, 1)
        );

        delete(&plan);
        assert!(!gone.exists() && !sub.exists() && !rollout.exists());
        assert!(memory.exists() && recent.exists() && resumed.exists() && unread.exists());
    }

    #[test]
    fn a_last_line_cut_off_for_good_still_counts_as_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        std::fs::write(&path, "{}\n{\"cut").unwrap();
        assert!(fully_read(&path, Some(3)));
        assert!(!fully_read(&path, Some(0)));
        assert!(!fully_read(&path, None));
    }
}
