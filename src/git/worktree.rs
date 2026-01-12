//! Git worktree operations for parallel task execution.
//!
//! This module provides functions for managing git worktrees, which allow
//! multiple agents to work on the same repository simultaneously without
//! conflicts.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Directory name for storing parallel task worktrees
pub const WORKTREES_DIR: &str = ".worktrees";

/// Check if a path is a git repository
pub fn is_git_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

/// Get the current branch name
pub fn get_current_branch(repo_path: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(repo_path)
        .output()
        .context("Failed to execute git rev-parse")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to get current branch: {}", stderr);
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Get the current HEAD commit hash (short form)
pub fn get_head_commit(repo_path: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(repo_path)
        .output()
        .context("Failed to execute git rev-parse")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to get HEAD commit: {}", stderr);
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Check if the working directory is clean (no uncommitted changes)
pub fn is_clean(repo_path: &Path) -> Result<bool> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_path)
        .output()
        .context("Failed to execute git status")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to check git status: {}", stderr);
    }

    // Empty output means clean working directory
    Ok(output.stdout.is_empty())
}

/// Create a new branch at the current HEAD
pub fn create_branch(repo_path: &Path, branch_name: &str) -> Result<()> {
    let output = Command::new("git")
        .args(["branch", branch_name])
        .current_dir(repo_path)
        .output()
        .context("Failed to execute git branch")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to create branch '{}': {}", branch_name, stderr);
    }

    Ok(())
}

/// Create a worktree for a parallel task attempt
///
/// This creates a new git worktree at the specified path, checking out
/// the given branch. If the branch doesn't exist, it will be created
/// at the current HEAD.
pub fn create_worktree(
    repo_path: &Path,
    branch_name: &str,
    worktree_path: &Path,
) -> Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent)
            .context("Failed to create worktree parent directory")?;
    }

    // Create branch first (git worktree add -b would fail if branch exists)
    // We ignore errors here since the branch might already exist
    let _ = create_branch(repo_path, branch_name);

    // Create worktree
    let output = Command::new("git")
        .args([
            "worktree",
            "add",
            worktree_path.to_str().unwrap(),
            branch_name,
        ])
        .current_dir(repo_path)
        .output()
        .context("Failed to execute git worktree add")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "Failed to create worktree at '{}' for branch '{}': {}",
            worktree_path.display(),
            branch_name,
            stderr
        );
    }

    Ok(())
}

/// Remove a worktree
///
/// Optionally also deletes the associated branch.
pub fn remove_worktree(
    repo_path: &Path,
    worktree_path: &Path,
    delete_branch: bool,
) -> Result<()> {
    // Get branch name before removing worktree (for later deletion)
    let branch_name = if delete_branch {
        get_worktree_branch(worktree_path).ok()
    } else {
        None
    };

    // Remove worktree (force to handle uncommitted changes)
    let output = Command::new("git")
        .args([
            "worktree",
            "remove",
            "--force",
            worktree_path.to_str().unwrap(),
        ])
        .current_dir(repo_path)
        .output()
        .context("Failed to execute git worktree remove")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Don't fail if worktree doesn't exist
        if !stderr.contains("is not a working tree") {
            bail!(
                "Failed to remove worktree at '{}': {}",
                worktree_path.display(),
                stderr
            );
        }
    }

    // Delete branch if requested
    if let Some(branch) = branch_name {
        let _ = delete_branch_force(repo_path, &branch);
    }

    Ok(())
}

/// Get the branch name for a worktree
fn get_worktree_branch(worktree_path: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(worktree_path)
        .output()
        .context("Failed to get worktree branch")?;

    if !output.status.success() {
        bail!("Failed to get worktree branch");
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Force delete a branch
fn delete_branch_force(repo_path: &Path, branch_name: &str) -> Result<()> {
    let output = Command::new("git")
        .args(["branch", "-D", branch_name])
        .current_dir(repo_path)
        .output()
        .context("Failed to execute git branch -D")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to delete branch '{}': {}", branch_name, stderr);
    }

    Ok(())
}

/// Merge a branch into the current branch
pub fn merge_branch(repo_path: &Path, source_branch: &str) -> Result<()> {
    let output = Command::new("git")
        .args(["merge", source_branch, "--no-edit"])
        .current_dir(repo_path)
        .output()
        .context("Failed to execute git merge")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to merge branch '{}': {}", source_branch, stderr);
    }

    Ok(())
}

