use anyhow::Result;
use clap::{Parser, Subcommand};

mod cli;
mod config;
mod git;
mod tmux;
mod ui;

#[derive(Parser)]
#[command(name = "twig")]
#[command(about = "Tmux session manager with git worktree support")]
#[command(version)]
struct Cli {
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
    List,

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

    /// Git worktree operations
    #[command(alias = "t")]
    Tree {
        #[command(subcommand)]
        action: TreeCommands,
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

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Start { project } => cli::start::run(project),
        Commands::List => cli::list::run(),
        Commands::New { name } => cli::new::run(name),
        Commands::Edit { project } => cli::edit::run(project),
        Commands::Delete { project } => cli::delete::run(project),
        Commands::Stop { session } => cli::kill::run(session),
        Commands::Tree { action } => match action {
            TreeCommands::Create { project, branch } => cli::worktree::create(project, branch),
            TreeCommands::List { project } => cli::worktree::list(project),
            TreeCommands::Delete { project, branch } => cli::worktree::delete(project, branch),
            TreeCommands::Merge { project, branch } => cli::worktree::merge(project, branch),
        },
    }
}
