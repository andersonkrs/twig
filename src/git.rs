use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{GlobalConfig, Project};

/// Create a git worktree for a project
pub fn create_worktree(project: &Project, branch: &str) -> Result<PathBuf> {
    let config = GlobalConfig::load()?;
    let project_root = project.root_expanded();

    // Worktree path: {worktree_base}/{project}/{branch}
    let branch_safe = branch.replace('/', "-");
    let worktree_path = config
        .worktree_base_expanded()
        .join(&project.name)
        .join(&branch_safe);

    // Check if worktree already exists
    if worktree_path.exists() {
        anyhow::bail!("Worktree already exists at {:?}", worktree_path);
    }

    // Ensure parent directory exists
    if let Some(parent) = worktree_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {:?}", parent))?;
    }

    // Check if branch exists locally or remotely
    let branch_exists = check_branch_exists(&project_root, branch)?;

    // Create the worktree
    let mut cmd = Command::new("git");
    cmd.current_dir(&project_root);
    cmd.arg("worktree").arg("add");

    if branch_exists {
        // Checkout existing branch
        cmd.arg(&worktree_path).arg(branch);
    } else {
        // Create new branch from current HEAD
        cmd.arg("-b").arg(branch).arg(&worktree_path);
    }

    let status = cmd.status().context("Failed to create git worktree")?;

    if !status.success() {
        anyhow::bail!("git worktree add failed");
    }

    // Copy files if configured
    if let Some(wt_config) = &project.worktree {
        for file in &wt_config.copy {
            let src = project_root.join(file);
            let dst = worktree_path.join(file);

            if src.exists() {
                // Create parent directories if needed
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent).ok();
                }

                if src.is_dir() {
                    copy_dir_recursive(&src, &dst)?;
                } else {
                    fs::copy(&src, &dst)
                        .with_context(|| format!("Failed to copy {:?} to {:?}", src, dst))?;
                }
            }
        }
    }

    Ok(worktree_path)
}

/// Run post-create commands in the worktree directory
pub fn run_post_create_commands(project: &Project, worktree_path: &Path) -> Result<()> {
    if let Some(wt_config) = &project.worktree {
        for cmd_str in &wt_config.post_create {
            println!("Running: {}", cmd_str);

            let status = Command::new("sh")
                .arg("-c")
                .arg(cmd_str)
                .current_dir(worktree_path)
                .status()
                .with_context(|| format!("Failed to run: {}", cmd_str))?;

            if !status.success() {
                eprintln!("Warning: command failed: {}", cmd_str);
            }
        }
    }

    Ok(())
}

/// Delete a git worktree
pub fn delete_worktree(project: &Project, branch: &str) -> Result<()> {
    let config = GlobalConfig::load()?;
    let project_root = project.root_expanded();

    let branch_safe = branch.replace('/', "-");
    let worktree_path = config
        .worktree_base_expanded()
        .join(&project.name)
        .join(&branch_safe);

    if !worktree_path.exists() {
        anyhow::bail!("Worktree does not exist at {:?}", worktree_path);
    }

    // Remove the worktree
    let status = Command::new("git")
        .current_dir(&project_root)
        .args(["worktree", "remove", "--force"])
        .arg(&worktree_path)
        .status()
        .context("Failed to remove git worktree")?;

    if !status.success() {
        // Try force removal of the directory
        fs::remove_dir_all(&worktree_path)
            .with_context(|| format!("Failed to remove worktree directory: {:?}", worktree_path))?;

        // Prune worktree references
        Command::new("git")
            .current_dir(&project_root)
            .args(["worktree", "prune"])
            .status()
            .ok();
    }

    Ok(())
}

/// List worktrees for a project
pub fn list_worktrees(project: &Project) -> Result<Vec<WorktreeInfo>> {
    let config = GlobalConfig::load()?;
    let project_root = project.root_expanded();

    let output = Command::new("git")
        .current_dir(&project_root)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .context("Failed to list git worktrees")?;

    if !output.status.success() {
        return Ok(vec![]);
    }

    let stdout = String::from_utf8(output.stdout)?;
    let mut worktrees = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_branch: Option<String> = None;

    let worktree_base = config.worktree_base_expanded().join(&project.name);

    for line in stdout.lines() {
        if line.starts_with("worktree ") {
            // Save previous worktree if any
            if let (Some(path), Some(branch)) = (current_path.take(), current_branch.take()) {
                // Only include worktrees under our worktree_base
                if path.starts_with(&worktree_base) {
                    worktrees.push(WorktreeInfo { path, branch });
                }
            }

            current_path = Some(PathBuf::from(line.strip_prefix("worktree ").unwrap()));
        } else if line.starts_with("branch ") {
            let branch = line
                .strip_prefix("branch refs/heads/")
                .unwrap_or(line.strip_prefix("branch ").unwrap_or(""));
            current_branch = Some(branch.to_string());
        }
    }

    // Don't forget the last one
    if let (Some(path), Some(branch)) = (current_path, current_branch) {
        if path.starts_with(&worktree_base) {
            worktrees.push(WorktreeInfo { path, branch });
        }
    }

    Ok(worktrees)
}

#[derive(Debug)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub branch: String,
}

/// Check if a branch exists (locally or remotely)
fn check_branch_exists(repo_path: &Path, branch: &str) -> Result<bool> {
    // Check local branches
    let local = Command::new("git")
        .current_dir(repo_path)
        .args(["rev-parse", "--verify", branch])
        .output()?;

    if local.status.success() {
        return Ok(true);
    }

    // Check remote branches
    let remote = Command::new("git")
        .current_dir(repo_path)
        .args(["rev-parse", "--verify", &format!("origin/{}", branch)])
        .output()?;

    Ok(remote.status.success())
}

/// Recursively copy a directory
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}
