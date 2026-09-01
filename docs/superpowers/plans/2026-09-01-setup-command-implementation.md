# `twig setup` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `twig setup [project] [branch]` command and a matching `x` keybinding in the tree-view TUI that bring an existing (or partially-existing) project/worktree up to date — creating the worktree/session if missing, or re-running `post_create` commands in a scratch window if the session is already running.

**Architecture:** A new `ControlClient::kill_window` primitive; a refactor of `SessionBuilder`'s post-create command loop into a shared helper reused by a new `run_post_create_in_scratch_window`; a new `cli/setup.rs` with the core `ensure_setup` resolution logic used by both the CLI command and the TUI `x` key.

**Tech Stack:** Rust, clap (CLI parsing), ratatui/crossterm (TUI), tmux control mode.

---

## Design clarification vs. spec

The spec (`docs/superpowers/specs/2026-08-31-setup-command-design.md`) says CLI
argument resolution "mirrors `worktree create`" when args are omitted. That
would force a branch prompt even when the user wants to set up a project's
main session (no worktree). This plan resolves that ambiguity concretely:

- `twig setup <project> <branch>` — both given, no prompts.
- `twig setup <project>` — branch omitted means the **project's main
  session**, not "prompt for branch". No prompt.
- `twig setup` (nothing given) — use `ui::select_project_or_worktree`, which
  already lets the user pick either a bare project or a `project / branch`
  worktree in one picker (existing function in `src/ui.rs:902`, currently
  unused anywhere in the codebase).

## File changes

- Modify: `src/tmux_control.rs` — add `ControlClient::kill_window`.
- Modify: `src/tmux.rs` — extract shared post-create command loop; add
  `SessionBuilder::run_post_create_in_scratch_window` and
  `SessionBuilder::has_post_create_commands`.
- Create: `src/cli/setup.rs` — `run()` (CLI entry, handles prompts/printing)
  and `ensure_setup()` (core resolution logic, reused by the TUI).
- Modify: `src/cli/mod.rs` — register `pub mod setup;`.
- Modify: `src/main.rs` — add `Commands::Setup` and dispatch it.
- Modify: `src/cli/tree_view.rs` — add `x`/`X` key handling,
  `HandleResult::SetupSession`, `setup_session_for_action`.

---

### Task 1: `ControlClient::kill_window`

**Files:**
- Modify: `src/tmux_control.rs:350-354` (add method after `kill_session`)
- Test: `src/tmux_control.rs` (`mod tests`, alongside `test_control_new_window`)

- [ ] **Step 1: Add the method**

In `src/tmux_control.rs`, right after `kill_session` (around line 354), add:

```rust
    pub fn kill_window(&mut self, target: &str) -> Result<()> {
        let command = format!("kill-window -t {}", quote_tmux_arg(target));
        self.command(&command)?;
        Ok(())
    }
```

- [ ] **Step 2: Write the integration test**

In `src/tmux_control.rs`, inside `mod tests` (after `test_control_new_window`,
around line 543), add:

```rust
    #[test]
    fn test_control_kill_window() {
        if !tmux_available() {
            eprintln!("tmux not available, skipping control mode test");
            return;
        }

        let server = unique_server_name();
        let _guard = ServerGuard::new(server.clone());
        let session = "twig_test_session";
        let window = "extra";

        let mut client = match ControlClient::connect_new_session(Some(&server)) {
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

        let target = format!("{}:{}", session, window);
        if let Err(err) = client.kill_window(&target) {
            eprintln!("failed to kill test window: {err}");
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
            !windows.iter().any(|name| name == window),
            "expected window '{}' to be gone, found {:?}",
            window,
            windows
        );
    }
```

- [ ] **Step 3: Run the test**

Run: `cargo test --lib tmux_control::tests::test_control_kill_window -- --nocapture`
Expected: PASS (or a `skipping control mode test` message if `tmux` isn't on
PATH in this environment — either is acceptable, but if `tmux` is available
it must PASS).

- [ ] **Step 4: Commit**

```bash
git add src/tmux_control.rs
git commit -m "feat(tmux): add ControlClient::kill_window"
```

