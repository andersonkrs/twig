use anyhow::Result;
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

    let socket_path = socket.or_else(|| {
        env::var("TMUX")
            .ok()
            .and_then(|value| value.split(',').next().map(|part| part.to_string()))
            .filter(|value| !value.is_empty())
    });

    let session_exists = match socket_path.as_deref() {
        Some(path) => tmux::session_exists_with_socket(&project.name, path)?,
        None => tmux::session_exists(&project.name)?,
    };

    if !session_exists {
        anyhow::bail!("Session '{}' is not running", project.name);
    }

    let mut client = match socket_path.as_deref() {
        Some(path) => ControlClient::connect_with_socket_path(path)?,
        None => ControlClient::connect(None)?,
    };
    client.new_window(&project.name, &window, &project.root_expanded())?;

    println!("Created window '{}' in session '{}'", window, project.name);

    Ok(())
}
