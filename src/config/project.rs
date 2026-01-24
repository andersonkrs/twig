use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use super::GlobalConfig;

#[derive(Debug, Deserialize, Clone)]
pub struct Project {
    /// Project/session name
    pub name: String,

    /// Root directory for the project
    pub root: String,

    /// Windows configuration
    #[serde(default)]
    pub windows: Vec<Window>,

    /// Worktree configuration (optional)
    pub worktree: Option<WorktreeConfig>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum Window {
    /// Simple window with optional command: `- shell:` or `- git: lazygit`
    Simple(HashMap<String, Option<String>>),

    /// Complex window with panes
    Complex {
        #[serde(flatten)]
        inner: HashMap<String, WindowConfig>,
    },
}

#[derive(Debug, Deserialize, Clone)]
pub struct WindowConfig {
    /// Layout: main-vertical, main-horizontal, even-vertical, even-horizontal, tiled
    pub layout: Option<String>,

    /// Panes configuration
    #[serde(default)]
    pub panes: Vec<Pane>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum Pane {
    /// Simple command string
    Command(String),

    /// Just an empty pane (null in YAML)
    Empty,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct WorktreeConfig {
    /// Files/folders to copy from parent project
    #[serde(default)]
    pub copy: Vec<String>,

    /// Commands to run after creating the worktree
    #[serde(default)]
    pub post_create: Vec<String>,
}

impl Project {
    /// Load a project by name
    pub fn load(name: &str) -> Result<Self> {
        let project_path = GlobalConfig::projects_dir()?.join(format!("{}.yml", name));

        if !project_path.exists() {
            anyhow::bail!("Project '{}' not found at {:?}", name, project_path);
        }

        let contents = fs::read_to_string(&project_path)
            .with_context(|| format!("Failed to read project: {:?}", project_path))?;

        let project: Project = serde_yaml::from_str(&contents)
            .with_context(|| format!("Failed to parse project: {:?}", project_path))?;

        Ok(project)
    }

    /// List all available projects
    pub fn list_all() -> Result<Vec<String>> {
        let projects_dir = GlobalConfig::projects_dir()?;

        if !projects_dir.exists() {
            return Ok(vec![]);
        }

        let mut projects = Vec::new();

        for entry in fs::read_dir(&projects_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map(|e| e == "yml").unwrap_or(false) {
                if let Some(stem) = path.file_stem() {
                    projects.push(stem.to_string_lossy().to_string());
                }
            }
        }

        projects.sort();
        Ok(projects)
    }

    /// Get the project config file path
    pub fn config_path(name: &str) -> Result<PathBuf> {
        Ok(GlobalConfig::projects_dir()?.join(format!("{}.yml", name)))
    }

    /// Expand root path (handle ~)
    pub fn root_expanded(&self) -> PathBuf {
        PathBuf::from(shellexpand::tilde(&self.root).to_string())
    }

    /// Get session name for a worktree
    pub fn worktree_session_name(&self, branch: &str) -> String {
        format!("{}__{}", self.name, branch.replace('/', "-"))
    }

    /// Delete project config
    pub fn delete(name: &str) -> Result<()> {
        let path = Self::config_path(name)?;
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("Failed to delete project: {:?}", path))?;
        }
        Ok(())
    }
}

impl Window {
    /// Get the window name
    pub fn name(&self) -> String {
        match self {
            Window::Simple(map) => map.keys().next().cloned().unwrap_or_default(),
            Window::Complex { inner } => inner.keys().next().cloned().unwrap_or_default(),
        }
    }

    /// Get the command for a simple window (single pane)
    pub fn simple_command(&self) -> Option<String> {
        match self {
            Window::Simple(map) => map.values().next().cloned().flatten(),
            Window::Complex { .. } => None,
        }
    }

    /// Get panes for a complex window
    pub fn panes(&self) -> Vec<Pane> {
        match self {
            Window::Simple(_) => vec![],
            Window::Complex { inner } => inner
                .values()
                .next()
                .map(|c| c.panes.clone())
                .unwrap_or_default(),
        }
    }

    /// Get layout for a complex window
    pub fn layout(&self) -> Option<String> {
        match self {
            Window::Simple(_) => None,
            Window::Complex { inner } => inner.values().next().and_then(|c| c.layout.clone()),
        }
    }

    /// Check if this is a complex window with panes
    pub fn has_panes(&self) -> bool {
        matches!(self, Window::Complex { .. })
    }
}

impl Pane {
    /// Get the command to run in this pane
    pub fn command(&self) -> Option<&str> {
        match self {
            Pane::Command(cmd) => Some(cmd),
            Pane::Empty => None,
        }
    }
}
