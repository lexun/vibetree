//! Vibetree - A CLI tool for managing isolated development environments using git worktrees
//!
//! This library provides functionality for:
//! - Managing git worktrees with isolated port configurations
//! - Automatic port allocation and conflict resolution
//! - Environment file generation for process orchestration
//! - Configuration management and state reconciliation

pub mod cli;
pub mod config;
pub mod display;
pub mod env;
pub mod git;
pub mod ports;
pub mod sync;
pub mod validation;

/// Current version of vibetree from Cargo.toml
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// Re-export public types for external use
pub use cli::{Cli, Commands, OutputFormat};
pub use config::{VariableConfig, VibeTreeConfig, WorktreeConfig};
pub use display::WorktreeDisplayData;
pub use env::EnvFileGenerator;
pub use git::{DiscoveredWorktree, GitManager, WorktreeValidation};
pub use validation::{ConfigValidator, ValidationResult};

use anyhow::{Context, Result};
use log::{info, warn};
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;

pub use ports::PortManager;

/// Main application context for vibetree operations
pub struct VibeTreeApp {
    config: VibeTreeConfig,
    vibetree_parent: PathBuf,
}

impl VibeTreeApp {
    /// Create a new VibeTreeApp instance
    pub fn new() -> Result<Self> {
        let vibetree_parent = VibeTreeConfig::get_vibetree_parent()
            .context("Failed to determine VIBETREE_PARENT directory")?;

        Self::with_parent(vibetree_parent)
    }

    /// Create a new VibeTreeApp instance with a specific parent directory
    pub fn with_parent(vibetree_parent: PathBuf) -> Result<Self> {
        let config = VibeTreeConfig::load_or_create_with_parent(Some(vibetree_parent.clone()))
            .context("Failed to load or create vibetree configuration")?;

        Ok(Self {
            config,
            vibetree_parent,
        })
    }

    /// Create a VibeTreeApp instance that only loads existing configuration (doesn't create new files)
    pub fn load_existing() -> Result<Self> {
        let vibetree_parent = VibeTreeConfig::get_vibetree_parent()
            .context("Failed to determine VIBETREE_PARENT directory")?;

        Self::load_existing_with_parent(vibetree_parent)
    }

    /// Create a VibeTreeApp instance with specific parent that only loads existing configuration
    pub fn load_existing_with_parent(vibetree_parent: PathBuf) -> Result<Self> {
        let config = VibeTreeConfig::load_existing_with_parent(Some(vibetree_parent.clone()))
            .context("Failed to load existing vibetree configuration")?;

        Ok(Self {
            config,
            vibetree_parent,
        })
    }

