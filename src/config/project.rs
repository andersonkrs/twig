use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use super::GlobalConfig;

/// Regex patterns for git URL parsing
static GIT_URL_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        // HTTPS: https://github.com/user/repo.git or https://github.com/user/repo
        Regex::new(r"^https?://[^/]+/(?:.+/)?([^/]+?)(?:\.git)?$").unwrap(),
        // SSH: git@github.com:user/repo.git or git@github.com:user/repo
        Regex::new(r"^git@[^:]+:(?:.+/)?([^/]+?)(?:\.git)?$").unwrap(),
        // SSH with protocol: ssh://git@github.com/user/repo.git
        Regex::new(r"^ssh://[^/]+/(?:.+/)?([^/]+?)(?:\.git)?$").unwrap(),
    ]
});

/// Regex to validate git URLs
static GIT_URL_VALIDATOR: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"^(?:https?://[^/]+/[^/]+/[^/]+(?:\.git)?|git@[^:]+:[^/]+/[^/]+(?:\.git)?|ssh://[^/]+/[^/]+/[^/]+(?:\.git)?)$"
    ).unwrap()
});

#[derive(Debug, Deserialize, Clone)]
pub struct Project {
    /// Project/session name
    pub name: String,

    /// Root directory for the project
    pub root: String,

    /// Git repository URL (https or ssh) - optional
    pub repo: Option<String>,

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

    /// Files/folders to symlink from parent project
    #[serde(default)]
    pub symlink: Vec<String>,

    /// Commands to run after creating the worktree
    #[serde(default)]
    pub post_create: Vec<String>,

    /// Windows to hand off when switching between any project sessions
    /// Commands in these windows are paused in other sessions and restarted
    /// in the target session.
    #[serde(default)]
    pub handoff_windows: Vec<String>,
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

    /// Clone the repository if root doesn't exist and repo URL is configured
    pub fn clone_if_needed(&self) -> Result<()> {
        let root = self.root_expanded();

        if root.exists() {
            return Ok(());
        }

        let repo_url = match &self.repo {
            Some(url) => url,
            None => anyhow::bail!(
                "Project root does not exist: {:?}\nAdd a 'repo' field to clone automatically.",
                root
            ),
        };

        println!("Cloning {} into {:?}...", repo_url, root);

        // Ensure parent directory exists
        if let Some(parent) = root.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {:?}", parent))?;
        }

        let status = Command::new("git")
            .args(["clone", repo_url, &root.to_string_lossy()])
            .status()
            .context("Failed to run git clone")?;

        if !status.success() {
            anyhow::bail!("git clone failed for {}", repo_url);
        }

        println!("Cloned successfully.");
        Ok(())
    }

    /// Extract project name from a git URL
    /// Supports:
    ///   - https://github.com/user/repo.git
    ///   - https://github.com/user/repo
    ///   - git@github.com:user/repo.git
    ///   - git@github.com:user/repo
    ///   - ssh://git@github.com/user/repo.git
    pub fn name_from_repo_url(url: &str) -> Option<String> {
        let url = url.trim();

        for pattern in GIT_URL_PATTERNS.iter() {
            if let Some(captures) = pattern.captures(url) {
                if let Some(name) = captures.get(1) {
                    let name = name.as_str().to_string();
                    if !name.is_empty() {
                        return Some(name);
                    }
                }
            }
        }

        None
    }

    /// Validate if a string is a valid git URL
    pub fn is_git_url(s: &str) -> bool {
        GIT_URL_VALIDATOR.is_match(s.trim())
    }

    /// Windows that should be handoff-managed when switching project sessions.
    pub fn worktree_handoff_windows(&self) -> Vec<String> {
        self.worktree
            .as_ref()
            .map(|worktree| worktree.handoff_windows.clone())
            .unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worktree_config_default_handoff_windows() {
        let config: WorktreeConfig = serde_yaml::from_str(r#"copy: []"#).unwrap();
        assert!(config.handoff_windows.is_empty());
        assert!(config.copy.is_empty());
    }

    #[test]
    fn test_worktree_config_handoff_windows() {
        let project_yaml = r#"
name: demo
root: /tmp/demo
windows:
  - shell:
worktree:
  handoff_windows:
    - rails
    - sidekiq
"#;

        let project: Project = serde_yaml::from_str(project_yaml).unwrap();
        assert_eq!(project.worktree_handoff_windows(), vec!["rails", "sidekiq"]);
    }

    #[test]
    fn test_worktree_session_handoff_windows_are_optional() {
        let project_yaml = r#"
name: demo
root: /tmp/demo
windows:
  - shell:
worktree:
  copy:
    - .env
"#;

        let project: Project = serde_yaml::from_str(project_yaml).unwrap();
        assert!(project.worktree_handoff_windows().is_empty());
    }

    #[test]
    fn test_name_from_https_url() {
        assert_eq!(
            Project::name_from_repo_url("https://github.com/user/myrepo.git"),
            Some("myrepo".to_string())
        );
        assert_eq!(
            Project::name_from_repo_url("https://github.com/user/myrepo"),
            Some("myrepo".to_string())
        );
        assert_eq!(
            Project::name_from_repo_url("https://gitlab.com/org/subgroup/repo.git"),
            Some("repo".to_string())
        );
    }

    #[test]
    fn test_name_from_ssh_url() {
        assert_eq!(
            Project::name_from_repo_url("git@github.com:user/myrepo.git"),
            Some("myrepo".to_string())
        );
        assert_eq!(
            Project::name_from_repo_url("git@github.com:user/myrepo"),
            Some("myrepo".to_string())
        );
        assert_eq!(
            Project::name_from_repo_url("git@gitlab.com:org/subgroup/repo.git"),
            Some("repo".to_string())
        );
    }

    #[test]
    fn test_name_from_ssh_protocol_url() {
        assert_eq!(
            Project::name_from_repo_url("ssh://git@github.com/user/myrepo.git"),
            Some("myrepo".to_string())
        );
        assert_eq!(
            Project::name_from_repo_url("ssh://git@github.com/user/myrepo"),
            Some("myrepo".to_string())
        );
    }

    #[test]
    fn test_is_git_url_valid() {
        assert!(Project::is_git_url("https://github.com/user/repo.git"));
        assert!(Project::is_git_url("https://github.com/user/repo"));
        assert!(Project::is_git_url("git@github.com:user/repo.git"));
        assert!(Project::is_git_url("git@github.com:user/repo"));
        assert!(Project::is_git_url("ssh://git@github.com/user/repo.git"));
    }

    #[test]
    fn test_is_git_url_invalid() {
        assert!(!Project::is_git_url("myproject"));
        assert!(!Project::is_git_url("some-name"));
        assert!(!Project::is_git_url("https://example.com"));
        assert!(!Project::is_git_url(""));
    }
}
