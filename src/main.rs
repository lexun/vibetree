use anyhow::Context;
use clap::{CommandFactory, Parser};
use clap_complete::env::CompleteEnv;
use log::error;
use vibetree::{generate_completions, Cli, Commands, VibeTreeApp};

fn main() {
    // Handle dynamic shell completions (if triggered by shell completion request)
    CompleteEnv::with_factory(Cli::command).complete();

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
        Commands::Init { variables } => {
            // Init command can create configuration if it doesn't exist
            let mut app = VibeTreeApp::new()?;
            app.init(variables)?;
        }

        Commands::Add {
            branch_name,
            from,
            ports,
            dry_run,
            switch,
        } => {
            // Try to load existing config first, fall back to empty config for worktrees without variables
            match VibeTreeApp::load_existing() {
                Ok(mut app) => {
                    app.add_worktree(branch_name, from, ports, dry_run, switch)?;
                }
                Err(_) => {
                    // No main config exists - only allow creation if no variables are needed (no ports specified)
                    if ports.is_some() {
                        anyhow::bail!(
                            "Cannot add worktree with custom values when no configuration exists. Run 'vibetree init' first to configure variables."
                        );
                    }
                    let mut app = VibeTreeApp::new()?;
                    app.add_worktree(branch_name, from, None, dry_run, switch)?;
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

        Commands::Repair { dry_run } => {
            // Try to load existing configuration first
            match VibeTreeApp::load_existing() {
                Ok(mut app) => {
                    app.repair(dry_run)?;
                }
                Err(_) => {
                    // No config exists - run repair in discovery mode
                    let mut app = VibeTreeApp::new()?;
                    app.repair(dry_run)?;
                    // Remove the created config file since repair shouldn't create it
                    let config_path = std::env::current_dir()?.join("vibetree.toml");
                    if config_path.exists() {
                        std::fs::remove_file(&config_path)
                            .context("Failed to remove created config file")?;
                    }
                }
            }
        }

        Commands::Switch { branch_name } => {
            // Try to load existing config, but fall back to simple directory navigation if none exists
            match VibeTreeApp::load_existing() {
                Ok(app) => {
                    app.switch_to_worktree(branch_name)?;
                }
                Err(_) => {
                    // No config exists - try simple directory navigation
                    let app = VibeTreeApp::new()?;
                    app.switch_to_worktree(branch_name)?;
                    // Remove the config file created by VibeTreeApp::new() since we're in discovery mode
                    let config_path = std::env::current_dir()?.join("vibetree.toml");
                    if config_path.exists() {
                        std::fs::remove_file(&config_path)
                            .context("Failed to remove created config file")?;
                    }
                }
            }
        }

        Commands::Completions { shell } => {
            generate_completions(shell);
        }

        Commands::Merge {
            branch_name,
            into,
            squash,
            rebase,
            remove,
        } => {
            // Try to load existing config
            match VibeTreeApp::load_existing() {
                Ok(mut app) => {
                    app.merge_worktree(branch_name, into, squash, rebase, remove)?;
                }
                Err(_) => {
                    // No config exists - try to create temporary app for merge
                    let mut app = VibeTreeApp::new()?;
                    app.merge_worktree(branch_name, into, squash, rebase, remove)?;
                    // Remove the config file created by VibeTreeApp::new() since we're in discovery mode
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
