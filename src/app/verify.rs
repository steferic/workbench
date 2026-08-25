//! Running a check, and recording what it said.
//!
//! Workbench runs these itself, out of process, and writes down the exit code.
//! A manager can ask for work and read the outcome; it cannot produce one.
//! That asymmetry is the only reason "verified" means anything here — an agent
//! reporting on its own work is the failure this design exists to avoid.
//!
//! Everything runs off the event loop on a blocking thread: a test suite takes
//! minutes, and the UI has twenty other agents to keep drawing.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use chrono::Utc;

use crate::models::{Outcome, RepoMark, VerificationRun};

/// How much of the output is kept. Enough to see which test failed; not so
/// much that a runaway build log becomes the saved state.
const TAIL_BYTES: usize = 4 * 1024;

/// How often the runner looks to see whether the command has finished.
const POLL: Duration = Duration::from_millis(100);

/// Makes each log file its own.
///
/// The obvious name — pid plus a timestamp — is not unique: `Utc::now()` does
/// not really resolve to nanoseconds, so two checks starting in the same
/// microsecond took the same path and overwrote each other's output. Two
/// verifications running at once is the normal case here, not a corner.
static NEXT_LOG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Run one check and report what happened.
///
/// Output goes to a temp file rather than a pipe. A pipe would deadlock the
/// moment a chatty command filled the buffer while we were busy waiting for it
/// to exit — which a test suite does easily.
pub fn run(command: &str, dir: &Path, timeout: Duration) -> VerificationRun {
    let started = Instant::now();
    let at = Utc::now();
    let finish = |outcome: Outcome, exit_code: Option<i32>, tail: String| VerificationRun {
        at,
        command: command.to_string(),
        exit_code,
        duration_ms: started.elapsed().as_millis() as u64,
        tail,
        outcome,
    };

    let ticket = NEXT_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let log = std::env::temp_dir().join(format!(
        "workbench-verify-{}-{ticket}.log",
        std::process::id()
    ));
    let Ok(out) = std::fs::File::create(&log) else {
        return finish(Outcome::CouldNotRun, None, "could not open a log file".into());
    };
    let Ok(err) = out.try_clone() else {
        return finish(Outcome::CouldNotRun, None, "could not open a log file".into());
    };

    // Through a shell, because a check is written the way you would type it:
    // pipes, `&&`, and cargo aliases all have to work.
    let spawned = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn();

    let mut child = match spawned {
        Ok(child) => child,
        Err(err) => {
            let _ = std::fs::remove_file(&log);
            return finish(Outcome::CouldNotRun, None, err.to_string());
        }
    };

    // Three ways to stop, and they are genuinely different things — flatten
    // them into an exit code and the timeout becomes indistinguishable from a
    // failing test.
    enum Ended {
        Status(std::process::ExitStatus),
        TimedOut,
        Error(String),
    }

    let ended = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ended::Status(status),
            Ok(None) => {}
            Err(err) => break Ended::Error(err.to_string()),
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            break Ended::TimedOut;
        }
        std::thread::sleep(POLL);
    };

    let tail = tail_of(&log);
    let _ = std::fs::remove_file(&log);

    match ended {
        Ended::Status(status) if status.success() => finish(Outcome::Passed, status.code(), tail),
        Ended::Status(status) => finish(Outcome::Failed, status.code(), tail),
        Ended::TimedOut => finish(Outcome::TimedOut, None, tail),
        Ended::Error(message) => finish(Outcome::CouldNotRun, None, format!("{message}\n{tail}")),
    }
}

/// The last few KB, cut at a line boundary so the excerpt starts mid-thought
/// rather than mid-word.
fn tail_of(path: &Path) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    let start = bytes.len().saturating_sub(TAIL_BYTES);
    let slice = &bytes[start..];
    let text = String::from_utf8_lossy(slice);
    if start == 0 {
        return text.into_owned();
    }
    match text.find('\n') {
        Some(at) => text[at + 1..].to_string(),
        None => text.into_owned(),
    }
}

/// Where the repository stands right now.
///
/// Both the commit and the working-tree stat, because work can land as either
/// and watching one of them misses half of it.
pub fn mark(dir: &PathBuf) -> RepoMark {
    let stat = crate::git::get_diff_shortstat(dir, None);
    RepoMark {
        head: head_commit(dir),
        tree: working_tree(dir),
        insertions: stat.insertions,
        deletions: stat.deletions,
    }
}

