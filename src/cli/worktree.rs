use anyhow::Result;

use crate::config::Project;
use crate::git;
use crate::gum;
use crate::tmux::{self, SessionBuilder};

pub fn create(project_name: Option<String>, branch: Option<String>) -> Result<()> {
    let name = match project_name {
        Some(n) => n,
        None => select_project("Select project for worktree...")?,
    };

    let project = Project::load(&name)?;

    let branch_name = match branch {
        Some(b) => b,
        None => match gum::input("Branch name", None)? {
            Some(b) if !b.is_empty() => b,
            _ => anyhow::bail!("Branch name is required"),
        },
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
        .build()?;

    tmux::connect_to_session(&session_name)?;

    Ok(())
}

pub fn list(project_name: Option<String>) -> Result<()> {
    let name = match project_name {
        Some(n) => n,
        None => select_project("Select project...")?,
    };

    let project = Project::load(&name)?;
    let worktrees = git::list_worktrees(&project)?;

    if worktrees.is_empty() {
        println!("No worktrees found for project '{}'", name);
        return Ok(());
    }

    println!("Worktrees for '{}':", name);
    for wt in worktrees {
        let session_name = project.worktree_session_name(&wt.branch);
        let running = if tmux::session_exists(&session_name).unwrap_or(false) {
            " (running)"
        } else {
            ""
        };
        println!("  {} -> {:?}{}", wt.branch, wt.path, running);
    }

    Ok(())
}

pub fn delete(project_name: Option<String>, branch: Option<String>) -> Result<()> {
    let name = match project_name {
        Some(n) => n,
        None => select_project("Select project...")?,
    };

    let project = Project::load(&name)?;

    let branch_name = match branch {
        Some(b) => b,
        None => select_worktree(&project)?,
    };

    // Confirm deletion
    if !gum::confirm(&format!(
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
        tmux::kill_session(&session_name)?;
    }

    // Delete the worktree
    println!("Deleting worktree...");
    git::delete_worktree(&project, &branch_name)?;

    println!("Deleted worktree: {}", branch_name);

    Ok(())
}

fn select_project(placeholder: &str) -> Result<String> {
    let projects = Project::list_all()?;

    if projects.is_empty() {
        anyhow::bail!("No projects found. Create one with: twig new <name>");
    }

    if projects.len() == 1 {
        return Ok(projects.into_iter().next().unwrap());
    }

    match gum::filter(&projects, placeholder)? {
        Some(selection) => Ok(selection),
        None => anyhow::bail!("No project selected"),
    }
}

fn select_worktree(project: &Project) -> Result<String> {
    let worktrees = git::list_worktrees(project)?;

    if worktrees.is_empty() {
        anyhow::bail!("No worktrees found for project '{}'", project.name);
    }

    let branches: Vec<String> = worktrees.iter().map(|w| w.branch.clone()).collect();

    match gum::filter(&branches, "Select worktree to delete...")? {
        Some(selection) => Ok(selection),
        None => anyhow::bail!("No worktree selected"),
    }
}
