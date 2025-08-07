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

        // Automatically sync to update all discovered worktrees with new configuration
        info!("Running sync to update all worktree configurations");
        self.sync(false)?;

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
        println!("[!] Add '.vibetree/' to your worktree .gitignore files");

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

    /// Update .gitignore to include branches directory
    fn update_gitignore(&self, repo_root: &std::path::Path) -> Result<()> {
        let gitignore_path = repo_root.join(".gitignore");
        let branches_rule = format!("{}/", self.config.project_config.branches_dir);

        // Read existing .gitignore or create empty content
        let mut content = if gitignore_path.exists() {
            std::fs::read_to_string(&gitignore_path).with_context(|| {
                format!("Failed to read .gitignore: {}", gitignore_path.display())
            })?
        } else {
            String::new()
        };

        // Check if rule already exists
        if content.lines().any(|line| line.trim() == branches_rule) {
            println!("[=] .gitignore already contains {} rule", branches_rule);
            return Ok(());
        }

        // Add the rule
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&format!("{}\n", branches_rule));

        std::fs::write(&gitignore_path, content).with_context(|| {
            format!("Failed to update .gitignore: {}", gitignore_path.display())
        })?;

        println!("[+] Added {} to .gitignore", branches_rule);
        Ok(())
    }

    /// Create a new worktree with isolated environment
    pub fn create_worktree(
        &mut self,
        branch_name: String,
        from_branch: Option<String>,
        custom_values: Option<Vec<u16>>,
        dry_run: bool,
    ) -> Result<()> {
        info!("Creating worktree: {}", branch_name);

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

            println!(
                "[?] Dry run - would create worktree '{}' with:",
                branch_name
            );
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
            "[✓] Created worktree '{}' at {}",
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

    /// Synchronize configuration and discover orphaned worktrees
    pub fn sync(&mut self, dry_run: bool) -> Result<()> {
        let mut sync_manager =
            crate::sync::SyncManager::new(&mut self.config, &self.vibetree_parent);
        sync_manager.sync(dry_run)
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
        repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            "Initial commit",
            &tree,
            &[],
        )?;
        
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