---

### Task 2: Scratch-window post-create re-run in `SessionBuilder`

**Files:**
- Modify: `src/tmux.rs:638-658` (`run_post_create_with_control`)

- [ ] **Step 1: Extract the shared command loop**

Replace the existing `run_post_create_with_control` (lines 638-658) with:

```rust
    pub fn run_post_create_with_control(&self, client: &mut ControlClient) -> Result<()> {
        if self.post_create_commands.is_empty() {
            return Ok(());
        }

        let target = format!("{}:{}", self.session_name, SETUP_WINDOW_NAME);
        self.send_commands_with_wait(client, &target)
    }

    /// Run `post_create` commands in a temporary window created in the
    /// current (already-running) session, then remove that window.
    /// No-op if there are no `post_create` commands configured.
    pub fn run_post_create_in_scratch_window(&self) -> Result<()> {
        if self.post_create_commands.is_empty() {
            return Ok(());
        }

        let mut client = ControlClient::connect(None)?;
        let root_expanded = PathBuf::from(shellexpand::tilde(&self.root).to_string());
        let window_name = format!("twig-setup-{}", std::process::id());

        client.new_window(&self.session_name, &window_name, &root_expanded)?;
        let target = format!("{}:{}", self.session_name, window_name);

        let result = self.send_commands_with_wait(&mut client, &target);

        // Best-effort cleanup: remove the scratch window even if a command failed.
        let _ = client.kill_window(&target);

        result
    }

    /// True if this project has any `post_create` commands configured.
    pub fn has_post_create_commands(&self) -> bool {
        !self.post_create_commands.is_empty()
    }

    fn send_commands_with_wait(&self, client: &mut ControlClient, target: &str) -> Result<()> {
        for (index, command) in self.post_create_commands.iter().enumerate() {
            let trimmed = command.trim();
            if trimmed.is_empty() {
                continue;
            }

            let token = unique_wait_token(&self.session_name, index);
            let signal = format!("{}; tmux wait-for -S {}", trimmed, token);
            client.send_keys(target, &signal, true)?;
            client.wait_for(&token)?;
        }

        Ok(())
    }
```

- [ ] **Step 2: Check `PathBuf` is imported**

`src/tmux.rs:3` already has `use std::path::PathBuf;` — no import change needed.

- [ ] **Step 3: Build**

Run: `cargo build`
Expected: builds with no errors. `run_post_create_in_scratch_window` and
`has_post_create_commands` will show `dead_code` warnings until Task 3 wires
them in — that's expected at this point, don't suppress it.

- [ ] **Step 4: Commit**

```bash
git add src/tmux.rs
git commit -m "feat(tmux): add run_post_create_in_scratch_window"
```

---

### Task 3: `cli/setup.rs` and the `twig setup` command

**Files:**
- Create: `src/cli/setup.rs`
- Modify: `src/cli/mod.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Create `src/cli/setup.rs`**

```rust
use anyhow::Result;

use crate::config::Project;
use crate::git;
use crate::tmux::{self, SessionBuilder};
use crate::ui;

/// `twig setup [project] [branch]`
pub fn run(project_name: Option<String>, branch: Option<String>) -> Result<()> {
    let (name, branch) = match project_name {
        Some(n) => (n, branch),
        None => ui::select_project_or_worktree("Select project or worktree to set up...")?
            .ok_or_else(|| anyhow::anyhow!("No project selected"))?,
    };

    let project = Project::load(&name)?;
    let message = ensure_setup(&project, branch.as_deref())?;
    println!("{}", message);

    Ok(())
}

