use anyhow::Result;

use crate::config::Project;
use crate::git;
use crate::tmux::{self, SessionBuilder};
use crate::ui;

/// `twig setup [project] [branch]`
pub fn run(project_name: Option<String>, branch: Option<String>) -> Result<()> {
    let (name, branch) = match project_name {
        Some(n) => (n, branch),
        None => ui::select_project_or_worktree("Select project or worktree to set up...")?
            .ok_or_else(|| anyhow::anyhow!("No project selected"))?,
    };

    let project = Project::load(&name)?;
    let message = ensure_setup(&project, branch.as_deref())?;
    println!("{}", message);

    Ok(())
}

/// Bring `project` (optionally a `branch` worktree of it) up to date:
/// create the worktree/session if missing, or re-run `post_create`
/// commands in a scratch window if the session is already running.
/// Returns a human-readable summary of what happened.
pub fn ensure_setup(project: &Project, branch: Option<&str>) -> Result<String> {
    let (session_name, root, worktree_branch) = match branch {
        Some(branch) => {
            let worktrees = git::list_worktrees(project)?;
            let worktree_path = match worktrees.iter().find(|wt| wt.branch == branch) {
                Some(wt) => wt.path.clone(),
                None => {
                    println!("Creating worktree '{}'...", branch);
                    git::create_worktree(project, branch)?
                }
            };

            (
                project.worktree_session_name(branch),
                worktree_path.to_string_lossy().to_string(),
                Some(branch.to_string()),
            )
        }
        None => (project.name.clone(), project.root.clone(), None),
    };

    if !tmux::session_exists(&session_name)? {
        project.clone_if_needed()?;
        println!("Starting session '{}'...", session_name);

        let mut builder = SessionBuilder::new(project)
            .with_session_name(session_name.clone())
            .with_root(root);

        if let Some(branch) = worktree_branch {
            builder = builder.with_worktree(branch);
        }

        builder.start_with_control()?;

        return Ok(format!("Started session '{}' and ran setup.", session_name));
    }

    let mut builder = SessionBuilder::new(project)
        .with_session_name(session_name.clone())
        .with_root(root);

    if let Some(branch) = worktree_branch {
        builder = builder.with_worktree(branch);
    }

    if !builder.has_post_create_commands() {
        return Ok(format!(
            "Nothing to set up for '{}' (no post_create commands configured).",
            session_name
        ));
    }

    println!("Running post-create tasks for '{}'...", session_name);
    builder.run_post_create_in_scratch_window()?;

    Ok(format!("Ran post-create tasks for '{}'.", session_name))
}
