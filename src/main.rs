use anyhow::Context;
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

    match cli.command {
        Commands::Init {
            variables,
            convert_repo,
        } => {
            // Init command can create configuration if it doesn't exist
            let mut app = VibeTreeApp::new()?;
            app.init(variables, convert_repo)?;
        }

        Commands::Create {
            branch_name,
            from,
            ports,
            dry_run,
        } => {
            // Other commands require existing configuration
            let mut app = VibeTreeApp::load_existing()?;
            app.create_worktree(branch_name, from, ports, dry_run)?;
        }

        Commands::Remove {
            branch_name,
            force,
            keep_branch,
        } => {
            let mut app = VibeTreeApp::load_existing()?;
            app.remove_worktree(branch_name, force, keep_branch)?;
        }

        Commands::List { format } => {
            let app = VibeTreeApp::load_existing()?;
            app.list_worktrees(format)?;
        }

        Commands::Sync { dry_run } => {
            // Try to load existing configuration first
            match VibeTreeApp::load_existing() {
                Ok(mut app) => {
                    app.sync(dry_run)?;
                }
Err(_) => {
                    // No config exists - run sync in discovery mode
                    let mut app = VibeTreeApp::new()?;
                    app.sync(dry_run)?;
                    // Remove the created config file since sync shouldn't create it
                    let config_path = std::env::current_dir()?.join("vibetree.toml");
                    if config_path.exists() {
                        std::fs::remove_file(&config_path).context("Failed to remove created config file")?;
                        println!("[!] Discovered worktrees but no main configuration exists. Run 'vibetree init' to create one.");
                    }
                }
            }
        }
    }

    Ok(())
}