/// Bring `project` (optionally a `branch` worktree of it) up to date:
/// create the worktree/session if missing, or re-run `post_create`
/// commands in a scratch window if the session is already running.
/// Returns a human-readable summary of what happened.
pub fn ensure_setup(project: &Project, branch: Option<&str>) -> Result<String> {
    let (session_name, root, worktree_branch) = match branch {
        Some(branch) => {
            let worktrees = git::list_worktrees(project)?;
            let worktree_path = match worktrees.iter().find(|wt| wt.branch == branch) {
                Some(wt) => wt.path.clone(),
                None => {
                    println!("Creating worktree '{}'...", branch);
                    git::create_worktree(project, branch)?
                }
            };

            (
                project.worktree_session_name(branch),
                worktree_path.to_string_lossy().to_string(),
                Some(branch.to_string()),
            )
        }
        None => (project.name.clone(), project.root.clone(), None),
    };

    if !tmux::session_exists(&session_name)? {
        project.clone_if_needed()?;
        println!("Starting session '{}'...", session_name);

        let mut builder = SessionBuilder::new(project)
            .with_session_name(session_name.clone())
            .with_root(root);

        if let Some(branch) = worktree_branch {
            builder = builder.with_worktree(branch);
        }

        builder.start_with_control()?;

        return Ok(format!("Started session '{}' and ran setup.", session_name));
    }

    let mut builder = SessionBuilder::new(project)
        .with_session_name(session_name.clone())
        .with_root(root);

    if let Some(branch) = worktree_branch {
        builder = builder.with_worktree(branch);
    }

    if !builder.has_post_create_commands() {
        return Ok(format!(
            "Nothing to set up for '{}' (no post_create commands configured).",
            session_name
        ));
    }

    println!("Running post-create tasks for '{}'...", session_name);
    builder.run_post_create_in_scratch_window()?;

    Ok(format!("Ran post-create tasks for '{}'.", session_name))
}
```

- [ ] **Step 2: Register the module**

In `src/cli/mod.rs`, add `pub mod setup;` alphabetically (after `pub mod
new;`, before `pub mod start;`):

```rust
pub mod delete;
pub mod edit;
pub mod kill;
pub mod list;
pub mod new;
pub mod setup;
pub mod start;
pub mod tree_view;
pub mod window;
pub mod worktree;
```

- [ ] **Step 3: Add the CLI subcommand**

In `src/main.rs`, add a new variant to `Commands` (after `Stop`, before
`Run`, around line 68):

```rust
    /// Set up an existing project/worktree: create what's missing, or
    /// re-run post-create tasks if the session is already running
    Setup {
        /// Project name (interactive selection if not provided)
        project: Option<String>,
        /// Worktree branch name (omit for the project's main session)
        branch: Option<String>,
    },
```

And dispatch it in the `match cli.command` block (after the `Commands::Stop`
arm, around line 203):

```rust
        Commands::Setup { project, branch } => cli::setup::run(project, branch),
```

- [ ] **Step 4: Build**

Run: `cargo build`
Expected: builds with no errors, and the `dead_code` warnings from Task 3
for `run_post_create_in_scratch_window` / `has_post_create_commands` are
now gone (they're used by `ensure_setup`).

- [ ] **Step 5: Sanity-check the CLI wiring**

Run: `cargo run -- setup --help`
Expected: help text shows `[PROJECT] [BRANCH]` args and the doc comment
above as the description.

- [ ] **Step 6: Commit**

```bash
git add src/cli/setup.rs src/cli/mod.rs src/main.rs
git commit -m "feat(cli): add twig setup command"
```

---

### Task 4: `x` keybinding in the tree-view TUI

**Files:**
- Modify: `src/cli/tree_view.rs`

- [ ] **Step 1: Add the `HandleResult` variant**

In `src/cli/tree_view.rs`, in the `enum HandleResult` block (around line
754-775), add a variant after `ActivateSession`:

```rust
    /// Activate handoff windows on selected session
    ActivateSession(SelectedAction),
    /// Set up selected project/worktree: create what's missing, or re-run
    /// post-create tasks if the session is already running
    SetupSession(SelectedAction),
    /// Attach directly to an orphaned session (exits tree view)
    AttachOrphaned(String),
```

- [ ] **Step 2: Add the key handler**

In `handle_key` (around line 314-320), add a handler for `x`/`X` right after
the `a`/`A` (activate) arm:

```rust
            // Activate handoff windows on selected session
            KeyCode::Char('a') | KeyCode::Char('A') => {
                if self.mode == TreeViewMode::Start {
                    if let Some(action) = self.get_selected_action() {
                        return Some(HandleResult::ActivateSession(action));
                    }
                }
            }

            // Set up (or re-run post-create tasks on) selected session
            KeyCode::Char('x') | KeyCode::Char('X') => {
                if self.mode == TreeViewMode::Start {
                    if let Some(action) = self.get_selected_action() {
                        return Some(HandleResult::SetupSession(action));
                    }
                }
            }
