use anyhow::{Context, Result};
use std::process::Command;

use crate::config::{Project, Window};

/// Check if a tmux session exists
pub fn session_exists(name: &str) -> Result<bool> {
    let output = Command::new("tmux")
        .args(["has-session", "-t", name])
        .output()
        .context("Failed to check tmux session")?;

    Ok(output.status.success())
}

/// Check if a tmux session exists on a specific socket
pub fn session_exists_with_socket(name: &str, socket_path: &str) -> Result<bool> {
    let output = Command::new("tmux")
        .args(["-S", socket_path, "has-session", "-t", name])
        .output()
        .context("Failed to check tmux session")?;

    Ok(output.status.success())
}

/// Attach to an existing tmux session
pub fn attach_session(name: &str) -> Result<()> {
    let status = Command::new("tmux")
        .args(["attach-session", "-t", name])
        .status()
        .context("Failed to attach to tmux session")?;

    if !status.success() {
        anyhow::bail!("Failed to attach to session: {}", name);
    }

    Ok(())
}

/// Switch to a tmux session (when already inside tmux)
pub fn switch_client(name: &str) -> Result<()> {
    let status = Command::new("tmux")
        .args(["switch-client", "-t", name])
        .status()
        .context("Failed to switch tmux client")?;

    if !status.success() {
        anyhow::bail!("Failed to switch to session: {}", name);
    }

    Ok(())
}

/// Check if we're inside a tmux session
pub fn inside_tmux() -> bool {
    std::env::var("TMUX").is_ok()
}

