# `twig setup` — Re-run Post-Tree-Creation Tasks

## Problem

Creating a project session or worktree runs `post_create` commands (from
`worktree.post_create` in the project config) and window setup, once, at
creation time. There's no way to re-run those tasks against a tree that
already exists — e.g. after pulling changes that touch `Gemfile`/`package.json`,
or after editing `post_create` in the config.

## Goal

Add a `twig setup` command, plus an `x` keybinding in the tree-view TUI, that
brings an existing (or partially-existing) project/worktree up to date by
running the same setup a fresh creation would run, skipping steps that are
already done.

## CLI

```
twig setup [project] [branch]
```

- `project` / `branch`: same interactive selection fallback as `worktree create`
  (project picker, then branch name input) when omitted.
- Non-destructive: never deletes or recreates an existing worktree or session.

## Behavior

1. Load the project config.
2. If `branch` is given and no worktree for that branch exists yet, create it
   (`git::create_worktree`), same as `worktree create`.
3. Resolve the target session name (`project.name` or `project__branch`).
4. **If the session does not exist**: run the existing full creation path,
   `SessionBuilder::start_with_control()` (create session, set env, run
   `post_create`, set up windows). This is unchanged behavior — just reached
   from a new entry point.
5. **If the session already exists**: run only the `post_create` commands,
   in a temporary scratch window created for the purpose, then remove that
   window. This is the new code path and the main point of the feature —
   applying `post_create` to a tree that's already up and running, without
   disturbing existing windows/panes.
6. If the project has no `post_create` commands configured and the session
   already exists, report "Nothing to set up" instead of running or erroring.

## Implementation

### `tmux.rs`: `SessionBuilder`

New method, alongside `run_post_create_with_control`:

```rust
pub fn run_post_create_in_scratch_window(&self) -> Result<()>
```

- Returns early (no-op) if `post_create_commands` is empty.
- Connects a `ControlClient`.
- Creates a temp window (`twig-setup-x`) in the target session.
- Sends each `post_create` command to that window and waits for it via the
  same `wait-for` token mechanism `run_post_create_with_control` already
  uses (refactor the send/wait loop into a shared helper taking a `target`
  string, called by both methods).
- Kills the temp window when done. Uses a `Result`-returning cleanup that
  still attempts the kill on a command failure (best-effort), then surfaces
  the original error.

### `cli/setup.rs` (new file)

```rust
pub fn run(project_name: Option<String>, branch: Option<String>) -> Result<()>
```

Implements the resolution steps above, printing progress the way
`worktree.rs` does (`Creating worktree...`, `Running post-create tasks for
'<session>'...`, `Nothing to set up.`, etc).

### `main.rs`

New `Commands::Setup { project: Option<String>, branch: Option<String> }`
variant, dispatched to `cli::setup::run`.

### `cli/tree_view.rs`

- New `HandleResult::SetupSession(SelectedAction)` variant.
- `x` / `X` key handler in `TreeViewMode::Start`, using the same
  `get_selected_action()` pattern as `a`/`ActivateSession` — valid on
  project and worktree nodes only.
- New `setup_session_for_action(action: SelectedAction) -> Result<String>`
  next to `activate_session_for_action`, calling the same underlying logic
  as `cli::setup::run` (extract a shared function taking a resolved
  `Project` + optional branch, used by both the CLI command and the TUI, to
  avoid duplicating the resolution steps).
- Handled synchronously in the event loop like `ActivateSession`: result or
  error goes into `app.status_message`. No confirmation modal (non-destructive).

## Out of scope

- Re-running window layout setup (`setup_windows_with_control`) on an
  already-running session. Only `post_create` commands are re-run; existing
  windows/panes are left alone.
- Running setup against a stopped session without attaching (headless
  background session). Not requested; `start_with_control` already covers
  "session doesn't exist yet" by starting it normally.
