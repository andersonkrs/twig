use anyhow::Result;
use clap::{Parser, Subcommand};

mod cli;
mod config;
mod git;
mod tmux;
mod tmux_control;
mod ui;

#[derive(Parser)]
#[command(name = "twig")]
#[command(about = "Tmux session manager with git worktree support")]
#[command(
    after_long_help = "Debug: use --verbose or set TWIG_DEBUG=1 to enable verbose tmux control output on stderr."
)]
#[command(version)]
struct Cli {
    /// Enable verbose tmux control output (sets TWIG_DEBUG=1)
    #[arg(long, short, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start or attach to a session
    #[command(alias = "s")]
    Start {
        /// Project name (interactive selection if not provided)
        project: Option<String>,
    },

    /// List all projects
    #[command(alias = "ls")]
    List {
        /// Focus on current TWIG_PROJECT/TWIG_WORKTREE
        #[arg(long)]
        focus_current: bool,
    },

    /// Create a new project
    #[command(alias = "n")]
    New {
        /// Project name
        name: Option<String>,
    },

    /// Edit project config in $EDITOR
    #[command(alias = "e")]
    Edit {
        /// Project name
        project: Option<String>,
    },

    /// Delete a project config
    #[command(alias = "rm")]
    Delete {
        /// Project name
        project: Option<String>,
    },

    /// Stop (kill) a tmux session
    #[command(alias = "kill")]
    Stop {
        /// Session name
        session: Option<String>,
    },

    /// Run a command in a tmux session
    #[command(alias = "r")]
    Run {
        /// Command to run
        #[arg(trailing_var_arg = true)]
        command: Vec<String>,
        /// Project/session name (defaults to TWIG_PROJECT when set)
        #[arg(long)]
        project: Option<String>,
        /// Worktree branch name (defaults to TWIG_WORKTREE when set)
        #[arg(long)]
        tree: Option<String>,
        /// Window index or name (defaults to current window if available)
        #[arg(long)]
        window: Option<String>,
        /// Target pane index or id
        #[arg(long)]
        pane: Option<String>,
        /// Tmux socket path to target
        #[arg(long)]
        socket: Option<String>,
    },

    /// Git worktree operations
    #[command(alias = "t")]
    Tree {
        #[command(subcommand)]
        action: TreeCommands,
    },

    /// Window operations
    #[command(alias = "w")]
    Window {
        #[command(subcommand)]
        action: WindowCommands,
    },
}

#[derive(Subcommand)]
enum TreeCommands {
    /// Create a new worktree and start a session
    #[command(alias = "c")]
    Create {
        /// Project name
        project: Option<String>,
        /// Branch name
        branch: Option<String>,
    },

    /// List worktrees for a project
    #[command(alias = "ls")]
    List {
        /// Project name
        project: Option<String>,
    },

    /// Delete a worktree and its session
    #[command(alias = "rm")]
    Delete {
        /// Project name
        project: Option<String>,
        /// Branch name
        branch: Option<String>,
    },

    /// Merge a worktree branch into main/master
    #[command(alias = "m")]
    Merge {
        /// Project name
        project: Option<String>,
        /// Branch name
        branch: Option<String>,
    },
}

#[derive(Subcommand)]
enum WindowCommands {
    /// Create a new window in an existing session
    #[command(alias = "n")]
    New {
        /// Project/session name (interactive selection if not provided)
        project: Option<String>,
        /// Window name
        name: Option<String>,
        /// Tmux socket path to target
        #[arg(long)]
        socket: Option<String>,
    },

    /// List panes for a window
    #[command(alias = "lp")]
    ListPanes {
        /// Window index or name
        window: String,
        /// Project/session name (defaults to current tmux session if available)
        #[arg(long)]
        project: Option<String>,
        /// Tmux socket path to target
        #[arg(long)]
        socket: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.verbose {
        std::env::set_var("TWIG_DEBUG", "1");
    }

    match cli.command {
        Commands::Start { project } => cli::start::run(project),
        Commands::List { focus_current } => cli::list::run(focus_current),
        Commands::New { name } => cli::new::run(name),
        Commands::Edit { project } => cli::edit::run(project),
        Commands::Delete { project } => cli::delete::run(project),
        Commands::Stop { session } => cli::kill::run(session),
        Commands::Run {
            command,
            project,
            tree,
            window,
            pane,
            socket,
        } => cli::window::run(project, tree, window, command, pane, socket),
        Commands::Tree { action } => match action {
            TreeCommands::Create { project, branch } => cli::worktree::create(project, branch),
            TreeCommands::List { project } => cli::worktree::list(project),
            TreeCommands::Delete { project, branch } => cli::worktree::delete(project, branch),
            TreeCommands::Merge { project, branch } => cli::worktree::merge(project, branch),
        },
        Commands::Window { action } => match action {
            WindowCommands::New {
                project,
                name,
                socket,
            } => cli::window::new(project, name, socket),
            WindowCommands::ListPanes {
                window,
                project,
                socket,
                json,
            } => cli::window::list_panes(project, window, socket, json),
        },
    }
}