/// A content hash of everything in the working tree, tracked or not.
///
/// Built in a throwaway index seeded from the repository's own, so `add -A` is
/// an incremental restat rather than a full read, and so the real index — which
/// an agent may be in the middle of using — is never touched.
fn working_tree(dir: &Path) -> Option<String> {
    let ticket = NEXT_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let index = std::env::temp_dir().join(format!(
        "workbench-index-{}-{ticket}",
        std::process::id()
    ));
    let git_dir = git(dir, &["rev-parse", "--git-dir"], None).filter(|d| !d.is_empty())?;
    let real = Path::new(dir).join(&git_dir).join("index");
    let _ = std::fs::copy(&real, &index);

    let hash = git(dir, &["add", "-A"], Some(&index))
        .and_then(|_| git(dir, &["write-tree"], Some(&index)))
        .filter(|hash| !hash.is_empty());
    let _ = std::fs::remove_file(&index);
    hash
}

fn git(dir: &Path, args: &[&str], index: Option<&Path>) -> Option<String> {
    let mut command = Command::new("git");
    command.args(args).current_dir(dir);
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    let output = command.output().ok()?;
    // Success, not output: `git add` says nothing at all when it works, so an
    // empty-means-failed rule would break the one caller that matters.
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn head_commit(dir: &Path) -> Option<String> {
    git(dir, &["rev-parse", "HEAD"], None).filter(|head| !head.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn here() -> PathBuf {
        std::env::temp_dir()
    }

    #[test]
    fn exit_zero_is_a_pass_and_output_is_kept() {
        let run = run("echo hello && exit 0", &here(), Duration::from_secs(10));
        assert_eq!(run.outcome, Outcome::Passed);
        assert_eq!(run.exit_code, Some(0));
        assert!(run.tail.contains("hello"), "{:?}", run.tail);
    }

    #[test]
    fn a_failure_keeps_its_code_and_its_stderr() {
        let run = run("echo boom >&2; exit 3", &here(), Duration::from_secs(10));
        assert_eq!(run.outcome, Outcome::Failed);
        assert_eq!(run.exit_code, Some(3));
        assert!(run.tail.contains("boom"), "{:?}", run.tail);
    }

    /// A check that never ends must not hold the pipeline open forever.
    #[test]
    fn a_command_that_hangs_is_killed_and_named() {
        let run = run("sleep 30", &here(), Duration::from_millis(400));
        assert_eq!(run.outcome, Outcome::TimedOut);
        assert!(run.duration_ms < 10_000, "should not have waited it out");
    }

    /// A new file is how most work arrives, and `git diff` cannot see one.
    /// Before the working-tree hash, that work registered as "nothing changed"
    /// and a passing check was thrown away as meaningless.
    #[test]
    fn an_untracked_file_counts_as_the_repository_moving() {
        let repo = std::env::temp_dir().join(format!("wb-mark-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            Command::new("git").args(args).current_dir(&repo).output().unwrap();
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(repo.join("kept"), "one").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-qm", "first"]);

        let before = mark(&repo);
        assert!(before.tree.is_some(), "the working tree should hash");

        std::fs::write(repo.join("brand-new"), "work").unwrap();
        let after = mark(&repo);
        assert!(after.changed_from(&before), "an untracked file is work: {after:?}");

        // And editing that still-untracked file is movement too — a count of
        // new files would have missed this one.
        std::fs::write(repo.join("brand-new"), "more work").unwrap();
        assert!(mark(&repo).changed_from(&after), "editing it is work too");

        // Whereas an ignored path is build output, not work.
        std::fs::write(repo.join(".gitignore"), "junk\n").unwrap();
        let settled = mark(&repo);
        std::fs::write(repo.join("junk"), "artifact").unwrap();
        assert!(!mark(&repo).changed_from(&settled), "ignored files are not work");

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn a_command_that_cannot_run_says_so() {
        let run = run("exit 0", Path::new("/no/such/directory"), Duration::from_secs(5));
        assert_eq!(run.outcome, Outcome::CouldNotRun);
    }

    /// Chatty commands are the ones that would deadlock a pipe, and the ones
    /// whose output has to be bounded.
    #[test]
    fn a_flood_of_output_is_bounded_and_keeps_the_end() {
        let run = run(
            "for i in $(seq 1 4000); do echo 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'; done; echo LAST",
            &here(),
            Duration::from_secs(30),
        );
        assert_eq!(run.outcome, Outcome::Passed);
        assert!(run.tail.len() <= TAIL_BYTES + 64, "tail was {}", run.tail.len());
        assert!(run.tail.trim_end().ends_with("LAST"), "the end is what matters");
    }
}
