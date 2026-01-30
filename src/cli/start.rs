use std::env;

use anyhow::Result;

use crate::config::{GlobalConfig, Project};
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
        tmux::connect_to_session(&project.name)?;
        return Ok(());
    }

    // Clone repo if root doesn't exist
    project.clone_if_needed()?;

    // Create the session builder
    let builder = SessionBuilder::new(&project);

    // Create session with setup window
    println!("Starting session '{}'...", project.name);
    builder.create_session()?;

    // If there are post-create commands, run them first, then setup windows
    if builder.has_post_create_commands() {
        // Build the command chain: post-create commands && twig project setup-windows
        builder.run_post_create_then("twig project setup-windows")?;
    } else {
        // No post-create commands, setup windows immediately
        builder.setup_windows()?;
    }

    // Connect to the session
    tmux::connect_to_session(&project.name)?;

    Ok(())
}

/// Internal command to setup windows for an existing session.
/// Called from within the session after post-create commands complete.
/// Reads TWIG_PROJECT and TWIG_WORKTREE from environment.
pub fn setup_windows() -> Result<()> {
    let project_name = env::var("TWIG_PROJECT")
        .map_err(|_| anyhow::anyhow!("TWIG_PROJECT not set - must run inside a twig session"))?;

    let project = Project::load(&project_name)?;

    // Check if we're in a worktree session
    let worktree_branch = env::var("TWIG_WORKTREE").ok();

    let mut builder = SessionBuilder::new(&project);

    // If in a worktree, override root and session name
    if let Some(ref branch) = worktree_branch {
        let config = GlobalConfig::load()?;
        let branch_safe = branch.replace('/', "-");
        let worktree_path = config
            .worktree_base_expanded()
            .join(&project_name)
            .join(&branch_safe);

        let session_name = project.worktree_session_name(branch);

        builder = builder
            .with_session_name(session_name)
            .with_root(worktree_path.to_string_lossy().to_string());
    }

    builder.setup_windows()?;

    Ok(())
}
