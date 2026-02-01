use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::config::{Project, Window};
use crate::tmux_control::ControlClient;

const SETUP_WINDOW_NAME: &str = "setup-twig";

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

/// Get the current tmux window name (if inside tmux)
pub fn current_window_name() -> Option<String> {
    if !inside_tmux() {
        return None;
    }

    let output = Command::new("tmux")
        .args(["display-message", "-p", "#{window_name}"])
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

/// Get the current tmux window name for a specific socket
pub fn current_window_name_with_socket(socket_path: &str) -> Option<String> {
    let output = Command::new("tmux")
        .args(["-S", socket_path, "display-message", "-p", "#{window_name}"])
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
                &format!("{}:{}", self.session_name, SETUP_WINDOW_NAME),
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

    /// Start the tmux session using tmux control mode.
    /// Creates session, runs post-create commands sequentially, then sets up windows.
    pub fn start_with_control(&self) -> Result<()> {
        let mut client = ControlClient::connect(None)?;
        self.create_session_with_control(&mut client)?;
        self.run_post_create_with_control(&mut client)?;
        self.setup_windows_with_control(&mut client)?;
        Ok(())
    }

    pub fn create_session_with_control(&self, client: &mut ControlClient) -> Result<()> {
        let root_expanded = PathBuf::from(shellexpand::tilde(&self.root).to_string());
        let mut env = vec![("TWIG_PROJECT", self.project_name.as_str())];
        if let Some(branch) = self.worktree_branch.as_deref() {
            env.push(("TWIG_WORKTREE", branch));
        }

        client.new_session(&self.session_name, SETUP_WINDOW_NAME, &root_expanded, &env)?;

        client.set_environment(&self.session_name, "TWIG_PROJECT", &self.project_name)?;
        if let Some(branch) = &self.worktree_branch {
            client.set_environment(&self.session_name, "TWIG_WORKTREE", branch)?;
        }

        Ok(())
    }

    pub fn run_post_create_with_control(&self, client: &mut ControlClient) -> Result<()> {
        if self.post_create_commands.is_empty() {
            return Ok(());
        }

        let target = format!("{}:{}", self.session_name, SETUP_WINDOW_NAME);

        for (index, command) in self.post_create_commands.iter().enumerate() {
            let trimmed = command.trim();
            if trimmed.is_empty() {
                continue;
            }

            let token = unique_wait_token(&self.session_name, index);
            let signal = format!("{}; tmux wait-for -S {}", trimmed, token);
            client.send_keys(&target, &signal, true)?;
            client.wait_for(&token)?;
        }

        Ok(())
    }

    pub fn setup_windows_with_control(&self, client: &mut ControlClient) -> Result<()> {
        let root_expanded = PathBuf::from(shellexpand::tilde(&self.root).to_string());

        let first_window = self.windows.first();
        let first_window_name = first_window
            .map(|w| w.name())
            .unwrap_or_else(|| "shell".to_string());

        client.rename_window(
            &format!("{}:{}", self.session_name, SETUP_WINDOW_NAME),
            &first_window_name,
        )?;

        if let Some(window) = first_window {
            self.setup_window_with_control(
                client,
                &self.session_name,
                &first_window_name,
                window,
                &root_expanded,
            )?;
        }

        for window in self.windows.iter().skip(1) {
            let window_name = window.name();
            client.new_window(&self.session_name, &window_name, &root_expanded)?;
            self.setup_window_with_control(
                client,
                &self.session_name,
                &window_name,
                window,
                &root_expanded,
            )?;
        }

        client.select_window(&format!("{}:{}", self.session_name, first_window_name))?;

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

    fn setup_window_with_control(
        &self,
        client: &mut ControlClient,
        session: &str,
        window_name: &str,
        window: &Window,
        root: &std::path::Path,
    ) -> Result<()> {
        let target = format!("{}:{}", session, window_name);

        if window.has_panes() {
            let panes = window.panes();
            let layout = window.layout();

            if let Some(first_pane) = panes.first() {
                if let Some(cmd) = first_pane.command() {
                    client.send_keys(&target, cmd, true)?;
                }
            }

            for pane in panes.iter().skip(1) {
                let split_arg = if layout.as_deref() == Some("main-horizontal") {
                    Some("-v")
                } else {
                    Some("-h")
                };

                client.split_window_with_direction(&target, root, split_arg)?;

                if let Some(cmd) = pane.command() {
                    client.send_keys(&target, cmd, true)?;
                }
            }

            if let Some(layout_name) = layout {
                client.select_layout(&target, &layout_name)?;
            }

            let base_index = get_base_index();
            client.select_pane(&format!("{}.{}", target, base_index))?;
        } else if let Some(cmd) = window.simple_command() {
            client.send_keys(&target, &cmd, true)?;
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

fn unique_wait_token(session: &str, index: usize) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("twig-post-create-{}-{}-{}", session, index, now)
}

/// Connect to a session (attach or switch depending on context)
pub fn connect_to_session(name: &str) -> Result<()> {
    if inside_tmux() {
        switch_client(name)
    } else {
        attach_session(name)
    }
}
