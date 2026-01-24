use anyhow::Result;

use crate::config::Project;
use crate::gum;

pub fn run(project_name: Option<String>) -> Result<()> {
    let name = match project_name {
        Some(n) => n,
        None => select_project()?,
    };

    let config_path = Project::config_path(&name)?;

    if !config_path.exists() {
        anyhow::bail!("Project '{}' not found", name);
    }

    // Confirm deletion
    if !gum::confirm(&format!("Delete project '{}'?", name))? {
        println!("Cancelled.");
        return Ok(());
    }

    Project::delete(&name)?;
    println!("Deleted project: {}", name);

    Ok(())
}

fn select_project() -> Result<String> {
    let projects = Project::list_all()?;

    if projects.is_empty() {
        anyhow::bail!("No projects found");
    }

    match gum::filter(&projects, "Select project to delete...")? {
        Some(selection) => Ok(selection),
        None => anyhow::bail!("No project selected"),
    }
}
