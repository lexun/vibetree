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

        Commands::Add {
            branch_name,
            from,
            ports,
            dry_run,
        } => {
            // Try to load existing config first, fall back to empty config for worktrees without variables
            match VibeTreeApp::load_existing() {
                Ok(mut app) => {
                    app.add_worktree(branch_name, from, ports, dry_run)?;
                }
                Err(_) => {
                    // No main config exists - only allow creation if no variables are needed (no ports specified)
                    if ports.is_some() {
                        anyhow::bail!(
                            "Cannot add worktree with custom values when no configuration exists. Run 'vibetree init' first to configure variables."
                        );
                    }
                    let mut app = VibeTreeApp::new()?;
                    app.add_worktree(branch_name, from, None, dry_run)?;
                    // Remove the config file created by VibeTreeApp::new() since we're in discovery mode
                    let config_path = std::env::current_dir()?.join("vibetree.toml");
                    if config_path.exists() {
                        std::fs::remove_file(&config_path)
                            .context("Failed to remove created config file")?;
                    }
                }
            }
        }

        Commands::Remove {
            branch_name,
            force,
            keep_branch,
        } => {
            // Try to load existing config first, fall back to discovery mode
            match VibeTreeApp::load_existing() {
                Ok(mut app) => {
                    app.remove_worktree(branch_name, force, keep_branch)?;
                }
                Err(_) => {
                    // No main config exists - try to load branches config directly for removal
                    let mut app = VibeTreeApp::new()?;
                    app.remove_worktree(branch_name, force, keep_branch)?;
                    // Remove the config file created by VibeTreeApp::new() since we're in discovery mode
                    let config_path = std::env::current_dir()?.join("vibetree.toml");
                    if config_path.exists() {
                        std::fs::remove_file(&config_path)
                            .context("Failed to remove created config file")?;
                    }
                }
            }
        }

        Commands::List { format } => {
            // Try to load existing configuration first, fall back to empty config
            match VibeTreeApp::load_existing() {
                Ok(app) => {
                    app.list_worktrees(format)?;
                }
                Err(_) => {
                    // No config exists - create temporary app to show empty list
                    let app = VibeTreeApp::new()?;
                    app.list_worktrees(format)?;
                    // Remove any config file that might have been created
                    let config_path = std::env::current_dir()?.join("vibetree.toml");
                    if config_path.exists() {
                        std::fs::remove_file(&config_path)
                            .context("Failed to remove created config file")?;
                    }
                }
            }
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
                        std::fs::remove_file(&config_path)
                            .context("Failed to remove created config file")?;
                    }
                }
            }
        }
    }

    Ok(())
}
