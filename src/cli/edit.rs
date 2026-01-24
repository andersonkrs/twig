use anyhow::{Context, Result};
use std::process::Command;

use crate::config::Project;
use crate::gum;

pub fn run(project_name: Option<String>) -> Result<()> {
    let name = match project_name {
        Some(n) => n,
        None => select_project()?,
    };

    let config_path = Project::config_path(&name)?;

    if !config_path.exists() {
        anyhow::bail!(
            "Project '{}' not found. Create it with: twig new {}",
            name,
            name
        );
    }

    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());

    Command::new(&editor)
        .arg(&config_path)
        .status()
        .with_context(|| format!("Failed to open editor: {}", editor))?;

    Ok(())
}

fn select_project() -> Result<String> {
    let projects = Project::list_all()?;

    if projects.is_empty() {
        anyhow::bail!("No projects found. Create one with: twig new <name>");
    }

    match gum::filter(&projects, "Select project to edit...")? {
        Some(selection) => Ok(selection),
        None => anyhow::bail!("No project selected"),
    }
}
