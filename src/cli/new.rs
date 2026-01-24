use anyhow::{Context, Result};
use std::fs;

use crate::config::{GlobalConfig, Project};
use crate::gum;

pub fn run(name: Option<String>) -> Result<()> {
    GlobalConfig::ensure_dirs()?;

    let project_name = match name {
        Some(n) => n,
        None => match gum::input("Project name", None)? {
            Some(n) if !n.is_empty() => n,
            _ => anyhow::bail!("Project name is required"),
        },
    };

    // Check if project already exists
    let config_path = Project::config_path(&project_name)?;
    if config_path.exists() {
        anyhow::bail!(
            "Project '{}' already exists at {:?}",
            project_name,
            config_path
        );
    }

    // Get project root
    let default_root = format!("~/Work/{}", project_name);
    let root = match gum::input("Project root", Some(&default_root))? {
        Some(r) if !r.is_empty() => r,
        _ => default_root,
    };

    // Generate default config
    let config_content = format!(
        r#"name: {}
root: {}

windows:
  - editor:
      panes:
        - nvim
  - shell:
  - shell:
  - git: lazygit

# Worktree configuration (optional)
# worktree:
#   copy:
#     - .env
#     - .env.local
#   post_create:
#     - bundle install
#     - yarn install
"#,
        project_name, root
    );

    // Write the config file
    fs::write(&config_path, &config_content)
        .with_context(|| format!("Failed to write config: {:?}", config_path))?;

    println!("Created project config: {:?}", config_path);
    println!();
    println!("Edit it with: twig edit {}", project_name);
    println!("Start it with: twig start {}", project_name);

    Ok(())
}
