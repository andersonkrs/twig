use anyhow::{anyhow, Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};

/// Check if gum is installed, error with helpful message if not
pub fn check_gum_installed() -> Result<()> {
    Command::new("gum")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| {
            anyhow!(
                "gum is not installed. Please install it first:\n\n\
                 brew install gum     # macOS\n\
                 pacman -S gum        # Arch Linux\n\
                 go install github.com/charmbracelet/gum@latest  # Go\n\n\
                 See: https://github.com/charmbracelet/gum"
            )
        })?;
    Ok(())
}

/// Interactive filter selection from a list of items
pub fn filter(items: &[String], placeholder: &str) -> Result<Option<String>> {
    check_gum_installed()?;

    let mut child = Command::new("gum")
        .arg("filter")
        .arg("--placeholder")
        .arg(placeholder)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed to spawn gum filter")?;

    if let Some(stdin) = child.stdin.as_mut() {
        for item in items {
            writeln!(stdin, "{}", item)?;
        }
    }

    let output = child.wait_with_output()?;

    if output.status.success() {
        let selection = String::from_utf8(output.stdout)?.trim().to_string();
        if selection.is_empty() {
            Ok(None)
        } else {
            Ok(Some(selection))
        }
    } else {
        Ok(None) // User cancelled (Ctrl+C or Escape)
    }
}

/// Interactive single-line input
pub fn input(placeholder: &str, default: Option<&str>) -> Result<Option<String>> {
    check_gum_installed()?;

    let mut cmd = Command::new("gum");
    cmd.arg("input").arg("--placeholder").arg(placeholder);

    if let Some(val) = default {
        cmd.arg("--value").arg(val);
    }

    let output = cmd.stdout(Stdio::piped()).spawn()?.wait_with_output()?;

    if output.status.success() {
        let value = String::from_utf8(output.stdout)?.trim().to_string();
        if value.is_empty() {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    } else {
        Ok(None)
    }
}

/// Interactive confirmation prompt
pub fn confirm(prompt: &str) -> Result<bool> {
    check_gum_installed()?;

    let status = Command::new("gum")
        .arg("confirm")
        .arg(prompt)
        .status()
        .context("Failed to run gum confirm")?;

    Ok(status.success())
}

/// Interactive choice selection
#[allow(dead_code)]
pub fn choose(items: &[String], header: Option<&str>) -> Result<Option<String>> {
    check_gum_installed()?;

    let mut cmd = Command::new("gum");
    cmd.arg("choose");

    if let Some(h) = header {
        cmd.arg("--header").arg(h);
    }

    for item in items {
        cmd.arg(item);
    }

    let output = cmd.stdout(Stdio::piped()).spawn()?.wait_with_output()?;

    if output.status.success() {
        let selection = String::from_utf8(output.stdout)?.trim().to_string();
        if selection.is_empty() {
            Ok(None)
        } else {
            Ok(Some(selection))
        }
    } else {
        Ok(None)
    }
}

/// Display styled text
#[allow(dead_code)]
pub fn style(text: &str, args: &[(&str, &str)]) -> Result<String> {
    check_gum_installed()?;

    let mut cmd = Command::new("gum");
    cmd.arg("style");

    for (key, value) in args {
        cmd.arg(format!("--{}", key)).arg(value);
    }

    cmd.arg(text);

    let output = cmd.stdout(Stdio::piped()).spawn()?.wait_with_output()?;

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

/// Display a spinner while running a command
#[allow(dead_code)]
pub fn spin(title: &str, command: &str) -> Result<()> {
    check_gum_installed()?;

    let status = Command::new("gum")
        .arg("spin")
        .arg("--spinner")
        .arg("dot")
        .arg("--title")
        .arg(title)
        .arg("--")
        .arg("sh")
        .arg("-c")
        .arg(command)
        .status()
        .context("Failed to run gum spin")?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("Command failed: {}", command))
    }
}