```

- [ ] **Step 3: Handle the result in the event loop**

In the event loop `match result` block (around line 1186-1196), add a case
right after `HandleResult::ActivateSession`:

```rust
                            HandleResult::ActivateSession(action) => {
                                match activate_session_for_action(action) {
                                    Ok(message) => {
                                        app.status_message = Some(StatusMessage::info(message));
                                    }
                                    Err(err) => {
                                        app.status_message =
                                            Some(StatusMessage::error(err.to_string()));
                                    }
                                }
                            }
                            HandleResult::SetupSession(action) => {
                                match setup_session_for_action(action) {
                                    Ok(message) => {
                                        app.status_message = Some(StatusMessage::info(message));
                                    }
                                    Err(err) => {
                                        app.status_message =
                                            Some(StatusMessage::error(err.to_string()));
                                    }
                                }
                            }
```

- [ ] **Step 4: Add `setup_session_for_action`**

Right after `activate_session_for_action` (around line 1256-1279), add:

```rust
fn setup_session_for_action(action: SelectedAction) -> Result<String> {
    match action {
        SelectedAction::StartProject(name) => {
            let project = Project::load(&name)?;
            crate::cli::setup::ensure_setup(&project, None)
        }
        SelectedAction::StartWorktree { project, branch } => {
            let config = Project::load(&project)?;
            crate::cli::setup::ensure_setup(&config, Some(&branch))
        }
        _ => anyhow::bail!("Select a project or worktree to set up"),
    }
}
```

- [ ] **Step 5: Build**

Run: `cargo build`
Expected: builds with no errors.

- [ ] **Step 6: Run existing tests**

Run: `cargo test --lib`
Expected: all existing tests still PASS (this task doesn't add new ones —
`setup_session_for_action` and the key handler follow the exact untested
pattern already used by `activate_session_for_action` and the `a` key in
this file).

- [ ] **Step 7: Commit**

```bash
git add src/cli/tree_view.rs
git commit -m "feat(tree-view): add x key to run setup on selected session"
```

---

### Task 5: Manual verification

**Files:** none (verification only)

- [ ] **Step 1: Full build + clippy + test pass**

Run: `cargo build && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: all green.

- [ ] **Step 2: Manual run**

Use the `run` skill (or `cargo run -- tree list`) against a real or scratch
`twig` project that has `worktree.post_create` commands configured:

1. Pick a worktree with a **running** session, press `x` — confirm the
   status line shows `Ran post-create tasks for '<session>'.` and that the
   session's existing windows are untouched (no new permanent window left
   behind).
2. Pick a project/worktree with **no running session**, press `x` — confirm
   it starts the session (same as pressing `s`/Enter would) and reports
   `Started session '<session>' and ran setup.`.
3. Run `twig setup <project>` and `twig setup <project> <branch>` from the
   shell and confirm matching output.

- [ ] **Step 3: Update the after-help text if needed**

If manual verification surfaces anything worth documenting for users (e.g.
`twig setup --help` wording), fix it and re-run Step 1.

---

## Self-review notes

- **Spec coverage:** CLI command (Task 3), TUI `x` key (Task 4), "skip
  session/worktree creation if it exists" (Task 3's `ensure_setup`),
  scratch-window mechanism (Task 2), "nothing to set up" case (Task 3),
  window-layout re-run explicitly out of scope (not touched by any task) —
  all covered.
- **Ambiguity resolved:** see "Design clarification vs. spec" above —
  branch is optional and un-prompted, not forced like `worktree create`.
- **Type consistency:** `ensure_setup(project: &Project, branch: Option<&str>) -> Result<String>`
  is the one signature used everywhere (Task 3's `run`, Task 4's
  `setup_session_for_action`) — no divergent copies.
