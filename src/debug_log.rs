use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_PREVIEW_LEN: usize = 400;

static LOG_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub fn log_tmux_command(args: &[&str]) {
    write_entry(&format!("[tmux] >> {}", command_to_string(args)));
}

pub fn log_tmux_command_result(args: &[&str], status: i32, stdout: &[u8], stderr: &[u8]) {
    write_entry(&format!(
        "[tmux] << {} (exit {}) stdout={} stderr={}",
        command_to_string(args),
        status,
        summarize_bytes(stdout),
        summarize_bytes(stderr)
    ));
}

pub fn log_tmux_command_failure(args: &[&str], reason: &str) {
    write_entry(&format!(
        "[tmux] !! {} failed: {}",
        command_to_string(args),
        reason
    ));
}

pub fn log_tmux_control(direction: &str, message: &str) {
    write_entry(&format!("[tmux-control] {} {}", direction, message));
}

fn write_entry(message: &str) {
    let path = log_file_path();
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return;
        }
    }

    let Ok(lock) = LOG_LOCK.get_or_init(|| Mutex::new(())).lock() else {
        return;
    };

    let line = format!("{} {}\n", timestamp(), message);

    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };

    let _ = file.write_all(line.as_bytes());
    drop(lock);
}

fn timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    format!("{}.{:09}", now.as_secs(), now.subsec_nanos())
}

fn command_to_string(args: &[&str]) -> String {
    args.iter()
        .map(|arg| {
            let escaped = arg.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{}\"", escaped)
        })
        .collect::<Vec<String>>()
        .join(" ")
}

fn summarize_bytes(data: &[u8]) -> String {
    if data.is_empty() {
        return "<empty>".to_string();
    }

    let text = String::from_utf8_lossy(data)
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    let mut shortened: String = text.chars().take(DEFAULT_PREVIEW_LEN).collect();
    if text.chars().count() > DEFAULT_PREVIEW_LEN {
        shortened.push_str("...");
    }

    shortened
}

fn log_file_path() -> PathBuf {
    if let Ok(path) = env::var("TWIG_LOG_FILE") {
        return PathBuf::from(path);
    }

    let mut path = env::temp_dir();
    path.push("twig");
    path.push("twig.log");
    path
}
