use anyhow::Result;

use crate::config::Project;
use crate::gum;
use crate::tmux::{self, SessionBuilder};

pub fn run(project_name: Option<String>) -> Result<()> {
    let name = match project_name {
        Some(n) => n,
        None => select_project()?,
    };

    let project = Project::load(&name)?;

    // Check if session already exists
    if tmux::session_exists(&project.name)? {
        println!("Session '{}' already exists, attaching...", project.name);
        tmux::connect_to_session(&project.name)?;
        return Ok(());
    }

    // Verify project root exists
    let root = project.root_expanded();
    if !root.exists() {
        anyhow::bail!("Project root does not exist: {:?}", root);
    }

    // Create the session
    println!("Starting session '{}'...", project.name);
    SessionBuilder::new(&project).build()?;

    // Attach to the session
    tmux::connect_to_session(&project.name)?;

    Ok(())
}

fn select_project() -> Result<String> {
    let projects = Project::list_all()?;

    if projects.is_empty() {
        anyhow::bail!("No projects found. Create one with: twig new <name>");
    }

    if projects.len() == 1 {
        return Ok(projects.into_iter().next().unwrap());
    }

    match gum::filter(&projects, "Select project...")? {
        Some(selection) => Ok(selection),
        None => anyhow::bail!("No project selected"),
    }
}