    /// Initialize vibetree configuration
    pub fn init(&mut self, variables: Vec<String>, convert_repo: bool) -> Result<()> {
        info!("Initializing vibetree configuration");

        // Handle repository conversion if requested
        if convert_repo {
            self.convert_existing_repo(&variables)?;
            return Ok(());
        }

        // Clear existing configuration to start fresh
        self.config.project_config.variables.clear();

        // Parse and update variables if provided
        if !variables.is_empty() {
            for variable_spec in &variables {
                if let Some((variable, port_str)) = variable_spec.split_once(':') {
                    let port = port_str.parse::<u16>().with_context(|| {
                        format!("Invalid port '{}' for variable '{}'", port_str, variable)
                    })?;

                    // Use variable name as-is (already should be a proper env var name)
                    let env_var_name = variable.to_uppercase();

                    self.config.project_config.variables.push(VariableConfig {
                        name: env_var_name,
                        default_value: port,
                    });
                } else {
                    // Variable without port - use default incremental port
                    let default_port =
                        8000 + (self.config.project_config.variables.len() as u16 * 100);
                    let env_var_name = variable_spec.to_uppercase();

                    self.config.project_config.variables.push(VariableConfig {
                        name: env_var_name,
                        default_value: default_port,
                    });
                }
            }
        }

        // Add or update the main branch to branches configuration if variables are configured
        if !self.config.project_config.variables.is_empty() {
            let main_branch = GitManager::get_current_branch(&self.vibetree_parent)
                .unwrap_or_else(|_| self.config.project_config.main_branch.clone());

            // Create value mapping for main branch using base variable values
            let mut main_branch_values = HashMap::new();
            for variable in &self.config.project_config.variables {
                main_branch_values.insert(variable.name.clone(), variable.default_value);
            }

            // Add or update main branch with the base variable values to branches.toml
            self.config
                .add_or_update_worktree(main_branch.clone(), Some(main_branch_values.clone()))?;

            // Generate env file for the main worktree
            let env_file_path = self.config.get_env_file_path(&self.vibetree_parent);
            EnvFileGenerator::generate_env_file(&env_file_path, &main_branch, &main_branch_values)
                .context("Failed to generate environment file for main worktree")?;
        }

        self.save_config()?;

        // Update .gitignore to include .vibetree directory
        self.update_gitignore(&self.vibetree_parent)?;

        // Automatically repair to update all discovered worktrees with new configuration
        info!("Running repair to update all worktree configurations");
        self.repair(false)?;

        println!(
            "[✓] Initialized vibetree configuration at {}",
            self.vibetree_parent.join("vibetree.toml").display()
        );
        println!(
            "[*] Configured variables: {}",
            self.config
                .project_config
                .variables
                .iter()
                .map(|v| format!("{}:{}", v.name, v.default_value))
                .collect::<Vec<_>>()
                .join(", ")
        );
        if !self.config.project_config.variables.is_empty() {
            println!("[+] Environment file created at .vibetree/env");
            println!(
                "    Use with process orchestrators like: docker compose --env-file .vibetree/env up"
            );
        }

        Ok(())
    }

    /// Convert existing git repository to vibetree-managed structure in-place
    fn convert_existing_repo(&mut self, variables: &[String]) -> Result<()> {
        let current_dir = std::env::current_dir().context("Failed to get current directory")?;

        // Validate conversion is possible and needed
        if !GitManager::is_git_repo_root(&current_dir) {
            anyhow::bail!(
                "Current directory is not a git repository root. Run this command from the root of your git repository."
            );
        }

        if GitManager::is_vibetree_configured(&current_dir) {
            anyhow::bail!("Repository is already managed by vibetree (vibetree.toml exists).");
        }

        info!(
            "Converting repository at {} to vibetree-managed structure",
            current_dir.display()
        );

        // Get current branch name for informational purposes
        let current_branch =
            GitManager::get_current_branch(&current_dir).unwrap_or_else(|_| "main".to_string());

        // Create branches directory
        let branches_dir = current_dir.join(&self.config.project_config.branches_dir);
        if !branches_dir.exists() {
            std::fs::create_dir_all(&branches_dir).with_context(|| {
                format!(
                    "Failed to create branches directory: {}",
                    branches_dir.display()
                )
            })?;
            println!(
                "📁 Created {} directory for worktrees",
                self.config.project_config.branches_dir
            );
        }

        // Update .gitignore to include branches directory
        self.update_gitignore(&current_dir)?;

        // Configure variables
        if !variables.is_empty() {
            for variable_spec in variables {
                if let Some((variable, port_str)) = variable_spec.split_once(':') {
                    let port = port_str.parse::<u16>().with_context(|| {
                        format!("Invalid port '{}' for variable '{}'", port_str, variable)
                    })?;

                    // Use variable name as-is (already should be a proper env var name)
                    let env_var_name = variable.to_uppercase();

                    self.config.project_config.variables.push(VariableConfig {
                        name: env_var_name,
                        default_value: port,
                    });
                } else {
                    // Variable without port - use default incremental port
                    let default_port =
                        8000 + (self.config.project_config.variables.len() as u16 * 100);
                    let env_var_name = variable_spec.to_uppercase();

                    self.config.project_config.variables.push(VariableConfig {
                        name: env_var_name,
                        default_value: default_port,
                    });
                }
            }
        }

        // Add or update the main branch to branches configuration if variables are configured
        if !self.config.project_config.variables.is_empty() {
            // Create value mapping for main branch using base variable values
            let mut main_branch_values = HashMap::new();
            for variable in &self.config.project_config.variables {
                main_branch_values.insert(variable.name.clone(), variable.default_value);
            }

            // Add or update main branch with the base variable values to branches.toml
            self.config
                .add_or_update_worktree(current_branch.clone(), Some(main_branch_values.clone()))?;

            // Generate env file for the main worktree
            let env_file_path = self.config.get_env_file_path(&self.vibetree_parent);
            EnvFileGenerator::generate_env_file(
                &env_file_path,
                &current_branch,
                &main_branch_values,
            )
            .context("Failed to generate environment file for main worktree")?;
        }

        // Save the configuration
        self.save_config()?;

        println!("[✓] Successfully converted repository to vibetree-managed structure");
        println!(
            "[*] Configured variables: {}",
            self.config
                .project_config
                .variables
                .iter()
                .map(|v| format!("{}:{}", v.name, v.default_value))
                .collect::<Vec<_>>()
                .join(", ")
        );
        if !self.config.project_config.variables.is_empty() {
            println!("[+] Environment file created at .vibetree/env");
            println!(
                "    Use with process orchestrators like: docker compose --env-file .vibetree/env up"
            );
        }
        println!(
            "[>] Current branch '{}' remains active in repository root",
            current_branch
        );
        println!(
            "[/] Future worktrees will be created in {}/",
            self.config.project_config.branches_dir
        );

        Ok(())
    }

