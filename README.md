# twig

A tmux session manager with git worktree support, inspired by [tmuxinator](https://github.com/tmuxinator/tmuxinator).

Built to scratch my own itch. Uses [Charmbracelet](https://charm.sh/) tools like [Gum](https://github.com/charmbracelet/gum) for glamorous terminal interactions.

## Requirements

- `tmux`
- `git`
- `gum` - [Charmbracelet Gum](https://github.com/charmbracelet/gum)

## Installation

We recommend using [mise](https://mise.jdx.dev/) to install.

### Via mise

```bash
mise use -g cargo:https://github.com/andersonkrs/twig gum
```

This compiles twig from source and installs gum globally.

### From source (for development)

```bash
git clone https://github.com/andersonkrs/twig.git ~/Work/twig
cd ~/Work/twig

# Install all tools (rust, gum, lefthook) + git hooks
mise install

# Build
cargo build --release

# Symlink to PATH
ln -s ~/Work/twig/target/release/twig ~/.local/bin/twig
```

## Usage

```bash
twig start [project]     # Start/attach to session (interactive if no arg)
twig list                # List all projects
twig new [name|repo_url] # Create new project (accepts name or git URL)
twig edit [project]      # Open config in $EDITOR
twig delete [project]    # Delete project config
twig stop [project]      # Kill tmux session

# Worktree commands
twig worktree create [project] [branch]   # Create worktree + session
twig worktree list [project]              # List worktrees
twig worktree delete [project] [branch]   # Delete worktree + kill session
```

When creating a project with a git URL, twig extracts the project name automatically:
```bash
twig new git@github.com:user/myproject.git  # Creates project "myproject"
```

Aliases: `ls` for `list`, `s` for `start`, `n` for `new`, `e` for `edit`, `rm` for `delete`, `wt` for `worktree`

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

  # Commands to run after worktree creation
  post_create:
    - bundle install
    - yarn install
    - rails db:migrate
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

### Sessions

When you run `twig start <project>`:

1. Checks if session already exists → attaches if so
2. Creates new tmux session with configured windows/panes
3. Runs commands in each pane
4. Attaches to the session (or switches if already in tmux)

### Worktrees

When you run `twig worktree create <project> <branch>`:

1. Creates git worktree at `{worktree_base}/{project}/{branch}`
2. Creates the branch if it doesn't exist
3. Copies configured files from parent project
4. Runs post-create commands
5. Starts a tmux session named `{project}__{branch}`

Session naming: `myproject__feature-auth` (double underscore separator)

Worktree path: `~/Work/.trees/myproject/feature-auth`

When you run `twig worktree delete <project> <branch>`:

1. Kills the tmux session if running
2. Removes the git worktree

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
