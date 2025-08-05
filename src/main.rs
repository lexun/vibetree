use clap::Parser;
use log::error;
use vibetree::{Cli, Commands, VibeTreeApp};

fn main() {
    env_logger::init();

    let cli = Cli::parse();

    if let Err(e) = run(cli) {
        error!("Error: {}", e);

        // Print error chain
        let mut current = e.source();
        while let Some(err) = current {
            error!("  Caused by: {}", err);
            current = err.source();
        }

        std::process::exit(1);
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    if cli.verbose {
        log::set_max_level(log::LevelFilter::Debug);
    }

    let mut app = VibeTreeApp::new()?;

    match cli.command {
        Commands::Init {
            variables,
            convert_repo,
        } => {
            app.init(variables, convert_repo)?;
        }

        Commands::Create {
            branch_name,
            from,
            ports,
            dry_run,
        } => {
            app.create_worktree(branch_name, from, ports, dry_run)?;
        }

        Commands::Remove {
            branch_name,
            force,
            keep_branch,
        } => {
            app.remove_worktree(branch_name, force, keep_branch)?;
        }

        Commands::List { format } => {
            app.list_worktrees(format)?;
        }
    }

    Ok(())
}
