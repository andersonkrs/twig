# twig

A tmux session manager with git worktree support, inspired by [tmuxinator](https://github.com/tmuxinator/tmuxinator).

Built to scratch my own itch. Terminal UI built with [Ratatui](https://ratatui.rs/).

When you are juggling features, fixes, and reviews, [git worktree](https://git-scm.com/docs/git-worktree)
lets you keep multiple branches checked out side by side. Twig ties each worktree to a tmux
session and provides a snappy TUI so you can spin up a clean, focused workspace per branch in
seconds.


https://github.com/user-attachments/assets/95e21d30-a055-4c1b-b0b6-dcccb61dd53e


## Requirements

- `tmux`
- `git`

## Installation

We recommend using [mise](https://mise.jdx.dev/) to install.

### Via mise

```bash
mise use -g cargo:https://github.com/andersonkrs/twig
```

This compiles twig from source and installs it globally.

### From source (for development)

```bash
git clone https://github.com/andersonkrs/twig.git ~/Work/twig
cd ~/Work/twig

# Install all tools (rust, lefthook) + git hooks
mise install

# Build
cargo build --release

# Symlink to PATH
ln -s ~/Work/twig/target/release/twig ~/.local/bin/twig
```

## Usage

```bash
twig start [project]     # Start/attach to session (interactive if no arg)
twig list                # List all projects/worktrees
twig list --focus-current # Focus current TWIG_PROJECT/TWIG_WORKTREE
twig new [name|repo_url] # Create new project (accepts name or git URL)
twig edit [project]      # Open config in $EDITOR
twig delete [project]    # Delete project config
twig stop [project]      # Kill tmux session

# Debug tmux control-mode I/O
Use `--verbose` (or `TWIG_DEBUG=1`) to enable verbose tmux control output on stderr.
twig --verbose window new [project] [name]

# Run a command in a window/pane
twig run --project=dotfiles --window=6 --pane=1 -- whoami

# Run a command in a worktree session
twig run --project=dotfiles --tree=feature-x --window=1 -- btop

# Worktree commands
twig tree create [project] [branch]   # Create worktree + session
twig tree list [project]              # List worktrees
twig tree delete [project] [branch]   # Delete worktree + kill session
```

When creating a project with a git URL, twig extracts the project name automatically:
```bash
twig new git@github.com:user/myproject.git  # Creates project "myproject"
```

Aliases: `ls` for `list`, `s` for `start`, `n` for `new`, `e` for `edit`, `rm` for `delete`, `t` for `tree`

## Configuration

### Global Config

Location: `~/.config/twig/config.yml`

```yaml
# Base path for worktrees (default: ~/Work/.trees)
# Worktrees are created at: {worktree_base}/{project}/{branch}
worktree_base: ~/Work/.trees

# Projects directory (default: ~/.config/twig/projects)
projects_dir: ~/.config/twig/projects
```

### Project Config

Location: `~/.config/twig/projects/<name>.yml`

```yaml
name: myproject
root: ~/Work/myproject

# Optional: git repo URL (https or ssh)
# If root doesn't exist, twig will clone this repo on first start
repo: git@github.com:user/myproject.git

windows:
  # Simple window with command
  - git: lazygit

  # Empty shell window
  - shell:

  # Window with multiple panes
  - editor:
      panes:
        - nvim

  # Window with layout and multiple panes
  - servers:
      layout: main-vertical    # main-vertical, main-horizontal, even-vertical, even-horizontal, tiled
      panes:
        - rails server
        - bin/sidekiq

# Optional: worktree configuration
worktree:
  # Files/folders to copy from parent project to worktree
  copy:
    - .env
    - .env.local
    - config/master.key

  # Files/folders to symlink from parent project to worktree
  # Only supported on Unix
  symlink:
    - .env

  # Commands to run after worktree creation
  post_create:
    - bundle install
    - yarn install
    - rails db:migrate

  # Note: post_create runs inside a temporary setup window in the worktree session
  # so your shell init and environment (mise/rbenv/etc) are applied.
```

### Example Configs

**Rails project:**
```yaml
name: myapp
root: ~/Work/myapp

windows:
  - editor:
      panes:
        - nvim
  - shell:
  - rails:
      layout: main-vertical
      panes:
        - rails server
        - bin/sidekiq
  - console: rails console
  - git: lazygit

worktree:
  copy:
    - .env
    - .env.local
    - config/master.key
    - config/credentials.yml.enc
  symlink:
    - .env
  post_create:
    - bundle install
    - yarn install
    - bin/rails db:prepare
```

**Simple project:**
```yaml
name: dotfiles
root: ~/.dotfiles

windows:
  - editor:
      panes:
        - nvim
  - shell:
  - shell:
  - git: lazygit
```

## How It Works

Twig is a thin Rust layer that turns YAML configs into tmux control-mode commands and
manages git worktrees when requested. The CLI orchestrates config loading, git worktree
creation, and tmux session construction; the TUI only renders state and triggers CLI
actions.

```text
YAML config
   |
   v
CLI (twig) ---> git worktree ops (optional)
   |
   v
tmux control mode -> tmux server -> sessions/windows/panes
```

```text
User input (TUI)
   |
   v
CLI commands -> tmux control mode
```

Tmux control protocol docs: https://man7.org/linux/man-pages/man1/tmux.1.html#CONTROL_MODE

### Sessions

When you run `twig start <project>`:

1. Checks if session already exists → attaches if so
2. Creates new tmux session with configured windows/panes
3. Runs commands in each pane
4. Attaches to the session (or switches if already in tmux)

### Worktrees

When you run `twig tree create <project> <branch>`:

1. Creates git worktree at `{worktree_base}/{project}/{branch}`
2. Creates the branch if it doesn't exist
3. Copies and symlinks configured files from parent project
4. Runs post-create commands
5. Starts a tmux session named `{project}__{branch}`

Session naming: `myproject__feature-auth` (double underscore separator)

Worktree path: `~/Work/.trees/myproject/feature-auth`

When you run `twig tree delete <project> <branch>`:

1. Kills the tmux session if running
2. Removes the git worktree

## Tmux Popup Session Picker

You can replace the tmux session picker with a popup that calls `twig ls --focus-current`.
This uses the `TWIG_PROJECT` and `TWIG_WORKTREE` environment variables to focus the cursor
on the current project/worktree when available.

Add a key binding to your `~/.tmux.conf`:

```tmux
# Twig popup
unbind s
bind-key s display-popup -E -w 80% -h 60% "twig ls --focus-current"
```

If you want the popup to always open from anywhere (not just inside a twig session), it
will still work but will fall back to the first project when the env vars are not set.

## Releases

- Releases are managed by release-plz using conventional commits to determine the bump.
- A release PR is created/updated on every push to `main`.
- Merge the release PR to tag and publish a GitHub release; binaries are uploaded for
  linux x86_64 and macOS universal2.

## Development

```bash
# Install dependencies + git hooks
mise install

# Build
cargo build --release
```

Formatting and linting are automatically run by lefthook on pre-commit.

## License

MIT
