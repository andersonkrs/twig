use anyhow::Result;

use crate::config::Project;

pub fn run() -> Result<()> {
    let projects = Project::list_all()?;

    if projects.is_empty() {
        println!("No projects found.");
        println!("Create one with: twig new <name>");
        return Ok(());
    }

    println!("Projects:");
    for project in projects {
        println!("  {}", project);
    }

    Ok(())
}
