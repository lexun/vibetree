use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "vibetree")]
#[command(about = "A CLI tool for managing isolated development environments using git worktrees")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    #[arg(short, long, global = true)]
    pub verbose: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    #[command(about = "Initialize vibetree configuration")]
    Init {
        #[arg(
            long,
            help = "Specify which variables need port isolation",
            value_delimiter = ','
        )]
        variables: Vec<String>,

        #[arg(
            long,
            help = "Convert current git repo into vibetree-managed structure in-place"
        )]
        convert_repo: bool,
    },

    #[command(about = "Add new git worktree with isolated port configuration")]
    Add {
        #[arg(help = "Name of the branch/worktree to add")]
        branch_name: String,

        #[arg(long, help = "Add worktree from specific branch")]
        from: Option<String>,

        #[arg(long, help = "Specify custom port assignments", value_delimiter = ',')]
        ports: Option<Vec<u16>>,

        #[arg(long, help = "Show what would be added without making changes")]
        dry_run: bool,
    },

    #[command(about = "Remove git worktree and clean up port allocations")]
    Remove {
        #[arg(help = "Name of the branch/worktree to remove")]
        branch_name: String,

        #[arg(
            short,
            long,
            help = "Remove even if processes are running on allocated ports"
        )]
        force: bool,

        #[arg(long, help = "Remove worktree but keep git branch")]
        keep_branch: bool,
    },

    #[command(about = "List all worktrees with their port allocations")]
    List {
        #[arg(short, long, help = "Output format")]
        format: Option<OutputFormat>,
    },

    #[command(about = "Synchronize configuration and discover orphaned worktrees")]
    Sync {
        #[arg(long, help = "Show what would be synchronized without making changes")]
        dry_run: bool,
    },
}

#[derive(Clone, Debug, ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
    Yaml,
}
