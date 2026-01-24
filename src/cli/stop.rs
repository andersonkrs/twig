use anyhow::Result;

use crate::config::Project;
use crate::gum;
use crate::tmux;

pub fn run(project_name: Option<String>) -> Result<()> {
    let name = match project_name {
        Some(n) => n,
        None => select_session()?,
    };

    if !tmux::session_exists(&name)? {
        anyhow::bail!("Session '{}' is not running", name);
    }

    // Confirm stop
    if !gum::confirm(&format!("Stop session '{}'?", name))? {
        println!("Cancelled.");
        return Ok(());
    }

    tmux::kill_session(&name)?;
    println!("Stopped session: {}", name);

    Ok(())
}

fn select_session() -> Result<String> {
    let sessions = tmux::list_sessions()?;

    if sessions.is_empty() {
        anyhow::bail!("No tmux sessions running");
    }

    // Filter to only show sessions that match our projects
    let projects = Project::list_all().unwrap_or_default();
    let our_sessions: Vec<String> = sessions
        .into_iter()
        .filter(|s| {
            // Match project name or project__branch pattern
            projects
                .iter()
                .any(|p| s == p || s.starts_with(&format!("{}_", p)))
        })
        .collect();

    if our_sessions.is_empty() {
        anyhow::bail!("No twig sessions running");
    }

    match gum::filter(&our_sessions, "Select session to stop...")? {
        Some(selection) => Ok(selection),
        None => anyhow::bail!("No session selected"),
    }
}
