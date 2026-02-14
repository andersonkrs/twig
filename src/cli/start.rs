use anyhow::Result;

use crate::config::Project;
use crate::tmux::{self, SessionBuilder};
use crate::ui;

pub fn run(project_name: Option<String>) -> Result<()> {
    let name = match project_name {
        Some(n) => n,
        None => ui::select_project("Select project...")?
            .ok_or_else(|| anyhow::anyhow!("No project selected"))?,
    };

    let project = Project::load(&name)?;

    // Check if session already exists
    if tmux::session_exists(&project.name)? {
        println!("Session '{}' already exists, attaching...", project.name);
        tmux::handoff_project_windows(&project, &project.name)?;
        tmux::connect_to_session(&project.name)?;
        return Ok(());
    }

    // Clone repo if root doesn't exist
    project.clone_if_needed()?;

    // Create the session builder
    let builder = SessionBuilder::new(&project);

    // Create session, run post-create, then setup windows via control mode
    println!("Starting session '{}'...", project.name);
    builder.start_with_control()?;
    tmux::handoff_project_windows(&project, &project.name)?;

    // Connect to the session
    tmux::connect_to_session(&project.name)?;

    Ok(())
}