    /// Update .gitignore to include .vibetree directory
    fn update_gitignore(&self, repo_root: &std::path::Path) -> Result<()> {
        let gitignore_path = repo_root.join(".gitignore");
        let vibetree_rule = ".vibetree/";

        // Read existing .gitignore or create empty content
        let mut content = if gitignore_path.exists() {
            std::fs::read_to_string(&gitignore_path).with_context(|| {
                format!("Failed to read .gitignore: {}", gitignore_path.display())
            })?
        } else {
            String::new()
        };

        // Check if rule already exists
        if content.lines().any(|line| line.trim() == vibetree_rule) {
            println!("[=] .gitignore already contains {} rule", vibetree_rule);
            return Ok(());
        }

        // Add the rule
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&format!("{}\n", vibetree_rule));

        std::fs::write(&gitignore_path, content).with_context(|| {
            format!("Failed to update .gitignore: {}", gitignore_path.display())
        })?;

        println!("[+] Added {} to .gitignore", vibetree_rule);
        Ok(())
    }

    /// Add a new worktree with isolated environment
    pub fn add_worktree(
        &mut self,
        branch_name: String,
        from_branch: Option<String>,
        custom_values: Option<Vec<u16>>,
        dry_run: bool,
        switch: bool,
    ) -> Result<()> {
        info!("Adding worktree: {}", branch_name);

        // Validate input
        if branch_name.is_empty() {
            anyhow::bail!("Branch name cannot be empty");
        }

        if self
            .config
            .branches_config
            .worktrees
            .contains_key(&branch_name)
        {
            anyhow::bail!("Worktree '{}' already exists", branch_name);
        }

        // Find git repository
        let repo_path = GitManager::find_repo_root(&self.vibetree_parent)
            .context("Not inside a git repository")?;

        let branches_dir = self
            .vibetree_parent
            .join(&self.config.project_config.branches_dir);
        let worktree_path = branches_dir.join(&branch_name);

        // Create branches directory if it doesn't exist
        if !branches_dir.exists() {
            std::fs::create_dir_all(&branches_dir).with_context(|| {
                format!(
                    "Failed to create branches directory: {}",
                    branches_dir.display()
                )
            })?;
        }

        if worktree_path.exists() {
            anyhow::bail!("Directory '{}' already exists", worktree_path.display());
        }

        // Convert custom values Vec to HashMap if provided
        let custom_value_map = if let Some(custom) = custom_values {
            // Validate value count matches variable count
            if custom.len() != self.config.project_config.variables.len() {
                anyhow::bail!(
                    "Expected {} values for variables: {}",
                    self.config.project_config.variables.len(),
                    self.config
                        .project_config
                        .variables
                        .iter()
                        .map(|v| v.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }

            let mut value_map = HashMap::new();
            for (variable, value) in self
                .config
                .project_config
                .variables
                .iter()
                .zip(custom.iter())
            {
                value_map.insert(variable.name.clone(), *value);
            }
            Some(value_map)
        } else {
            None
        };

        // First, add the worktree to configuration (this handles port allocation and validation)
        let values = self
            .config
            .add_worktree(branch_name.clone(), custom_value_map)?;

        // Validate that allocated values are actually available on the system (for port variables)
        let value_list: Vec<u16> = values.values().cloned().collect();
        let availability = PortManager::check_ports_availability(&value_list);
        let unavailable: Vec<u16> = availability
            .iter()
            .filter_map(|(&value, &available)| if !available { Some(value) } else { None })
            .collect();

        if !unavailable.is_empty() {
            // Remove the worktree from config since value validation failed
            self.config.remove_worktree(&branch_name)?;
            anyhow::bail!(
                "The following values are not available as ports: {}",
                unavailable
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }

        if dry_run {
            // Remove from configuration since this was just a dry run
            self.config.remove_worktree(&branch_name)?;

            println!("[?] Dry run - would add worktree '{}' with:", branch_name);
            println!("  [/] Path: {}", worktree_path.display());
            println!(
                "  [>] Base branch: {}",
                from_branch.as_deref().unwrap_or("HEAD")
            );
            println!("  [#] Values:");
            for (variable, value) in &values {
                println!("    {} → {}", variable, value);
            }
            return Ok(());
        }

        // Create git worktree
        GitManager::create_worktree(
            &repo_path,
            &worktree_path,
            &branch_name,
            from_branch.as_deref(),
        )
        .context("Failed to create git worktree")?;

        // Configuration was already updated by add_worktree above

        // Generate environment file
        let env_file_path = self.config.get_env_file_path(&worktree_path);
        EnvFileGenerator::generate_env_file(&env_file_path, &branch_name, &values)
            .context("Failed to generate environment file")?;

        // Check and suggest .gitignore update
        if !EnvFileGenerator::suggest_gitignore_update(&worktree_path)? {
            println!(
                "💡 Consider adding '.vibetree/' to {}/.gitignore",
                worktree_path.display()
            );
        }

        // Save configuration
        self.save_config()?;

        println!(
            "[✓] Added worktree '{}' at {}",
            branch_name,
            worktree_path.display()
        );
        println!("[#] Allocated values:");
        for (variable, value) in &values {
            println!("  {} → {}", variable, value);
        }
        println!("[+] Environment file created at .vibetree/env");
        println!(
            "    Use with process orchestrators like: docker compose --env-file .vibetree/env up"
        );

        // Handle switch flag
        if switch {
            self.spawn_shell_in_directory(&worktree_path)?;
        }

        Ok(())
    }

    /// Remove a worktree and clean up resources
    pub fn remove_worktree(
        &mut self,
        branch_name: String,
        force: bool,
        keep_branch: bool,
    ) -> Result<()> {
        self.remove_worktree_with_confirmation(branch_name, force, keep_branch, true)
    }

    /// Remove a worktree and clean up resources with optional confirmation
    fn remove_worktree_with_confirmation(
        &mut self,
        branch_name: String,
        force: bool,
        keep_branch: bool,
        prompt_for_confirmation: bool,
    ) -> Result<()> {
        info!("Removing worktree: {}", branch_name);

        if !self
            .config
            .branches_config
            .worktrees
            .contains_key(&branch_name)
        {
            anyhow::bail!("Worktree '{}' does not exist in configuration", branch_name);
        }

        let worktree_path = self
            .vibetree_parent
            .join(&self.config.project_config.branches_dir)
            .join(&branch_name);

        if !force && prompt_for_confirmation {
            println!(
                "[!] Make sure no important processes are using the allocated ports before removing"
            );
            print!(
                "Are you sure you want to remove worktree '{}'? (y/N): ",
                branch_name
            );
            io::stdout().flush().context("Failed to flush stdout")?;

            let mut input = String::new();
            io::stdin()
                .read_line(&mut input)
                .context("Failed to read confirmation input")?;

            let input = input.trim().to_lowercase();
            if input != "y" && input != "yes" {
                println!("[X] Cancelled removal of worktree '{}'", branch_name);
                return Ok(());
            }
        }

        // Find git repository and remove worktree
        if let Ok(repo_path) = GitManager::find_repo_root(&self.vibetree_parent) {
            if let Err(e) = GitManager::remove_worktree(&repo_path, &branch_name, keep_branch) {
                warn!("Failed to remove git worktree: {}", e);
                // Continue with cleanup even if git removal fails
            }
        }

        // Remove from configuration
        self.config.remove_worktree(&branch_name)?;
        self.save_config()?;

        // Remove directory if it still exists
        if worktree_path.exists() {
            std::fs::remove_dir_all(&worktree_path).with_context(|| {
                format!("Failed to remove directory: {}", worktree_path.display())
            })?;
        }

        println!("[✓] Removed worktree '{}'", branch_name);
        if keep_branch {
            println!("[>] Kept git branch '{}'", branch_name);
        }

        Ok(())
    }

    /// List all worktrees and their configurations
    pub fn list_worktrees(&self, format: Option<OutputFormat>) -> Result<()> {
        let display_manager =
            crate::display::DisplayManager::new(&self.config, &self.vibetree_parent);
        display_manager.list_worktrees(format)
    }

    /// Collect worktree data with validation status for display
    pub fn collect_worktree_data(&self) -> Result<Vec<WorktreeDisplayData>> {
        let display_manager =
            crate::display::DisplayManager::new(&self.config, &self.vibetree_parent);
        display_manager.collect_worktree_data()
    }

    fn save_config(&self) -> Result<()> {
        self.config.save().context("Failed to save configuration")
    }

    // Getter methods to allow tests to access private fields
    pub fn get_variables(&self) -> &Vec<VariableConfig> {
        &self.config.project_config.variables
    }

    pub fn get_worktrees(&self) -> &std::collections::HashMap<String, WorktreeConfig> {
        &self.config.branches_config.worktrees
    }

    /// Get mutable access to config for testing
    #[doc(hidden)]
    pub fn get_config_mut(&mut self) -> &mut VibeTreeConfig {
        &mut self.config
    }

    /// Repair configuration and discover orphaned worktrees
    pub fn repair(&mut self, dry_run: bool) -> Result<()> {
        let mut sync_manager =
            crate::sync::SyncManager::new(&mut self.config, &self.vibetree_parent);
        sync_manager.sync(dry_run)
    }

    /// Switch to an existing worktree directory
    pub fn switch_to_worktree(&self, branch_name: String) -> Result<()> {
        info!("Switching to worktree: {}", branch_name);

        // Determine target directory
        let target_path = if branch_name == self.config.project_config.main_branch {
            // Switching to main branch - use root directory
            self.vibetree_parent.clone()
        } else {
            // Switching to a worktree branch
            let worktree_path = self
                .vibetree_parent
                .join(&self.config.project_config.branches_dir)
                .join(&branch_name);

            if !worktree_path.exists() {
                return Err(anyhow::anyhow!(
                    "Worktree '{}' does not exist at {}",
                    branch_name,
                    worktree_path.display()
                ));
            }

            // Check if it's actually a git worktree
            let worktree_list = std::process::Command::new("git")
                .args(["worktree", "list", "--porcelain"])
                .current_dir(&self.vibetree_parent)
                .output()
                .context("Failed to list worktrees")?;

            let worktree_list_str = String::from_utf8_lossy(&worktree_list.stdout);
            let worktree_path_str = worktree_path.to_string_lossy();

            if !worktree_list_str
                .lines()
                .any(|line| line.starts_with("worktree ") && line.contains(&*worktree_path_str))
            {
                return Err(anyhow::anyhow!(
                    "Directory '{}' exists but is not a git worktree",
                    worktree_path.display()
                ));
            }

            worktree_path
        };

        // Spawn a shell in the target directory
        self.spawn_shell_in_directory(&target_path)
    }

    /// Spawn a new shell in the specified directory
    fn spawn_shell_in_directory(&self, path: &std::path::Path) -> Result<()> {
        use std::process::Command;
        
        if !path.exists() {
            return Err(anyhow::anyhow!("Directory does not exist: {}", path.display()));
        }
        
        // Check if we're already in a vibetree subshell and switching to main
        let current_depth = std::env::var("VIBETREE_DEPTH")
            .unwrap_or_else(|_| "0".to_string())
            .parse::<u32>()
            .unwrap_or(0);
        
        let is_switching_to_main = path == self.vibetree_parent;
        
        // If we're in a subshell and switching back to main, use exec to replace the shell
        if current_depth > 0 && is_switching_to_main {
            println!("🔙 Returning to main directory");
            
            // Use exec to replace the current shell process with a new one in main directory
            let shell_env = std::env::var("SHELL").unwrap_or_default();
            let shell_name = shell_env.split('/').last().unwrap_or("bash");
            
            // Get the actual main directory path from stored environment or config
            let main_path = std::env::var("VIBETREE_PREV_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| self.vibetree_parent.clone());
            
            // Try to find and terminate the current subshell to return to main
            let shells = self.find_all_shell_processes();
            if let Some((shell_pid, _)) = shells.first() {
                // Terminate the current subshell with SIGTERM
                let result = unsafe { libc::kill(*shell_pid, libc::SIGTERM) };
                if result == 0 {
                    std::process::exit(0);
                } else {
                    // Fallback to SIGKILL if SIGTERM fails
                    unsafe { libc::kill(*shell_pid, libc::SIGKILL); }
                    std::process::exit(0);
                }
            } else {
                println!("❌ Could not find shell process to terminate");
            }
            
            if shell_name.contains("nu") {
                println!("💡 Manual fallback: cd {}; exit", main_path.display());
            } else {
                println!("💡 Manual fallback: exec bash -c 'cd {}; exec $SHELL'", main_path.display());
            }
            
            return Ok(());
        }
        
        // Detect the user's shell
        let shell = std::env::var("SHELL").unwrap_or_else(|_| {
            // Default shells by OS
            if cfg!(windows) {
                "cmd".to_string()
            } else {
                "/bin/bash".to_string()
            }
        });
        
        println!("🚀 Starting new shell in {}", path.display());
        
        // Set up direnv integration if project uses direnv and root is allowed
        if self.project_uses_direnv() && self.is_direnv_available() {
            if !self.is_root_direnv_allowed() {
                println!("⚠️  Direnv detected but not allowed in root directory");
                println!("   Run 'direnv allow' in {} first", self.vibetree_parent.display());
            } else if let Err(e) = self.setup_direnv_integration(path) {
                println!("⚠️  Warning: Failed to set up direnv: {}", e);
            } else {
                println!("✅ Set up direnv for automatic environment loading");
            }
        }
        
        println!("💡 Type 'exit' to return to your previous directory");
        
        // Get current directory to set as OLDPWD for cd - functionality
        let current_dir = std::env::current_dir()
            .unwrap_or_else(|_| self.vibetree_parent.clone());
        
        // For nushell, we need to handle it differently since it doesn't support all features
        let shell_env = std::env::var("SHELL").unwrap_or_default();
        let shell_name = shell_env.split('/').last().unwrap_or("bash");
        
        // Spawn the shell in the target directory with environment variables
        let mut cmd = Command::new(&shell);
        cmd.current_dir(path)
            .env("VIBETREE_DEPTH", (current_depth + 1).to_string())
            .env("VIBETREE_PREV_DIR", &current_dir)
            .env("OLDPWD", &current_dir);
            
        // For nushell, add initialization script
        if shell_name.contains("nu") {
            let init_script = format!(
                "$env.VIBETREE_DEPTH = {}; $env.VIBETREE_PREV_DIR = '{}'; $env.OLDPWD = '{}'", 
                current_depth + 1, 
                current_dir.display(), 
                current_dir.display()
            );
            cmd.arg("-e").arg(&init_script);
        }
        
        // Get current process PID to pass to the child shell
        let parent_pid = std::process::id();
        cmd.env("VIBETREE_SHELL_PID", parent_pid.to_string());
        
        // Spawn the interactive shell
        let status = cmd.status()
            .with_context(|| format!("Failed to start shell: {}", shell))?;
        
        if !status.success() {
            // Check if this was a shell terminated by signal (normal for vibetree switching)
            if status.code().is_none() {
                return Ok(());
            }
            
            return Err(anyhow::anyhow!("Shell exited with error code: {:?}", status.code()));
        }
        
        Ok(())
    }

    /// Find all shell processes in the process tree to understand the hierarchy
    fn find_all_shell_processes(&self) -> Vec<(i32, String)> {
        let mut shells = Vec::new();
        let mut pid = unsafe { libc::getppid() };
        let mut depth = 0;
        const MAX_DEPTH: u8 = 15; // Prevent infinite loops
        
        while depth < MAX_DEPTH {
            // Get process name using ps command
            if let Ok(output) = std::process::Command::new("ps")
                .args(["-p", &pid.to_string(), "-o", "comm="])
                .output()
            {
                if output.status.success() {
                    let process_name = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    
                    // Check if this is a shell process (common shell names)
                    if process_name.ends_with("sh") || process_name.ends_with("zsh") || 
                       process_name.ends_with("bash") || process_name.ends_with("fish") ||
                       process_name.contains("nu") {
                        shells.push((pid, process_name.clone()));
                    }
                    
                    // For any process, try to get its parent and continue walking up
                    if let Ok(parent_output) = std::process::Command::new("ps")
                        .args(["-p", &pid.to_string(), "-o", "ppid="])
                        .output()
                    {
                        if parent_output.status.success() {
                            if let Ok(parent_pid) = String::from_utf8_lossy(&parent_output.stdout)
                                .trim()
                                .parse::<i32>()
                            {
                                pid = parent_pid;
                                depth += 1;
                                continue;
                            }
                        }
                    }
                }
            }
            
            break;
        }
        
        shells
    }

    /// Check if direnv is available in the system
    fn is_direnv_available(&self) -> bool {
        std::process::Command::new("direnv")
            .arg("version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    /// Check if the project uses direnv by looking for .envrc in root
    fn project_uses_direnv(&self) -> bool {
        self.vibetree_parent.join(".envrc").exists()
    }

    /// Check if direnv is allowed in the root directory
    fn is_root_direnv_allowed(&self) -> bool {
        // Run direnv status in the root to check if it's allowed
        std::process::Command::new("direnv")
            .arg("status")
            .current_dir(&self.vibetree_parent)
            .output()
            .map(|output| {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    // Check if .envrc is allowed (allowed level 0 or 1, denied is 2+)
                    if let Some(line) = stdout.lines().find(|line| line.contains("Found RC allowed")) {
                        // Extract the number after "Found RC allowed"
                        if let Some(allowed_str) = line.split("Found RC allowed").nth(1) {
                            if let Ok(level) = allowed_str.trim().parse::<u32>() {
                                return level <= 1; // 0 = allowed, 1 = allowed, 2+ = denied
                            }
                        }
                    }
                    false
                } else {
                    false
                }
            })
            .unwrap_or(false)
    }

    /// Set up direnv integration for the worktree
    fn setup_direnv_integration(&self, path: &std::path::Path) -> Result<()> {
        let envrc_path = path.join(".envrc");

        // Copy the root .envrc to the worktree if it doesn't exist
        if !envrc_path.exists() {
            let root_envrc = self.vibetree_parent.join(".envrc");
            if root_envrc.exists() {
                std::fs::copy(&root_envrc, &envrc_path)
                    .with_context(|| format!("Failed to copy .envrc to worktree: {}", envrc_path.display()))?;
            }
        }

        // Run direnv allow
        let output = std::process::Command::new("direnv")
            .arg("allow")
            .arg(path)
            .output()
            .context("Failed to execute direnv allow")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("direnv allow failed: {}", stderr));
        }

        Ok(())
    }

    /// Internal method for testing - bypasses confirmation prompts
    /// DO NOT USE in production code - use remove_worktree instead
    #[doc(hidden)]
    pub fn remove_worktree_for_test(
        &mut self,
        branch_name: String,
        force: bool,
        keep_branch: bool,
    ) -> Result<()> {
        self.remove_worktree_with_confirmation(branch_name, force, keep_branch, false)
    }
}

impl Default for VibeTreeApp {
    fn default() -> Self {
        Self::new().expect("Failed to create VibeTreeApp")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_app() -> Result<(TempDir, VibeTreeApp)> {
        let temp_dir = TempDir::new()?;

        // Initialize a git repository in the temp directory for testing
        let repo = git2::Repository::init(temp_dir.path())?;

        // Create initial commit to have a valid HEAD
        let sig = git2::Signature::now("Test", "test@example.com")?;
        let tree_id = {
            let mut index = repo.index()?;
            index.write_tree()?
        };
        let tree = repo.find_tree(tree_id)?;
        repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])?;

        let app = VibeTreeApp::with_parent(temp_dir.path().to_path_buf())?;
        Ok((temp_dir, app))
    }

    #[test]
    fn test_new_app() -> Result<()> {
        let (_temp_dir, app) = setup_test_app()?;
        assert_eq!(app.config.project_config.version, "1");
        assert_eq!(app.config.project_config.main_branch, "main");
        // Variables should be empty by default - user specifies them during init
        assert!(app.config.project_config.variables.is_empty());
        Ok(())
    }

    #[test]
    fn test_init() -> Result<()> {
        let (_temp_dir, mut app) = setup_test_app()?;

        let variables = vec!["postgres".to_string(), "redis".to_string()];
        app.init(variables.clone(), false)?;

        // Variables should be updated after init
        // Verify variables were configured
        let config_path = app.vibetree_parent.join("vibetree.toml");
        assert!(config_path.exists());
        assert!(
            app.config
                .project_config
                .variables
                .iter()
                .any(|v| v.name == "POSTGRES")
        );
        assert!(
            app.config
                .project_config
                .variables
                .iter()
                .any(|v| v.name == "REDIS")
        );

        Ok(())
    }

    #[test]
    fn test_list_empty_worktrees() -> Result<()> {
        let (_temp_dir, app) = setup_test_app()?;

        // Should not panic with empty worktrees
        app.list_worktrees(Some(OutputFormat::Table))?;
        app.list_worktrees(Some(OutputFormat::Json))?;

        Ok(())
    }
}
