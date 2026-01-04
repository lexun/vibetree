use clap::{Parser, Subcommand, ValueEnum};
use clap_complete::engine::{ArgValueCompleter, CompletionCandidate};
use std::ffi::OsStr;

/// Supported shells for completion generation
#[derive(Clone, Debug, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Elvish,
    Fish,
    Powershell,
    Zsh,
    /// Generate carapace spec (YAML)
    Carapace,
    /// Auto-detect and install completions
    Install,
}

/// Custom completer that returns existing worktree names
fn complete_worktree_names(current: &OsStr) -> Vec<CompletionCandidate> {
    let current_str = current.to_str().unwrap_or("");

    // Use the current binary to ensure we're calling the same version
    let exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(_) => return vec![],
    };

    let output = std::process::Command::new(exe)
        .args(["list", "--format", "names"])
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout
                .lines()
                .filter(|name| name.starts_with(current_str))
                .map(|name| CompletionCandidate::new(name))
                .collect()
        }
        _ => vec![],
    }
}

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
    },

    #[command(about = "Add new git worktree with isolated port configuration")]
    Add {
        #[arg(help = "Name of the branch/worktree to add")]
        branch_name: String,

        #[arg(long, help = "Add worktree from specific branch")]
        from: Option<String>,

        #[arg(long, help = "Specify custom value assignments", value_delimiter = ',')]
        ports: Option<Vec<String>>,

        #[arg(long, help = "Show what would be added without making changes")]
        dry_run: bool,

        #[arg(long, help = "Switch to the newly created worktree directory")]
        switch: bool,
    },

    #[command(about = "Remove git worktree and clean up port allocations")]
    Remove {
        #[arg(help = "Name of the branch/worktree to remove", add = ArgValueCompleter::new(complete_worktree_names))]
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

    #[command(about = "Repair configuration and discover orphaned worktrees")]
    Repair {
        #[arg(long, help = "Show what would be repaired without making changes")]
        dry_run: bool,
    },

    #[command(about = "Switch to an existing worktree directory")]
    Switch {
        #[arg(help = "Name of the branch/worktree to switch to", add = ArgValueCompleter::new(complete_worktree_names))]
        branch_name: String,
    },

    #[command(about = "Generate shell completions", hide = true)]
    Completions {
        #[arg(help = "Shell to generate completions for")]
        shell: CompletionShell,
    },
}

#[derive(Clone, Debug, ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
    Yaml,
    /// Output just branch names, one per line (useful for shell completions)
    Names,
}