/// Checkout a specific branch
pub fn checkout_branch(repo_path: &Path, branch_name: &str) -> Result<()> {
    let output = Command::new("git")
        .args(["checkout", branch_name])
        .current_dir(repo_path)
        .output()
        .context("Failed to execute git checkout")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to checkout branch '{}': {}", branch_name, stderr);
    }

    Ok(())
}

/// Get the worktrees directory path for a workspace
pub fn get_worktrees_dir(workspace_path: &Path) -> PathBuf {
    workspace_path.join(WORKTREES_DIR)
}

/// Get the worktree path for a specific parallel task attempt
pub fn get_attempt_worktree_path(
    workspace_path: &Path,
    task_id_short: &str,
    agent_name: &str,
) -> PathBuf {
    get_worktrees_dir(workspace_path)
        .join(format!("parallel-{}", task_id_short))
        .join(agent_name.to_lowercase())
}

/// Get the worktree path for a session
pub fn get_session_worktree_path(workspace_path: &Path, session_id_short: &str) -> PathBuf {
    get_worktrees_dir(workspace_path).join(format!("session-{}", session_id_short))
}

/// Generate a branch name for a session worktree
pub fn session_branch_name(agent_name: &str, session_id_short: &str) -> String {
    format!("agent-{}-{}", agent_name.to_lowercase(), session_id_short)
}

/// Check if a worktree has any uncommitted changes
pub fn worktree_has_changes(worktree_path: &Path) -> bool {
    if !worktree_path.exists() {
        return false;
    }
    !is_clean(worktree_path).unwrap_or(true)
}

/// Commit all changes in a worktree with a given message
pub fn commit_all_changes(worktree_path: &Path, message: &str) -> Result<()> {
    if !worktree_path.exists() {
        return Err(anyhow::anyhow!("Worktree path does not exist"));
    }

    // Stage all changes
    let add_output = Command::new("git")
        .args(["add", "-A"])
        .current_dir(worktree_path)
        .output()
        .context("Failed to run git add")?;

    if !add_output.status.success() {
        return Err(anyhow::anyhow!(
            "git add failed: {}",
            String::from_utf8_lossy(&add_output.stderr)
        ));
    }

    // Commit with message
    let commit_output = Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(worktree_path)
        .output()
        .context("Failed to run git commit")?;

    if !commit_output.status.success() {
        let stderr = String::from_utf8_lossy(&commit_output.stderr);
        // "nothing to commit" is not an error for our purposes
        if stderr.contains("nothing to commit") {
            return Ok(());
        }
        return Err(anyhow::anyhow!("git commit failed: {}", stderr));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::fs::write(dir.path().join("test.txt"), "hello").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        dir
    }

    #[test]
    fn test_is_git_repo() {
        let dir = create_test_repo();
        assert!(is_git_repo(dir.path()));
    }

    #[test]
    fn test_get_current_branch() {
        let dir = create_test_repo();
        let branch = get_current_branch(dir.path()).unwrap();
        // Modern git uses "main", older versions use "master"
        assert!(branch == "main" || branch == "master");
    }

    #[test]
    fn test_is_clean() {
        let dir = create_test_repo();
        assert!(is_clean(dir.path()).unwrap());

        // Make it dirty
        std::fs::write(dir.path().join("new.txt"), "dirty").unwrap();
        assert!(!is_clean(dir.path()).unwrap());
    }

    #[test]
    fn test_create_and_remove_worktree() {
        let dir = create_test_repo();
        let worktree_path = dir.path().join(".worktrees").join("test-worktree");

        create_worktree(dir.path(), "test-branch", &worktree_path).unwrap();
        assert!(worktree_path.exists());

        remove_worktree(dir.path(), &worktree_path, true).unwrap();
        assert!(!worktree_path.exists());
    }
}
