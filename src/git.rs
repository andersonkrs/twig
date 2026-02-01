use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

    // Create the worktree (suppress output to avoid breaking TUI)
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

    let output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("Failed to create git worktree")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git worktree add failed: {}", stderr.trim());
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

                copy_path_preserve_symlinks(&src, &dst)?;
            }
        }

        for file in &wt_config.symlink {
            let src = project_root.join(file);
            let dst = worktree_path.join(file);

            if src.exists() {
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent).ok();
                }

                create_symlink(&src, &dst)?;
            }
        }
    }

    Ok(worktree_path)
}

pub fn parse_pr_number(input: &str) -> Option<u64> {
    let trimmed = input.trim();
    let number = trimmed.strip_prefix('#')?;
    if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    number.parse().ok()
}

pub struct WorktreeFromPr {
    pub path: PathBuf,
    pub branch: String,
}

#[derive(Deserialize)]
struct GhPrInfo {
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "headRepository")]
    head_repository: GhRepository,
}

#[derive(Deserialize)]
struct GhRepository {
    #[serde(rename = "nameWithOwner")]
    name_with_owner: String,
}

#[derive(Deserialize)]
struct GhRepoView {
    #[serde(rename = "sshUrl")]
    ssh_url: Option<String>,
    url: Option<String>,
}

pub fn create_worktree_from_pr(project: &Project, pr_number: u64) -> Result<WorktreeFromPr> {
    let project_root = project.root_expanded();
    let pr_info = gh_pr_info(&project_root, pr_number)?;
    let repo_url = gh_repo_clone_url(&project_root, &pr_info.head_repository.name_with_owner)?;
    let branch_name = select_pr_branch_name(&project_root, pr_number, &pr_info.head_ref_name)?;

    fetch_pr_branch(&project_root, &repo_url, &pr_info.head_ref_name)?;
    create_local_branch_from_fetch(&project_root, &branch_name)?;

    let path = create_worktree(project, &branch_name)?;

    Ok(WorktreeFromPr {
        path,
        branch: branch_name,
    })
}

/// Delete a git worktree and its local branch
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

    // Remove the worktree (suppress output to avoid breaking TUI)
    let output = Command::new("git")
        .current_dir(&project_root)
        .args(["worktree", "remove", "--force"])
        .arg(&worktree_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("Failed to remove git worktree")?;

    if !output.status.success() {
        // Try force removal of the directory
        fs::remove_dir_all(&worktree_path)
            .with_context(|| format!("Failed to remove worktree directory: {:?}", worktree_path))?;

        // Prune worktree references
        Command::new("git")
            .current_dir(&project_root)
            .args(["worktree", "prune"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok();
    }

    // Delete the local branch
    delete_local_branch(&project_root, branch)?;

    Ok(())
}

/// Delete a local git branch
fn delete_local_branch(repo_path: &Path, branch: &str) -> Result<()> {
    // Force delete the branch (-D) since the worktree is already removed
    let output = Command::new("git")
        .current_dir(repo_path)
        .args(["branch", "-D", branch])
        .output()
        .context("Failed to delete local branch")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Ignore error if branch doesn't exist (may have been a remote-tracking branch)
        if !stderr.contains("not found") {
            eprintln!(
                "Warning: could not delete branch '{}': {}",
                branch,
                stderr.trim()
            );
        }
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

fn gh_pr_info(repo_path: &Path, pr_number: u64) -> Result<GhPrInfo> {
    let output = Command::new("gh")
        .current_dir(repo_path)
        .args([
            "pr",
            "view",
            &pr_number.to_string(),
            "--json",
            "headRefName,headRepository",
        ])
        .output()
        .context("Failed to run gh pr view")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("gh pr view failed: {}", stderr.trim());
    }

    let info: GhPrInfo =
        serde_json::from_slice(&output.stdout).context("Failed to parse gh pr view output")?;
    Ok(info)
}

fn gh_repo_clone_url(repo_path: &Path, name_with_owner: &str) -> Result<String> {
    let output = Command::new("gh")
        .current_dir(repo_path)
        .args(["repo", "view", name_with_owner, "--json", "sshUrl,url"])
        .output()
        .context("Failed to run gh repo view")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("gh repo view failed: {}", stderr.trim());
    }

    let info: GhRepoView =
        serde_json::from_slice(&output.stdout).context("Failed to parse gh repo view output")?;
    if let Some(url) = info.ssh_url.or(info.url) {
        if !url.is_empty() {
            return Ok(url);
        }
    }

    anyhow::bail!("gh repo view returned empty clone url")
}

fn select_pr_branch_name(repo_path: &Path, pr_number: u64, head_ref_name: &str) -> Result<String> {
    let primary = format!("pr-{}", pr_number);
    if !check_branch_exists(repo_path, &primary)? {
        return Ok(primary);
    }

    let base = format!("pr-{}-{}", pr_number, head_ref_name);
    if !check_branch_exists(repo_path, &base)? {
        return Ok(base);
    }

    for idx in 2..=50 {
        let candidate = format!("{}-{}", base, idx);
        if !check_branch_exists(repo_path, &candidate)? {
            return Ok(candidate);
        }
    }

    anyhow::bail!("Unable to find available branch name for PR #{}", pr_number)
}

fn fetch_pr_branch(repo_path: &Path, repo_url: &str, head_ref_name: &str) -> Result<()> {
    let output = Command::new("git")
        .current_dir(repo_path)
        .args(["fetch", repo_url, head_ref_name])
        .output()
        .context("Failed to fetch PR branch")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git fetch failed: {}", stderr.trim());
    }

    Ok(())
}

