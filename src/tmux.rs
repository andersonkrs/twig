use std::path::PathBuf;
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::config::{Project, Window};
use crate::tmux_control::ControlClient;

const SETUP_WINDOW_NAME: &str = "setup-twig";
const WORKTREE_SESSION_PREFIX: &str = "__";

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
    kill_session_with_timeout(name, Duration::from_secs(30))
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

/// Get the project name from a worktree session name
fn worktree_project_name(session_name: &str) -> Option<&str> {
    session_name
        .split_once(WORKTREE_SESSION_PREFIX)
        .map(|(project, _)| project)
}

/// Check if a session name belongs to a worktree session for the given project
fn is_worktree_session_for_project(session_name: &str, project_name: &str) -> bool {
    worktree_project_name(session_name).is_some_and(|project| project == project_name)
}

fn is_project_session(project_name: &str, session_name: &str) -> bool {
    session_name == project_name || is_worktree_session_for_project(session_name, project_name)
}

/// List running worktree sessions for a project.
#[allow(dead_code)]
pub fn running_worktree_sessions_for_project(project_name: &str) -> Result<Vec<String>> {
    let sessions = list_sessions()?;

    Ok(sessions
        .into_iter()
        .filter(|session_name| is_worktree_session_for_project(session_name, project_name))
        .collect())
}

/// List all running sessions for a project, including the main session and all worktrees.
pub fn running_project_sessions(project_name: &str) -> Result<Vec<String>> {
    let sessions = list_sessions()?;

    Ok(sessions
        .into_iter()
        .filter(|session_name| is_project_session(project_name, session_name))
        .collect())
}

/// Pause configured handoff windows in every other session for this project,
/// then restart those windows in the target session.
pub fn handoff_project_windows(project: &Project, target_session: &str) -> Result<()> {
    let handoff_windows = project.worktree_handoff_windows();
    if handoff_windows.is_empty() {
        return Ok(());
    }

    let sessions = running_project_sessions(&project.name)?;
    if sessions.is_empty() {
        return Ok(());
    }

    let mut client = ControlClient::connect(None)?;
    let mut first_error: Option<anyhow::Error> = None;

    let configured_windows: Vec<(&str, Vec<String>)> = handoff_windows
        .iter()
        .filter_map(|window_name| {
            let commands = commands_for_window(&project.windows, window_name);
            if commands.is_empty() {
                None
            } else {
                Some((window_name.as_str(), commands))
            }
        })
        .collect();

    if configured_windows.is_empty() {
        return Ok(());
    }

    for session_name in sessions {
        let session_windows = match client.list_windows(&session_name) {
            Ok(windows) => windows,
            Err(err) => {
                if first_error.is_none() {
                    first_error = Some(err);
                }
                continue;
            }
        };

        let is_target = session_name == target_session;

        for (window_name, commands) in &configured_windows {
            if !session_windows.iter().any(|name| name == window_name) {
                continue;
            }

            let pane_target = format!("{}:{}", session_name, window_name);
            let panes = match client.list_panes(&pane_target) {
                Ok(panes) => panes,
                Err(err) => {
                    if first_error.is_none() {
                        first_error = Some(err);
                    }
                    continue;
                }
            };

            let pane_infos = parse_pane_infos(&panes);
            if pane_infos.is_empty() {
                continue;
            }

            let pane_indices: Vec<u32> = pane_infos.iter().map(|pane| pane.index).collect();

            for pane in &pane_infos {
                let target = format!("{}:{}.{}", session_name, window_name, pane.index);

                if let Some(pid) = pane.pid {
                    let _ = send_pane_interrupt_signal(&mut client, pid);
                }

                let stop_token = handoff_stop_token(&session_name, window_name, pane.index);
                if let Err(err) = client.send_keys(&target, "C-c", false) {
                    if first_error.is_none() {
                        first_error = Some(err);
                    }
                    break;
                }

                let stop_signal = handoff_stop_signal(&stop_token);
                if let Err(err) = client.send_keys(&target, &stop_signal, true) {
                    if first_error.is_none() {
                        first_error = Some(err);
                    }
                    break;
                }
            }

            if is_target {
                for (command_index, command) in commands.iter().enumerate() {
                    if command_index >= pane_indices.len() {
                        break;
                    }

                    let pane_target = format!(
                        "{}:{}.{}",
                        session_name, window_name, pane_indices[command_index]
                    );

                    if let Err(err) = client.send_keys(&pane_target, command, true) {
                        if first_error.is_none() {
                            first_error = Some(err);
                        }
                        break;
                    }
                }
            }
        }
    }

    if let Some(error) = first_error {
        anyhow::bail!("Failed to apply worktree handoff: {}", error);
    }

    Ok(())
}

