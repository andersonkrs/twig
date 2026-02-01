# AGENTS.md - Coding Agent Instructions for twig

## Project Overview

**twig** is a tmux session manager with git worktree support, written in Rust.
It manages tmux sessions based on YAML project configs, supports git worktrees with
automatic file copying and post-create commands, and provides a TUI for interactive
terminal workflows.

## Build/Lint/Test Commands

```bash
# Build
cargo build                  # Debug build
cargo build --release        # Release build

# Test
cargo test                   # Run all tests
cargo test <test_name>       # Run single test by name (e.g., cargo test test_name_from_https_url)
cargo test <module>::       # Run all tests in a module (e.g., cargo test tests::)

# Lint and Format
cargo fmt                    # Format code
cargo fmt -- --check         # Check formatting without changes
cargo clippy --all-targets --all-features -- -D warnings  # Run clippy with strict warnings

# Development setup
mise install                 # Install all tools (rust, gum, lefthook) + git hooks
```

### Git Hooks (via Lefthook)

- **pre-commit**: Runs `cargo fmt -- --check` and `cargo clippy --all-targets --all-features -- -D warnings`
- **pre-push**: Runs `cargo test` and `cargo build --release`

## Directory Structure

```
twig/
├── src/
│   ├── main.rs             # Entry point, CLI definition (clap)
│   ├── cli/                # CLI command handlers
│   │   ├── mod.rs
│   │   ├── delete.rs
│   │   ├── edit.rs
│   │   ├── kill.rs
│   │   ├── list.rs
│   │   ├── new.rs
│   │   ├── start.rs
│   │   ├── tree_view.rs
│   │   ├── window.rs
│   │   └── worktree.rs
│   ├── config/             # Configuration types
│   │   ├── mod.rs
│   │   ├── global.rs       # GlobalConfig
│   │   └── project.rs      # Project, Window, Pane types
│   ├── git.rs              # Git worktree operations
│   └── tmux.rs             # Tmux session management
│   ├── tmux_control.rs      # Low-level tmux control helpers
│   └── ui.rs                # TUI rendering
├── Cargo.toml
├── rustfmt.toml            # Max width 100, 4 spaces
├── clippy.toml
└── lefthook.yml
```

## Code Style Guidelines

### Imports

Order imports in groups, separated by blank lines:

1. Standard library (`std::`)
2. External crates (`anyhow::`, `serde::`, `clap::`, etc.)
3. Internal modules (`crate::`, `super::`)

```rust
use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::config::Project;
use super::GlobalConfig;
```

### Error Handling

- Use `anyhow::Result<T>` as return type for fallible functions
- Use `.context()` or `.with_context()` for adding context to errors
- Use `anyhow::bail!()` for early returns with error messages
- Use `anyhow::anyhow!()` to create ad-hoc errors

```rust
let contents = fs::read_to_string(&path)
    .with_context(|| format!("Failed to read project: {:?}", path))?;

if !status.success() {
    anyhow::bail!("Failed to attach to session: {}", name);
}
```

### Naming Conventions

- Functions: `snake_case` (e.g., `create_worktree`, `session_exists`)
- Types/Structs: `PascalCase` (e.g., `GlobalConfig`, `SessionBuilder`)
- Constants: `SCREAMING_SNAKE_CASE` with `Lazy` for computed statics
- Module files: `snake_case.rs`

### Types and Structs

- Derive `Debug`, `Deserialize`, `Clone` on data types
- Use `#[serde(default)]` for optional fields with defaults
- Use `#[serde(untagged)]` for enum variants that share serialization format
- Use `#[serde(flatten)]` to flatten nested structures

```rust
#[derive(Debug, Deserialize, Clone)]
pub struct Project {
    pub name: String,
    #[serde(default)]
    pub windows: Vec<Window>,
}
```

### Function Patterns

- Public entry points in CLI modules: `pub fn run(...) -> Result<()>`
- Helper functions are private: `fn select_project() -> Result<String>`
- Use builder pattern for complex construction: `SessionBuilder::new(&project).build()?`
- Short functions that do one thing well

### Formatting

- Max line width: 100 characters
- 4 spaces indentation (no tabs)
- Follow rustfmt defaults for Edition 2021

### Documentation

- Use `///` doc comments for public functions
- Use inline `//` comments for complex logic
- Keep comments concise and meaningful

### Patterns for Command Execution

```rust
let status = Command::new("git")
    .args(["clone", repo_url, &path.to_string_lossy()])
    .status()
    .context("Failed to run git clone")?;

if !status.success() {
    anyhow::bail!("git clone failed for {}", repo_url);
}
```

### Regex with Lazy Static

```rust
use once_cell::sync::Lazy;
use regex::Regex;

static GIT_URL_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r"^https?://[^/]+/(?:.+/)?([^/]+?)(?:\.git)?$").unwrap(),
    ]
});
```

### Testing

- Tests are inline in source files using `#[cfg(test)]` modules
- Test function names: `test_<what_is_being_tested>`
- Use `assert_eq!` for equality checks, `assert!` for boolean conditions

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name_from_https_url() {
        assert_eq!(
            Project::name_from_repo_url("https://github.com/user/repo.git"),
            Some("repo".to_string())
        );
    }
}
```

## Dependencies

| Crate | Purpose |
|-------|---------|
| clap | CLI argument parsing (derive feature) |
| serde + serde_yaml | YAML config serialization |
| serde_json | JSON serialization |
| anyhow | Error handling |
| shellexpand | Path expansion (~) |
| dirs | Standard directories |
| regex + once_cell | Lazy static regex patterns |
| ratatui + crossterm | Terminal UI rendering |
| tui-tree-widget | Tree view UI component |
| fuzzy-matcher | Fuzzy matching for search |
