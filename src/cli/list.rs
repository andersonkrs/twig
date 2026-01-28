use anyhow::Result;

use crate::cli::kill;
use crate::cli::tree_view::{self, SelectedAction};
use crate::config::Project;
use crate::git;
use crate::tmux::{self, SessionBuilder};

/// List all projects and worktrees with interactive tree view
pub fn run(focus_current: bool) -> Result<()> {
    let action = tree_view::run(None, focus_current)?;

    match action {
        Some(SelectedAction::StartProject(name)) => start_project_session(&name),
        Some(SelectedAction::StartWorktree { project, branch }) => {
            start_worktree_session(&project, &branch)
        }
        Some(SelectedAction::KillProject(name)) => kill::run(Some(name)),
        Some(SelectedAction::KillWorktree { project, branch }) => {
            let session_name = format!("{}__{}", project, branch);
            kill::run(Some(session_name))
        }
        None => Ok(()), // User quit
    }
}

/// Start a project's main session
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
