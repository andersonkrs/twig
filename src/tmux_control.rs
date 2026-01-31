use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use anyhow::{Context, Result};

pub struct ControlClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl ControlClient {
    pub fn connect(server: Option<&str>) -> Result<Self> {
        let mut command = Command::new("tmux");
        if let Some(socket) = server {
            command.args(["-L", socket]);
        }

        let mut child = command
            .arg("-C")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("Failed to spawn tmux control client")?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to open tmux control stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to open tmux control stdout"))?;

        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    pub fn connect_with_socket_path(socket_path: &str) -> Result<Self> {
        let mut command = Command::new("tmux");
        command.args(["-S", socket_path]);

        let mut child = command
            .arg("-C")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("Failed to spawn tmux control client")?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to open tmux control stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to open tmux control stdout"))?;

        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    pub fn command(&mut self, cmd: &str) -> Result<Vec<String>> {
        if debug_enabled() {
            eprintln!("[tmux-control] >> {}", cmd);
        }
        writeln!(self.stdin, "{}", cmd).context("Failed to write tmux control command")?;
        self.stdin
            .flush()
            .context("Failed to flush tmux control command")?;

        let mut output = Vec::new();
        let mut command_id: Option<u64> = None;

        loop {
            let mut line = String::new();
            let bytes = self
                .stdout
                .read_line(&mut line)
                .context("Failed to read tmux control output")?;

            if bytes == 0 {
                anyhow::bail!("tmux control mode closed unexpectedly");
            }

            let trimmed = line.trim_end_matches(['\r', '\n']);

            if debug_enabled() {
                eprintln!("[tmux-control] << {}", trimmed);
            }

            if trimmed.starts_with("%exit") {
                anyhow::bail!("tmux control mode exited unexpectedly");
            }

            if trimmed.starts_with("%error") {
                anyhow::bail!("tmux control error: {}", trimmed);
            }

            if trimmed.starts_with("%begin") {
                if command_id.is_none() {
                    command_id = Some(parse_command_id(trimmed)?);
                }
                continue;
            }

            if trimmed.starts_with("%end") {
                if let Some(expected) = command_id {
                    if parse_command_id(trimmed)? == expected {
                        break;
                    }
                }
                continue;
            }

            if command_id.is_none() {
                continue;
            }

            if trimmed.starts_with('%') {
                continue;
            }

            output.push(trimmed.to_string());
        }

        Ok(output)
    }

    pub fn command_with_output(&mut self, cmd: &str) -> Result<Vec<String>> {
        let sentinel = format!("__TWIG_DONE__{}__", unique_nonce());
        let sentinel_cmd = format!("display-message -p {}", quote_tmux_arg(&sentinel));

        if debug_enabled() {
            eprintln!("[tmux-control] >> {}", cmd);
            eprintln!("[tmux-control] >> {}", sentinel_cmd);
        }

        writeln!(self.stdin, "{}", cmd).context("Failed to write tmux control command")?;
        writeln!(self.stdin, "{}", sentinel_cmd)
            .context("Failed to write tmux control sentinel")?;
        self.stdin
            .flush()
            .context("Failed to flush tmux control command")?;

        let mut output = Vec::new();
        let mut error: Option<String> = None;
        let mut command_id: Option<u64> = None;
        let mut sentinel_id: Option<u64> = None;
        let mut sentinel_seen = false;
        let mut sentinel_end_seen = false;

        while !(sentinel_seen && sentinel_end_seen) {
            let mut line = String::new();
            let bytes = self
                .stdout
                .read_line(&mut line)
                .context("Failed to read tmux control output")?;

            if bytes == 0 {
                anyhow::bail!("tmux control mode closed unexpectedly");
            }

            let trimmed = line.trim_end_matches(['\r', '\n']);

            if debug_enabled() {
                eprintln!("[tmux-control] << {}", trimmed);
            }

            if trimmed.starts_with("%exit") {
                error = Some("tmux control mode exited unexpectedly".to_string());
                continue;
            }

            if trimmed.starts_with("%error") {
                if error.is_none() {
                    error = Some(format!("tmux control error: {}", trimmed));
                }
                continue;
            }

            if trimmed.starts_with("%begin") {
                if command_id.is_none() {
                    command_id = Some(parse_command_id(trimmed)?);
                } else if sentinel_id.is_none() {
                    sentinel_id = Some(parse_command_id(trimmed)?);
                }
                continue;
            }

            if trimmed.starts_with("%end") {
                if let Some(id) = sentinel_id {
                    if parse_command_id(trimmed)? == id {
                        sentinel_end_seen = true;
                    }
                }
                continue;
            }

            if trimmed.starts_with('%') {
                continue;
            }

            if trimmed == sentinel {
                sentinel_seen = true;
                continue;
            }

            output.push(trimmed.to_string());
        }

        if let Some(message) = error {
            anyhow::bail!(message);
        }

        Ok(output)
    }

    pub fn new_window(&mut self, session: &str, name: &str, cwd: &std::path::Path) -> Result<()> {
        let command = format!(
            "new-window -d -t {} -n {} -c {}",
            quote_tmux_arg(session),
            quote_tmux_arg(name),
            quote_tmux_arg(&cwd.to_string_lossy())
        );
        self.command(&command)?;
        Ok(())
    }

    pub fn split_window(&mut self, target: &str, cwd: &std::path::Path) -> Result<()> {
        let command = format!(
            "split-window -t {} -c {}",
            quote_tmux_arg(target),
            quote_tmux_arg(&cwd.to_string_lossy())
        );
        self.command(&command)?;
        Ok(())
    }

    pub fn send_keys(&mut self, target: &str, keys: &str, enter: bool) -> Result<()> {
        let mut command = format!(
            "send-keys -t {} {}",
            quote_tmux_arg(target),
            quote_tmux_arg(keys)
        );

        if enter {
            command.push_str(" Enter");
        }

        self.command(&command)?;
        Ok(())
    }

    pub fn list_panes(&mut self, target: &str) -> Result<Vec<String>> {
        let command = format!(
            "list-panes -t {} -F {}",
            quote_tmux_arg(target),
            quote_tmux_arg(
                "#{pane_index}\t#{pane_id}\t#{pane_current_command}\t#{pane_current_path}"
            )
        );
        self.command_with_output(&command)
    }
}

fn quote_tmux_arg(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{}\"", escaped)
}

fn debug_enabled() -> bool {
    std::env::var_os("TWIG_TMUX_DEBUG").is_some()
}

fn unique_nonce() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn parse_command_id(line: &str) -> Result<u64> {
    let mut parts = line.split_whitespace();
    let prefix = parts.next().unwrap_or_default();
    if !prefix.starts_with('%') {
        anyhow::bail!("Malformed tmux control line: {}", line);
    }

    let _time = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("Malformed tmux control line: {}", line))?;
    let id = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("Malformed tmux control line: {}", line))?;

    id.parse::<u64>()
        .with_context(|| format!("Invalid tmux command id: {}", line))
}

