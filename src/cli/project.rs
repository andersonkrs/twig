use anyhow::Result;

use crate::cli::kill;
use crate::cli::tree_view::{self, SelectedAction};
use crate::config::Project;
use crate::tmux::{self, SessionBuilder};

/// List projects only (no worktrees) with interactive tree view
pub fn list() -> Result<()> {
    let action = tree_view::run_projects_only()?;

    match action {
        Some(SelectedAction::StartProject(name)) => start_project_session(&name),
        Some(SelectedAction::StartWorktree { .. }) => {
            // Not expected from projects-only view
            Ok(())
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