fn handoff_stop_token(session_name: &str, window_name: &str, pane_index: u32) -> String {
    format!("twig-handoff-stop:{session_name}:{window_name}:{pane_index}")
}

fn commands_for_window(windows: &[Window], window_name: &str) -> Vec<String> {
    let Some(window) = windows.iter().find(|window| window.name() == window_name) else {
        return vec![];
    };

    if let Some(cmd) = window.simple_command() {
        let trimmed = cmd.trim();
        if trimmed.is_empty() {
            return vec![];
        }
        return vec![trimmed.to_string()];
    }

    window
        .panes()
        .into_iter()
        .filter_map(|pane| pane.command().map(|command| command.trim().to_string()))
        .filter(|command| !command.is_empty())
        .collect()
}

fn parse_pane_infos(lines: &[String]) -> Vec<PaneInfo> {
    let mut panes = Vec::new();

    for line in lines {
        let mut parts = line.split('\t');
        let index = match parts.next() {
            Some(index) => index.trim().parse::<u32>().ok(),
            None => None,
        };

        if let Some(index) = index {
            let pid = parts
                .nth(3)
                .and_then(|value| value.trim().parse::<u32>().ok());
            panes.push(PaneInfo { index, pid });
        }
    }

    panes.sort_unstable_by_key(|pane| pane.index);
    panes
}

#[derive(Debug)]
struct PaneInfo {
    index: u32,
    pid: Option<u32>,
}

fn handoff_stop_signal(stop_token: &str) -> String {
    format!("tmux wait-for -S {}", stop_token)
}

fn send_pane_interrupt_signal(client: &mut ControlClient, pane_pid: u32) -> Result<()> {
    client.command(&format!("run-shell -b \"kill -s SIGINT {}\"", pane_pid))?;
    Ok(())
}

/// Kill all running worktree sessions for a project except the given session.
#[allow(dead_code)]
pub fn kill_other_worktree_sessions_for_project(
    project_name: &str,
    keep_session: &str,
) -> Result<()> {
    let mut target_sessions = running_worktree_sessions_for_project(project_name)?;
    target_sessions.retain(|name| name != keep_session);

    let mut first_error: Option<anyhow::Error> = None;

    for session_name in target_sessions {
        if !session_exists(&session_name)? {
            continue;
        }

        if let Err(err) = safe_kill_session(&session_name) {
            if session_exists(&session_name)? && first_error.is_none() {
                first_error = Some(err);
            }
        }
    }

    if let Some(error) = first_error {
        anyhow::bail!("Failed to stop other worktree sessions: {}", error);
    }

    Ok(())
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

fn kill_session_with_timeout(name: &str, timeout: Duration) -> Result<()> {
    let mut client = ControlClient::connect(None)?;
    client.kill_session(name)?;

    let start = Instant::now();
    loop {
        if !session_exists(name)? {
            return Ok(());
        }

        if start.elapsed() >= timeout {
            anyhow::bail!("Timed out waiting for session '{}' to stop", name);
        }

        sleep(Duration::from_millis(200));
    }
}

/// Connect to a session (attach or switch depending on context)
pub fn connect_to_session(name: &str) -> Result<()> {
    if inside_tmux() {
        switch_client(name)
    } else {
        attach_session(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worktree_project_name() {
        assert_eq!(
            worktree_project_name("myproject__feature-auth"),
            Some("myproject")
        );
        assert_eq!(worktree_project_name("myproject"), None);
    }

    #[test]
    fn test_is_worktree_session_for_project() {
        assert!(is_worktree_session_for_project(
            "myproject__feature-auth",
            "myproject"
        ));
        assert!(!is_worktree_session_for_project(
            "other__feature-auth",
            "myproject"
        ));
        assert!(!is_worktree_session_for_project("myproject", "myproject"));
    }
}
