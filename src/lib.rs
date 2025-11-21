//! Vibetree - A CLI tool for managing isolated development environments using git worktrees
//!
//! This library provides functionality for:
//! - Managing git worktrees with isolated port configurations
//! - Automatic port allocation and conflict resolution
//! - Environment file generation for process orchestration
//! - Configuration management and state reconciliation

pub mod allocator;
pub mod cli;
pub mod config;
pub mod display;
pub mod env;
pub mod git;
pub mod ports;
pub mod sync;
pub mod template;
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
use log::{debug, info, warn};
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
    pub fn init(&mut self, variables: Vec<String>) -> Result<()> {
        info!("Initializing vibetree configuration");

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
                        value: Some(toml::Value::Integer(port as i64)),
                        r#type: Some(crate::config::VariableType::Port),
                        branch: None,
                    });
                } else {
                    // Variable without port - use default incremental port
                    let default_port =
                        8000 + (self.config.project_config.variables.len() as u16 * 100);
                    let env_var_name = variable_spec.to_uppercase();

                    self.config.project_config.variables.push(VariableConfig {
                        name: env_var_name,
                        value: Some(toml::Value::Integer(default_port as i64)),
                        r#type: Some(crate::config::VariableType::Port),
                        branch: None,
                    });
                }
            }
        }

        // Add or update the main branch to branches configuration if variables are configured
        if !self.config.project_config.variables.is_empty() {
            // Always use the configured main_branch, regardless of current git branch
            let main_branch = self.config.project_config.main_branch.clone();

            // Allocate values for the main branch using the new allocator
            let existing_worktrees = HashMap::new(); // Empty since this is init
            let main_branch_values = self.config.project_config
                .allocate_values(&main_branch, &existing_worktrees)?;

            // Add or update main branch with the allocated values to branches.toml
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

        info!(
            "Initialized vibetree configuration at {}",
            self.vibetree_parent.join("vibetree.toml").display()
        );
        info!(
            "Configured variables: {}",
            self.config
                .project_config
                .variables
                .iter()
                .map(|v| {
                    if let Some(value) = &v.value {
                        match value {
                            toml::Value::Integer(num) => format!("{}:{}", v.name, num),
                            toml::Value::String(s) => format!("{}={}", v.name, s),
                            _ => v.name.clone(),
                        }
                    } else {
                        v.name.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        );
        if !self.config.project_config.variables.is_empty() {
            info!("Environment file created at .vibetree/env");
            info!(
                "Use with process orchestrators like: docker compose --env-file .vibetree/env up"
            );
        }

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
            debug!(".gitignore already contains {} rule", vibetree_rule);
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

        info!("Added {} to .gitignore", vibetree_rule);
        Ok(())
    }

    /// Add a new worktree with isolated environment
    pub fn add_worktree(
        &mut self,
        branch_name: String,
        from_branch: Option<String>,
        custom_values: Option<Vec<String>>,
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
                value_map.insert(variable.name.clone(), value.clone());
            }
            Some(value_map)
        } else {
            None
        };

        // First, add the worktree to configuration (this handles port allocation and validation)
        let values = self
            .config
            .add_worktree(branch_name.clone(), custom_value_map)?;

        // Validate that allocated values that are ports are actually available on the system
        // Only validate values that look like user ports (>= 1024), to avoid false positives
        // from integer values like INSTANCE_ID that happen to be < 1024
        let port_values: Vec<u16> = values
            .values()
            .filter_map(|v| v.parse::<u16>().ok())
            .filter(|&port| port >= 1024)
            .collect();

        if !port_values.is_empty() {
            let availability = PortManager::check_ports_availability(&port_values);
            let unavailable: Vec<u16> = availability
                .iter()
                .filter_map(|(&value, &available)| if !available { Some(value) } else { None })
                .collect();

            if !unavailable.is_empty() {
                // Remove the worktree from config since value validation failed
                self.config.remove_worktree(&branch_name)?;
                anyhow::bail!(
                    "The following ports are not available: {}",
                    unavailable
                        .iter()
                        .map(|p| p.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }

        if dry_run {
            // Remove from configuration since this was just a dry run
            self.config.remove_worktree(&branch_name)?;

            info!("Dry run - would add worktree '{}' with:", branch_name);
            info!("  Path: {}", worktree_path.display());
            info!(
                "  Base branch: {}",
                from_branch.as_deref().unwrap_or("HEAD")
            );
            info!("  Values:");
            for (variable, value) in &values {
                info!("    {} → {}", variable, value);
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
            info!(
                "Consider adding '.vibetree/' to {}/.gitignore",
                worktree_path.display()
            );
        }

        // Save configuration
        self.save_config()?;

        info!(
            "Added worktree '{}' at {}",
            branch_name,
            worktree_path.display()
        );
        info!("Allocated values:");
        for (variable, value) in &values {
            info!("  {} → {}", variable, value);
        }
        info!("Environment file created at .vibetree/env");
        info!(
            "Use with process orchestrators like: docker compose --env-file .vibetree/env up"
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
            warn!(
                "Make sure no important processes are using the allocated ports before removing"
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
                info!("Cancelled removal of worktree '{}'", branch_name);
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

        info!("Removed worktree '{}'", branch_name);
        if keep_branch {
            info!("Kept git branch '{}'", branch_name);
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
            info!("Returning to main directory");
            
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
                warn!("Could not find shell process to terminate");
            }
            
            if shell_name.contains("nu") {
                info!("Manual fallback: cd {}; exit", main_path.display());
            } else {
                info!("Manual fallback: exec bash -c 'cd {}; exec $SHELL'", main_path.display());
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
        
        info!("Starting new shell in {}", path.display());
        
        // Set up direnv integration if project uses direnv and root is allowed
        if self.project_uses_direnv() && self.is_direnv_available() {
            if !self.is_root_direnv_allowed() {
                warn!("Direnv detected but not allowed in root directory");
                info!("Run 'direnv allow' in {} first", self.vibetree_parent.display());
            } else if let Err(e) = self.setup_direnv_integration(path) {
                warn!("Failed to set up direnv: {}", e);
            } else {
                info!("Set up direnv for automatic environment loading");
            }
        }
        
        info!("Type 'exit' to return to your previous directory");
        
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
        app.init(variables.clone())?;

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
