use anyhow::{Context, Result};
use std::env;

use crate::config::Project;
use crate::tmux;
use crate::tmux_control::ControlClient;
use crate::ui;

pub fn new(
    project_name: Option<String>,
    window_name: Option<String>,
    socket: Option<String>,
) -> Result<()> {
    let name = match project_name {
        Some(n) => n,
        None => ui::select_project("Select project...")?
            .ok_or_else(|| anyhow::anyhow!("No project selected"))?,
    };

    let window = match window_name {
        Some(n) => n,
        None => ui::input("Window", "Window name...", None)?
            .ok_or_else(|| anyhow::anyhow!("Window name is required"))?,
    };

    let project = Project::load(&name)?;
    let session_name = name.clone();

    if project.name != session_name {
        eprintln!(
            "Warning: project config name '{}' differs from requested session '{}'",
            project.name, session_name
        );
    }

    let socket_path = socket.or_else(|| {
        env::var("TMUX")
            .ok()
            .and_then(|value| value.split(',').next().map(|part| part.to_string()))
            .filter(|value| !value.is_empty())
    });

    let session_exists = match socket_path.as_deref() {
        Some(path) => tmux::session_exists_with_socket(&session_name, path)?,
        None => tmux::session_exists(&session_name)?,
    };

    if !session_exists {
        anyhow::bail!("Session '{}' is not running", session_name);
    }

    let mut client = match socket_path.as_deref() {
        Some(path) => ControlClient::connect_with_socket_path(path)?,
        None => ControlClient::connect(None)?,
    };
    client.new_window(&session_name, &window, &project.root_expanded())?;

    println!("Created window '{}' in session '{}'", window, session_name);

    Ok(())
}

pub fn new_run(
    project_name: Option<String>,
    window_name: Option<String>,
    command: Vec<String>,
    socket: Option<String>,
) -> Result<()> {
    let name = match project_name {
        Some(n) => n,
        None => ui::select_project("Select project...")?
            .ok_or_else(|| anyhow::anyhow!("No project selected"))?,
    };

    let window = match window_name {
        Some(n) => n,
        None => ui::input("Window", "Window name...", None)?
            .ok_or_else(|| anyhow::anyhow!("Window name is required"))?,
    };

    let command = if command.is_empty() {
        ui::input("Command", "Command to run...", None)?
            .ok_or_else(|| anyhow::anyhow!("Command is required"))?
    } else {
        command.join(" ")
    };

    let project = Project::load(&name)?;
    let session_name = name.clone();

    if project.name != session_name {
        eprintln!(
            "Warning: project config name '{}' differs from requested session '{}'",
            project.name, session_name
        );
    }

    let socket_path = socket.or_else(|| {
        env::var("TMUX")
            .ok()
            .and_then(|value| value.split(',').next().map(|part| part.to_string()))
            .filter(|value| !value.is_empty())
    });

    let session_exists = match socket_path.as_deref() {
        Some(path) => tmux::session_exists_with_socket(&session_name, path)?,
        None => tmux::session_exists(&session_name)?,
    };

    if !session_exists {
        anyhow::bail!("Session '{}' is not running", session_name);
    }

    let mut client = match socket_path.as_deref() {
        Some(path) => ControlClient::connect_with_socket_path(path)?,
        None => ControlClient::connect(None)?,
    };

    client.new_window(&session_name, &window, &project.root_expanded())?;
    let target = format!("{}:{}", session_name, window);
    client.send_keys(&target, &command, true)?;

    println!(
        "Created window '{}' and started command in session '{}'",
        window, session_name
    );

    Ok(())
}

pub fn run(
    project_name: Option<String>,
    window: String,
    command: Vec<String>,
    socket: Option<String>,
) -> Result<()> {
    let socket_path = socket.or_else(|| {
        env::var("TMUX")
            .ok()
            .and_then(|value| value.split(',').next().map(|part| part.to_string()))
            .filter(|value| !value.is_empty())
    });

    let name = match project_name {
        Some(n) => n,
        None => match socket_path.as_deref() {
            Some(path) => tmux::current_session_name_with_socket(path)
                .ok_or_else(|| anyhow::anyhow!("No project selected"))?,
            None => tmux::current_session_name().ok_or_else(|| {
                anyhow::anyhow!("No project selected; use --project or run inside tmux")
            })?,
        },
    };

    let command = if command.is_empty() {
        ui::input("Command", "Command to run...", None)?
            .ok_or_else(|| anyhow::anyhow!("Command is required"))?
    } else {
        command.join(" ")
    };

    let project = Project::load(&name)?;
    let session_name = name.clone();

    if project.name != session_name {
        eprintln!(
            "Warning: project config name '{}' differs from requested session '{}'",
            project.name, session_name
        );
    }

    let session_exists = match socket_path.as_deref() {
        Some(path) => tmux::session_exists_with_socket(&session_name, path)?,
        None => tmux::session_exists(&session_name)?,
    };

    if !session_exists {
        anyhow::bail!("Session '{}' is not running", session_name);
    }

    let mut client = match socket_path.as_deref() {
        Some(path) => ControlClient::connect_with_socket_path(path)?,
        None => ControlClient::connect(None)?,
    };

    let target = format!("{}:{}", session_name, window);
    client.split_window(&target, &project.root_expanded())?;
    client.send_keys(&target, &command, true)?;

    println!(
        "Started command in new pane for session '{}' window '{}'",
        session_name, window
    );

    Ok(())
}

pub fn list_panes(
    project_name: Option<String>,
    window: String,
    socket: Option<String>,
    json: bool,
) -> Result<()> {
    let socket_path = socket.or_else(|| {
        env::var("TMUX")
            .ok()
            .and_then(|value| value.split(',').next().map(|part| part.to_string()))
            .filter(|value| !value.is_empty())
    });

    let name = match project_name {
        Some(n) => n,
        None => match socket_path.as_deref() {
            Some(path) => tmux::current_session_name_with_socket(path)
                .ok_or_else(|| anyhow::anyhow!("No project selected"))?,
            None => tmux::current_session_name().ok_or_else(|| {
                anyhow::anyhow!("No project selected; use --project or run inside tmux")
            })?,
        },
    };

    let project = Project::load(&name)?;
    let session_name = name.clone();

    if project.name != session_name {
        eprintln!(
            "Warning: project config name '{}' differs from requested session '{}'",
            project.name, session_name
        );
    }

    let session_exists = match socket_path.as_deref() {
        Some(path) => tmux::session_exists_with_socket(&session_name, path)?,
        None => tmux::session_exists(&session_name)?,
    };

    if !session_exists {
        anyhow::bail!("Session '{}' is not running", session_name);
    }

    let mut client = match socket_path.as_deref() {
        Some(path) => ControlClient::connect_with_socket_path(path)?,
        None => ControlClient::connect(None)?,
    };

    let target = format!("{}:{}", session_name, window);
    let panes = client.list_panes(&target)?;

    if json {
        let mut entries = Vec::new();
        for pane in panes {
            let parts: Vec<&str> = pane.split('\t').collect();
            if parts.len() < 4 {
                continue;
            }
            entries.push(serde_json::json!({
                "index": parts[0],
                "id": parts[1],
                "command": parts[2],
                "path": parts[3],
            }));
        }

        println!(
            "{}",
            serde_json::to_string_pretty(&entries).context("Failed to serialize JSON output")?
        );
        return Ok(());
    }

    if panes.is_empty() {
        println!("No panes found for window '{}'", window);
        return Ok(());
    }

    for pane in panes {
        println!("{}", pane);
    }

    Ok(())
}