fn create_local_branch_from_fetch(repo_path: &Path, branch_name: &str) -> Result<()> {
    let output = Command::new("git")
        .current_dir(repo_path)
        .args(["branch", branch_name, "FETCH_HEAD"])
        .output()
        .context("Failed to create local branch from fetched PR")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git branch failed: {}", stderr.trim());
    }

    Ok(())
}

/// Get the default branch (main or master) for a repository
pub fn get_default_branch(repo_path: &Path) -> Result<String> {
    // Try to get from remote HEAD
    let output = Command::new("git")
        .current_dir(repo_path)
        .args(["symbolic-ref", "refs/remotes/origin/HEAD", "--short"])
        .output()
        .context("Failed to get default branch")?;

    if output.status.success() {
        let branch = String::from_utf8_lossy(&output.stdout)
            .trim()
            .strip_prefix("origin/")
            .unwrap_or("main")
            .to_string();
        return Ok(branch);
    }

    // Fallback: check if main or master exists
    for branch in ["main", "master"] {
        let status = Command::new("git")
            .current_dir(repo_path)
            .args(["rev-parse", "--verify", branch])
            .output()?;

        if status.status.success() {
            return Ok(branch.to_string());
        }
    }

    Ok("main".to_string())
}

/// Merge a branch into the default branch (main/master)
pub fn merge_branch_to_default(repo_path: &Path, branch: &str) -> Result<()> {
    let default_branch = get_default_branch(repo_path)?;

    // Checkout default branch (suppress output to avoid breaking TUI)
    let output = Command::new("git")
        .current_dir(repo_path)
        .args(["checkout", &default_branch])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("Failed to checkout default branch")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to checkout '{}': {}", default_branch, stderr.trim());
    }

    // Merge the branch (suppress output to avoid breaking TUI)
    let output = Command::new("git")
        .current_dir(repo_path)
        .args(["merge", branch])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("Failed to merge branch")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Merge failed: {}. Please resolve conflicts manually in the main repository.",
            stderr.trim()
        );
    }

    Ok(())
}

/// Copy a file or directory, preserving symlinks
fn copy_path_preserve_symlinks(src: &Path, dst: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(src)
        .with_context(|| format!("Failed to read metadata for {:?}", src))?;

    if metadata.file_type().is_symlink() {
        let target = fs::read_link(src)
            .with_context(|| format!("Failed to read symlink target for {:?}", src))?;
        create_symlink(&target, dst)?;
        return Ok(());
    }

    if metadata.is_dir() {
        copy_dir_recursive(src, dst)?;
    } else {
        fs::copy(src, dst).with_context(|| format!("Failed to copy {:?} to {:?}", src, dst))?;

        // Ensure the file is fully synced to disk before returning
        // This prevents race conditions where the file appears corrupted
        // to processes that read it immediately after copying
        let file = File::open(dst)
            .with_context(|| format!("Failed to open copied file for sync: {:?}", dst))?;
        file.sync_all()
            .with_context(|| format!("Failed to sync copied file to disk: {:?}", dst))?;
    }

    Ok(())
}

/// Recursively copy a directory, preserving symlinks
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        copy_path_preserve_symlinks(&src_path, &dst_path)?;
    }

    Ok(())
}

#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path) -> Result<()> {
    use std::os::unix::fs::symlink;

    symlink(target, link)
        .with_context(|| format!("Failed to create symlink {:?} -> {:?}", link, target))
}

#[cfg(not(unix))]
fn create_symlink(_target: &Path, _link: &Path) -> Result<()> {
    anyhow::bail!("Symlink copying is only supported on Unix systems")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pr_number() {
        assert_eq!(parse_pr_number("#123"), Some(123));
        assert_eq!(parse_pr_number(" #42 "), Some(42));
        assert_eq!(parse_pr_number("#"), None);
        assert_eq!(parse_pr_number("#abc"), None);
        assert_eq!(parse_pr_number("123"), None);
    }
}