impl Drop for ControlClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmux_available() -> bool {
        Command::new("tmux")
            .arg("-V")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn unique_server_name() -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        format!("twig-test-{}-{}", std::process::id(), now)
    }

    #[test]
    fn test_control_new_window() {
        if !tmux_available() {
            eprintln!("tmux not available, skipping control mode test");
            return;
        }

        let server = unique_server_name();
        let _guard = ServerGuard::new(server.clone());
        let session = "twig_test_session";
        let window = "extra";

        let mut client = match ControlClient::connect(Some(&server)) {
            Ok(client) => client,
            Err(err) => {
                eprintln!("tmux control client unavailable: {err}");
                return;
            }
        };

        if let Err(err) = client.command(&format!("new-session -d -s {}", session)) {
            eprintln!("failed to create test session: {err}");
            let _ = client.command("kill-server");
            return;
        }

        if let Err(err) = client.new_window(session, window, std::path::Path::new("/")) {
            eprintln!("failed to create test window: {err}");
            let _ = client.command("kill-server");
            return;
        }

        let output = Command::new("tmux")
            .args([
                "-L",
                &server,
                "list-windows",
                "-t",
                session,
                "-F",
                "#{window_name}",
            ])
            .output()
            .expect("failed to run tmux list-windows");

        assert!(
            output.status.success(),
            "tmux list-windows failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let windows: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect();

        assert!(
            windows.iter().any(|name| name == window),
            "expected window '{}' in {:?}",
            window,
            windows
        );
    }

    #[test]
    fn test_control_split_window_adds_pane() {
        if !tmux_available() {
            eprintln!("tmux not available, skipping control mode test");
            return;
        }

        let server = unique_server_name();
        let _guard = ServerGuard::new(server.clone());
        let session = "twig_test_session";

        let mut client = match ControlClient::connect(Some(&server)) {
            Ok(client) => client,
            Err(err) => {
                eprintln!("tmux control client unavailable: {err}");
                return;
            }
        };

        if let Err(err) = client.command(&format!("new-session -d -s {}", session)) {
            eprintln!("failed to create test session: {err}");
            return;
        }

        let before = Command::new("tmux")
            .args([
                "-L",
                &server,
                "list-panes",
                "-t",
                session,
                "-F",
                "#{pane_id}",
            ])
            .output()
            .expect("failed to run tmux list-panes");

        if !before.status.success() {
            eprintln!(
                "tmux list-panes failed: {}",
                String::from_utf8_lossy(&before.stderr)
            );
            return;
        }

        let before_panes: Vec<String> = String::from_utf8_lossy(&before.stdout)
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect();

        if let Err(err) = client.split_window(session, std::path::Path::new("/")) {
            eprintln!("failed to split window: {err}");
            return;
        }

        let output = Command::new("tmux")
            .args([
                "-L",
                &server,
                "list-panes",
                "-t",
                session,
                "-F",
                "#{pane_id}",
            ])
            .output()
            .expect("failed to run tmux list-panes");

        assert!(
            output.status.success(),
            "tmux list-panes failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let panes: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect();

        assert!(
            panes.len() > before_panes.len(),
            "expected more panes after split: before {:?}, after {:?}",
            before_panes,
            panes
        );
    }

    #[test]
    fn test_control_list_panes_returns_entries() {
        if !tmux_available() {
            eprintln!("tmux not available, skipping control mode test");
            return;
        }

        let server = unique_server_name();
        let _guard = ServerGuard::new(server.clone());
        let session = "twig_test_session";

        let mut client = match ControlClient::connect(Some(&server)) {
            Ok(client) => client,
            Err(err) => {
                eprintln!("tmux control client unavailable: {err}");
                return;
            }
        };

        if let Err(err) = client.command(&format!("new-session -d -s {}", session)) {
            eprintln!("failed to create test session: {err}");
            return;
        }

        let panes = match client.list_panes(session) {
            Ok(panes) => panes,
            Err(err) => {
                eprintln!("failed to list panes: {err}");
                return;
            }
        };

        assert!(
            !panes.is_empty(),
            "expected panes for session '{}', got none",
            session
        );
    }

    struct ServerGuard {
        name: String,
    }

    impl ServerGuard {
        fn new(name: String) -> Self {
            Self { name }
        }
    }

    impl Drop for ServerGuard {
        fn drop(&mut self) {
            let _ = Command::new("tmux")
                .args(["-L", &self.name, "kill-server"])
                .status();
        }
    }
}