/// Get the current tmux session name (if inside tmux)
pub fn current_session_name() -> Option<String> {
    if !inside_tmux() {
        return None;
    }

    let output = Command::new("tmux")
        .args(["display-message", "-p", "#{session_name}"])
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// Get the current tmux session name for a specific socket
pub fn current_session_name_with_socket(socket_path: &str) -> Option<String> {
    let output = Command::new("tmux")
        .args([
            "-S",
            socket_path,
            "display-message",
            "-p",
            "#{session_name}",
        ])
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// Detach from current tmux session
pub fn detach() -> Result<()> {
    Command::new("tmux")
        .arg("detach-client")
        .status()
        .context("Failed to detach from tmux")?;
    Ok(())
}

/// Kill a tmux session
pub fn kill_session(name: &str) -> Result<()> {
    let status = Command::new("tmux")
        .args(["kill-session", "-t", name])
        .status()
        .context("Failed to kill tmux session")?;

    if !status.success() {
        anyhow::bail!("Failed to kill session: {}", name);
    }

    Ok(())
}

/// Safely kill a session, switching away first if we're inside it
pub fn safe_kill_session(name: &str) -> Result<()> {
    if let Some(current) = current_session_name() {
        if current == name {
            // We're inside the session we want to kill
            // Try to switch to another session first
            let sessions = list_sessions()?;
            let other_session = sessions.iter().find(|s| *s != name);

            if let Some(other) = other_session {
                switch_client(other)?;
            } else {
                // No other session, detach first
                detach()?;
            }
        }
    }

    kill_session(name)
}

/// List all tmux sessions
pub fn list_sessions() -> Result<Vec<String>> {
    let output = Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .output()
        .context("Failed to list tmux sessions")?;

    if output.status.success() {
        let sessions = String::from_utf8(output.stdout)?
            .lines()
            .map(|s| s.to_string())
            .collect();
        Ok(sessions)
    } else {
        // No sessions exist
        Ok(vec![])
    }
}

/// Builder for creating tmux sessions
pub struct SessionBuilder {
    session_name: String,
    root: String,
    windows: Vec<Window>,
    project_name: String,
    worktree_branch: Option<String>,
    post_create_commands: Vec<String>,
}

impl SessionBuilder {
    pub fn new(project: &Project) -> Self {
        let post_create_commands = project
            .worktree
            .as_ref()
            .map(|w| w.post_create.clone())
            .unwrap_or_default();

        Self {
            session_name: project.name.clone(),
            root: project.root.clone(),
            windows: project.windows.clone(),
            project_name: project.name.clone(),
            worktree_branch: None,
            post_create_commands,
        }
    }

    pub fn with_session_name(mut self, name: String) -> Self {
        self.session_name = name;
        self
    }

    pub fn with_root(mut self, root: String) -> Self {
        self.root = root;
        self
    }

    pub fn with_worktree(mut self, branch: String) -> Self {
        self.worktree_branch = Some(branch);
        self
    }

    /// Create the session with a single setup window.
    /// Call `setup_windows()` after post-create commands to configure windows/panes.
    pub fn create_session(&self) -> Result<()> {
        let root_expanded = shellexpand::tilde(&self.root).to_string();

        // Create the session with a temporary setup window
        let mut new_session_cmd = Command::new("tmux");
        new_session_cmd.args([
            "new-session",
            "-d",
            "-s",
            &self.session_name,
            "-n",
            "setup",
            "-c",
            &root_expanded,
            "-e",
            &format!("TWIG_PROJECT={}", self.project_name),
        ]);

        if let Some(branch) = &self.worktree_branch {
            new_session_cmd.args(["-e", &format!("TWIG_WORKTREE={}", branch)]);
        }

        new_session_cmd
            .status()
            .context("Failed to create tmux session")?;

        // Set twig environment variables for the session
        Command::new("tmux")
            .args([
                "set-environment",
                "-t",
                &self.session_name,
                "TWIG_PROJECT",
                &self.project_name,
            ])
            .status()
            .context("Failed to set TWIG_PROJECT")?;

        if let Some(branch) = &self.worktree_branch {
            Command::new("tmux")
                .args([
                    "set-environment",
                    "-t",
                    &self.session_name,
                    "TWIG_WORKTREE",
                    branch,
                ])
                .status()
                .context("Failed to set TWIG_WORKTREE")?;
        }

        Ok(())
    }

    /// Run post-create commands in the session's setup window.
    /// Commands are chained with && so the last command's completion triggers setup.
    pub fn run_post_create_in_session(&self) -> Result<()> {
        if self.post_create_commands.is_empty() {
            return Ok(());
        }

        let target = format!("{}:setup", self.session_name);
        let chained_commands = self.post_create_commands.join(" && ");

        send_keys(&target, &chained_commands)?;

        Ok(())
    }

    /// Run post-create commands followed by a final command.
    /// All commands are chained with && so they run sequentially.
    pub fn run_post_create_then(&self, final_cmd: &str) -> Result<()> {
        let target = format!("{}:setup", self.session_name);

        let chained = if self.post_create_commands.is_empty() {
            final_cmd.to_string()
        } else {
            format!(
                "{} && {}",
                self.post_create_commands.join(" && "),
                final_cmd
            )
        };

        send_keys(&target, &chained)?;

        Ok(())
    }

    /// Check if there are post-create commands to run
    pub fn has_post_create_commands(&self) -> bool {
        !self.post_create_commands.is_empty()
    }

    /// Setup all windows and panes from the project configuration.
    /// This should be called after post-create commands complete.
    pub fn setup_windows(&self) -> Result<()> {
        let root_expanded = shellexpand::tilde(&self.root).to_string();
        let base_index = get_base_index();

        // Get first window name for setup
        let first_window = self.windows.first();
        let first_window_name = first_window
            .map(|w| w.name())
            .unwrap_or_else(|| "shell".to_string());

        // Rename the setup window to the first window name
        Command::new("tmux")
            .args([
                "rename-window",
                "-t",
                &format!("{}:setup", self.session_name),
                &first_window_name,
            ])
            .status()
            .context("Failed to rename setup window")?;

        // Set up the first window
        if let Some(window) = first_window {
            self.setup_window(
                &self.session_name,
                &first_window_name,
                window,
                &root_expanded,
            )?;
        }

        // Create remaining windows
        for window in self.windows.iter().skip(1) {
            let window_name = window.name();

            Command::new("tmux")
                .args([
                    "new-window",
                    "-t",
                    &self.session_name,
                    "-n",
                    &window_name,
                    "-c",
                    &root_expanded,
                ])
                .status()
                .context("Failed to create tmux window")?;

            self.setup_window(&self.session_name, &window_name, window, &root_expanded)?;
        }

        // Select the first window
        Command::new("tmux")
            .args([
                "select-window",
                "-t",
                &format!("{}:{}", self.session_name, base_index),
            ])
            .status()
            .ok();

        Ok(())
    }

    /// Build and start the tmux session (legacy method for backwards compatibility).
    /// Creates session, runs post-create commands, and sets up windows in one call.
    pub fn build(&self) -> Result<()> {
        self.create_session()?;

        // If there are post-create commands, run them but don't wait
        // The old behavior was to set up windows immediately
        if !self.post_create_commands.is_empty() {
            // For backwards compatibility, we run commands but continue with setup
            // Note: This doesn't wait for completion like the new flow
            self.run_post_create_in_session()?;
        }

        self.setup_windows()?;

        Ok(())
    }

    fn setup_window(
        &self,
        session: &str,
        window_name: &str,
        window: &Window,
        root: &str,
    ) -> Result<()> {
        // Use window name instead of index for more reliable targeting
        let target = format!("{}:{}", session, window_name);

        if window.has_panes() {
            let panes = window.panes();
            let layout = window.layout();

            // First pane already exists, run its command if any
            if let Some(first_pane) = panes.first() {
                if let Some(cmd) = first_pane.command() {
                    send_keys(&target, cmd)?;
                }
            }

            // Create additional panes
            for pane in panes.iter().skip(1) {
                // Split based on layout preference
                let split_arg = if layout.as_deref() == Some("main-horizontal") {
                    "-v"
                } else {
                    "-h" // Default to horizontal split (vertical panes)
                };

                Command::new("tmux")
                    .args(["split-window", split_arg, "-t", &target, "-c", root])
                    .status()
                    .context("Failed to split tmux pane")?;

                if let Some(cmd) = pane.command() {
                    // Send to the last pane
                    send_keys(&target, cmd)?;
                }
            }

            // Apply layout if specified
            if let Some(layout_name) = layout {
                Command::new("tmux")
                    .args(["select-layout", "-t", &target, &layout_name])
                    .status()
                    .ok();
            }

            // Select first pane
            Command::new("tmux")
                .args(["select-pane", "-t", &format!("{}.0", target)])
                .status()
                .ok();
        } else if let Some(cmd) = window.simple_command() {
            // Simple window with a single command
            send_keys(&target, &cmd)?;
        }

        Ok(())
    }
}

/// Send keys to a tmux target
fn send_keys(target: &str, keys: &str) -> Result<()> {
    Command::new("tmux")
        .args(["send-keys", "-t", target, keys, "Enter"])
        .status()
        .context("Failed to send keys to tmux")?;

    Ok(())
}

/// Get tmux base-index setting (default is 0, but users often set to 1)
fn get_base_index() -> u32 {
    let output = Command::new("tmux")
        .args(["show-option", "-gv", "base-index"])
        .output()
        .ok();

    output
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Connect to a session (attach or switch depending on context)
pub fn connect_to_session(name: &str) -> Result<()> {
    if inside_tmux() {
        switch_client(name)
    } else {
        attach_session(name)
    }
}
