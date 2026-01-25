use anyhow::Result;

use crate::cli::tree_view::{self, SelectedAction};
use crate::config::Project;
use crate::git;
use crate::tmux::{self, SessionBuilder};
use crate::ui;

pub fn create(project_name: Option<String>, branch: Option<String>) -> Result<()> {
    let name = match project_name {
        Some(n) => n,
        None => ui::select_project("Select project for worktree...")?
            .ok_or_else(|| anyhow::anyhow!("No project selected"))?,
    };

    let project = Project::load(&name)?;

    let branch_name = match branch {
        Some(b) => b,
        None => ui::input("Branch name", "Enter branch name...", None)?
            .ok_or_else(|| anyhow::anyhow!("Branch name is required"))?,
    };

    println!(
        "Creating worktree for '{}' on branch '{}'...",
        name, branch_name
    );

    // Create the git worktree
    let worktree_path = git::create_worktree(&project, &branch_name)?;
    println!("Created worktree at: {:?}", worktree_path);

    // Run post-create commands if configured
    if project
        .worktree
        .as_ref()
        .map(|w| !w.post_create.is_empty())
        .unwrap_or(false)
    {
        println!("Running post-create commands...");
        git::run_post_create_commands(&project, &worktree_path)?;
    }

    // Create tmux session for the worktree
    let session_name = project.worktree_session_name(&branch_name);

    if tmux::session_exists(&session_name)? {
        println!("Session '{}' already exists, attaching...", session_name);
        tmux::connect_to_session(&session_name)?;
        return Ok(());
    }

    println!("Starting session '{}'...", session_name);
    SessionBuilder::new(&project)
        .with_session_name(session_name.clone())
        .with_root(worktree_path.to_string_lossy().to_string())
        .with_worktree(branch_name.clone())
        .build()?;

    tmux::connect_to_session(&session_name)?;

    Ok(())
}

pub fn list(project_name: Option<String>) -> Result<()> {
    let action = tree_view::run(project_name)?;

    match action {
        Some(SelectedAction::StartProject(name)) => start_project_session(&name),
        Some(SelectedAction::StartWorktree { project, branch }) => {
            start_worktree_session(&project, &branch)
        }
        Some(SelectedAction::KillProject(_) | SelectedAction::KillWorktree { .. }) => {
            // Kill actions not expected from tree list, ignore
            Ok(())
        }
        None => Ok(()), // User quit
    }
}

/// Start a project's main session (same as `twig start <project>`)
fn start_project_session(name: &str) -> Result<()> {
    let project = Project::load(name)?;

    if tmux::session_exists(&project.name)? {
        println!("Session '{}' already exists, attaching...", project.name);
        tmux::connect_to_session(&project.name)?;
        return Ok(());
    }

    project.clone_if_needed()?;

    println!("Starting session '{}'...", project.name);
    SessionBuilder::new(&project).build()?;
    tmux::connect_to_session(&project.name)?;

    Ok(())
}

/// Start or attach to a worktree session
fn start_worktree_session(project_name: &str, branch: &str) -> Result<()> {
    let project = Project::load(project_name)?;
    let session_name = project.worktree_session_name(branch);

    if tmux::session_exists(&session_name)? {
        println!("Session '{}' already exists, attaching...", session_name);
        tmux::connect_to_session(&session_name)?;
        return Ok(());
    }

    // Find the worktree path
    let worktrees = git::list_worktrees(&project)?;
    let worktree = worktrees
        .iter()
        .find(|wt| wt.branch == branch)
        .ok_or_else(|| anyhow::anyhow!("Worktree '{}' not found", branch))?;

    println!("Starting session '{}'...", session_name);
    SessionBuilder::new(&project)
        .with_session_name(session_name.clone())
        .with_root(worktree.path.to_string_lossy().to_string())
        .with_worktree(branch.to_string())
        .build()?;

    tmux::connect_to_session(&session_name)?;

    Ok(())
}

pub fn delete(project_name: Option<String>, branch: Option<String>) -> Result<()> {
    let name = match project_name {
        Some(n) => n,
        None => ui::select_project("Select project...")?
            .ok_or_else(|| anyhow::anyhow!("No project selected"))?,
    };

    let project = Project::load(&name)?;

    let branch_name = match branch {
        Some(b) => b,
        None => ui::select_worktree(&project, "Select worktree to delete...")?
            .ok_or_else(|| anyhow::anyhow!("No worktree selected"))?,
    };

    // Confirm deletion
    if !ui::confirm(&format!(
        "Delete worktree '{}' for project '{}'?",
        branch_name, name
    ))? {
        println!("Cancelled.");
        return Ok(());
    }

    // Kill the tmux session if running
    let session_name = project.worktree_session_name(&branch_name);
    if tmux::session_exists(&session_name)? {
        println!("Stopping session '{}'...", session_name);
        tmux::safe_kill_session(&session_name)?;
    }

    // Delete the worktree
    println!("Deleting worktree...");
    git::delete_worktree(&project, &branch_name)?;

    println!("Deleted worktree: {}", branch_name);

    Ok(())
}

pub fn merge(project_name: Option<String>, branch: Option<String>) -> Result<()> {
    let name = match project_name {
        Some(n) => n,
        None => ui::select_project("Select project...")?
            .ok_or_else(|| anyhow::anyhow!("No project selected"))?,
    };

    let project = Project::load(&name)?;

    let branch_name = match branch {
        Some(b) => b,
        None => ui::select_worktree(&project, "Select worktree to merge...")?
            .ok_or_else(|| anyhow::anyhow!("No worktree selected"))?,
    };

    let default_branch = git::get_default_branch(&project.root_expanded())?;

    // Confirm merge
    if !ui::confirm(&format!(
        "Merge '{}' into '{}'?",
        branch_name, default_branch
    ))? {
        println!("Cancelled.");
        return Ok(());
    }

    // Perform the merge
    println!("Merging '{}' into '{}'...", branch_name, default_branch);
    git::merge_branch_to_default(&project.root_expanded(), &branch_name)?;
    println!("Merged successfully.");

    // Ask if user wants to delete the worktree
    if ui::confirm(&format!(
        "Delete worktree '{}' and its session?",
        branch_name
    ))? {
        // Kill the tmux session if running
        let session_name = project.worktree_session_name(&branch_name);
        if tmux::session_exists(&session_name)? {
            println!("Stopping session '{}'...", session_name);
            tmux::safe_kill_session(&session_name)?;
        }

        // Delete the worktree (also deletes the local branch)
        println!("Deleting worktree...");
        git::delete_worktree(&project, &branch_name)?;
        println!("Deleted worktree: {}", branch_name);
    }

    Ok(())
}
