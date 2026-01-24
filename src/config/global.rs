use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct GlobalConfig {
    /// Base path for worktrees (e.g., ~/Work/twig)
    #[serde(default = "default_worktree_base")]
    pub worktree_base: String,

    /// Path to projects directory (e.g., ~/.config/twig/projects)
    #[serde(default)]
    pub projects_dir: Option<String>,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            worktree_base: default_worktree_base(),
            projects_dir: None,
        }
    }
}

fn default_worktree_base() -> String {
    "~/Work/twig".to_string()
}

impl GlobalConfig {
    /// Get the XDG config directory for twig
    pub fn config_dir() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .context("Could not determine config directory")?
            .join("twig");
        Ok(config_dir)
    }

    /// Get the projects directory (default: ~/.config/twig/projects)
    pub fn projects_dir() -> Result<PathBuf> {
        let config = Self::load()?;
        match config.projects_dir {
            Some(dir) => Ok(PathBuf::from(shellexpand::tilde(&dir).to_string())),
            None => Ok(Self::config_dir()?.join("projects")),
        }
    }

    /// Load global config from ~/.config/twig/config.yml
    pub fn load() -> Result<Self> {
        let config_path = Self::config_dir()?.join("config.yml");

        if config_path.exists() {
            let contents = fs::read_to_string(&config_path)
                .with_context(|| format!("Failed to read config: {:?}", config_path))?;
            let config: GlobalConfig = serde_yaml::from_str(&contents)
                .with_context(|| format!("Failed to parse config: {:?}", config_path))?;
            Ok(config)
        } else {
            Ok(GlobalConfig::default())
        }
    }

    /// Expand the worktree base path (handle ~)
    pub fn worktree_base_expanded(&self) -> PathBuf {
        PathBuf::from(shellexpand::tilde(&self.worktree_base).to_string())
    }

    /// Ensure config directories exist
    pub fn ensure_dirs() -> Result<()> {
        let config_dir = Self::config_dir()?;
        let projects_dir = Self::projects_dir()?;

        if !config_dir.exists() {
            fs::create_dir_all(&config_dir)
                .with_context(|| format!("Failed to create config dir: {:?}", config_dir))?;
        }

        if !projects_dir.exists() {
            fs::create_dir_all(&projects_dir)
                .with_context(|| format!("Failed to create projects dir: {:?}", projects_dir))?;
        }

        Ok(())
    }
}
