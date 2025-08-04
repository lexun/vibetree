//! Vibetree - A CLI tool for managing isolated development environments using git worktrees
//!
//! This library provides functionality for:
//! - Managing git worktrees with isolated port configurations
//! - Automatic port allocation and conflict resolution
//! - Environment file generation for process orchestration
//! - Configuration management and state reconciliation

pub mod cli;
pub mod config;
pub mod env;
pub mod git;
pub mod ports;

/// Current version of vibetree from Cargo.toml
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// Re-export public types for external use
pub use cli::{Cli, Commands, OutputFormat};
pub use config::{VibeTreeConfig, WorktreeConfig};
pub use env::EnvFileGenerator;
pub use git::{GitManager, WorktreeValidation};

use anyhow::{Context, Result};
use log::{info, warn};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;

pub use ports::PortManager;

/// Helper struct for formatting worktree data across different output formats
#[derive(Debug, Serialize)]
struct WorktreeDisplayData {
    name: String,
    status: String,
    ports: HashMap<String, u16>,
    #[serde(skip)]
    ports_display: String,
}

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

    /// Initialize vibetree configuration
    pub fn init(&mut self, services: Vec<String>, convert_repo: bool) -> Result<()> {
        info!("Initializing vibetree configuration");

        // Handle repository conversion if requested
        if convert_repo {
            self.convert_existing_repo(&services)?;
            return Ok(());
        }

        // Clear existing configuration to start fresh
        self.config.project_config.services.clear();

        // Parse and update services if provided
        if !services.is_empty() {
            for service_spec in &services {
                if let Some((service, port_str)) = service_spec.split_once(':') {
                    let port = port_str.parse::<u16>()
                        .with_context(|| format!("Invalid port '{}' for service '{}'", port_str, service))?;
                    
                    self.config.project_config.services.insert(service.to_string(), port);
                } else {
                    // Service without port - use default incremental port
                    let default_port = 8000 + (self.config.project_config.services.len() as u16 * 100);
                    self.config.project_config.services.insert(service_spec.clone(), default_port);
                }
            }
        }

        self.save_config()?;

        // Create .vibetree/env file in the main worktree if services are configured
        if !self.config.project_config.services.is_empty() {
            let current_dir = std::env::current_dir().context("Failed to get current directory")?;
            let main_branch = GitManager::get_current_branch(&current_dir)
                .unwrap_or_else(|_| self.config.project_config.main_branch.clone());
            
            EnvFileGenerator::generate_env_file(
                &current_dir,
                &main_branch,
                &self.config.project_config.services,
                &self.config.project_config.env_var_names,
            )
            .context("Failed to generate environment file for main worktree")?;
        }

        println!(
            "✅ Initialized vibetree configuration at {}",
            VibeTreeConfig::get_project_config_path().unwrap().display()
        );
        println!(
            "📝 Configured services: {}",
            self.config.project_config.services.keys().cloned().collect::<Vec<_>>().join(", ")
        );
        if !self.config.project_config.services.is_empty() {
            println!("🚀 Environment file created at .vibetree/env");
            println!("   Use with process orchestrators like: docker compose --env-file .vibetree/env up");
        }
        println!("💡 Add '.vibetree/' to your worktree .gitignore files");

        Ok(())
    }

    /// Convert existing git repository to vibetree-managed structure in-place
    fn convert_existing_repo(&mut self, services: &[String]) -> Result<()> {
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

        // Configure services
        if !services.is_empty() {
            for service_spec in services {
                if let Some((service, port_str)) = service_spec.split_once(':') {
                    let port = port_str.parse::<u16>()
                        .with_context(|| format!("Invalid port '{}' for service '{}'", port_str, service))?;
                    
                    self.config.project_config.services.insert(service.to_string(), port);
                } else {
                    // Service without port - use default incremental port
                    let default_port = 8000 + (self.config.project_config.services.len() as u16 * 100);
                    self.config.project_config.services.insert(service_spec.clone(), default_port);
                }
            }
        }

        // Save the configuration
        self.save_config()?;

        // Create .vibetree/env file in the main worktree if services are configured
        if !self.config.project_config.services.is_empty() {
            EnvFileGenerator::generate_env_file(
                &current_dir,
                &current_branch,
                &self.config.project_config.services,
                &self.config.project_config.env_var_names,
            )
            .context("Failed to generate environment file for main worktree")?;
        }

        println!("✅ Successfully converted repository to vibetree-managed structure");
        println!(
            "📝 Configured services: {}",
            self.config.project_config.services.keys().cloned().collect::<Vec<_>>().join(", ")
        );
        if !self.config.project_config.services.is_empty() {
            println!("🚀 Environment file created at .vibetree/env");
            println!("   Use with process orchestrators like: docker compose --env-file .vibetree/env up");
        }
        println!(
            "🌿 Current branch '{}' remains active in repository root",
            current_branch
        );
        println!(
            "📁 Future worktrees will be created in {}/",
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
            println!("📝 .gitignore already contains {} rule", branches_rule);
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

        println!("📝 Added {} to .gitignore", branches_rule);
        Ok(())
    }

    /// Create a new worktree with isolated environment
    pub fn create_worktree(
        &mut self,
        branch_name: String,
        from_branch: Option<String>,
        custom_ports: Option<Vec<u16>>,
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

        // Convert custom ports Vec to HashMap if provided
        let custom_port_map = if let Some(custom) = custom_ports {
            // Validate port count matches service count
            if custom.len() != self.config.project_config.services.len() {
                anyhow::bail!(
                    "Expected {} ports for services: {}",
                    self.config.project_config.services.len(),
                    self.config.project_config.services.keys().cloned().collect::<Vec<_>>().join(", ")
                );
            }

            let mut port_map = HashMap::new();
            let service_names: Vec<String> = self.config.project_config.services.keys().cloned().collect();
            for (service, port) in service_names.iter().zip(custom.iter()) {
                port_map.insert(service.clone(), *port);
            }
            Some(port_map)
        } else {
            None
        };

        // First, add the worktree to configuration (this handles port allocation and validation)
        let ports = self
            .config
            .add_worktree(branch_name.clone(), custom_port_map)?;

        // Validate that allocated ports are actually available on the system
        let port_list: Vec<u16> = ports.values().cloned().collect();
        let availability = PortManager::check_ports_availability(&port_list);
        let unavailable: Vec<u16> = availability
            .iter()
            .filter_map(|(&port, &available)| if !available { Some(port) } else { None })
            .collect();

        if !unavailable.is_empty() {
            // Remove the worktree from config since port validation failed
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

        if dry_run {
            // Remove from configuration since this was just a dry run
            self.config.remove_worktree(&branch_name)?;

            println!("🔍 Dry run - would create worktree '{}' with:", branch_name);
            println!("  📁 Path: {}", worktree_path.display());
            println!(
                "  🌿 Base branch: {}",
                from_branch.as_deref().unwrap_or("HEAD")
            );
            println!("  🔌 Ports:");
            for (service, port) in &ports {
                println!("    {} → {}", service, port);
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
        EnvFileGenerator::generate_env_file(
            &worktree_path,
            &branch_name,
            &ports,
            &self.config.project_config.env_var_names,
        )
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
            "✅ Created worktree '{}' at {}",
            branch_name,
            worktree_path.display()
        );
        println!("🔌 Allocated ports:");
        for (service, port) in &ports {
            println!("  {} → {}", service, port);
        }
        println!("🚀 Environment file created at .vibetree/env");
        println!("   Use with process orchestrators like: docker compose --env-file .vibetree/env up");

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
            println!("⚠️  Make sure no important processes are using the allocated ports before removing");
            print!("Are you sure you want to remove worktree '{}'? (y/N): ", branch_name);
            io::stdout().flush().context("Failed to flush stdout")?;
            
            let mut input = String::new();
            io::stdin().read_line(&mut input)
                .context("Failed to read confirmation input")?;
            
            let input = input.trim().to_lowercase();
            if input != "y" && input != "yes" {
                println!("❌ Cancelled removal of worktree '{}'", branch_name);
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

        println!("✅ Removed worktree '{}'", branch_name);
        if keep_branch {
            println!("🌿 Kept git branch '{}'", branch_name);
        }

        Ok(())
    }

    /// List all worktrees and their configurations
    pub fn list_worktrees(&self, format: Option<OutputFormat>) -> Result<()> {
        let format = format.unwrap_or(OutputFormat::Table);

        match format {
            OutputFormat::Table => self.list_worktrees_table(),
            OutputFormat::Json => self.list_worktrees_json(),
            OutputFormat::Yaml => self.list_worktrees_yaml(),
        }
    }

    fn list_worktrees_table(&self) -> Result<()> {
        let worktree_data = self.collect_worktree_data()?;

        if worktree_data.is_empty() {
            println!("No worktrees configured");
            return Ok(());
        }

        println!(
            "{:<20} {:<15} {:<15} {:<50}",
            "Name", "Branch", "Status", "Ports"
        );
        println!("{}", "-".repeat(100));

        for data in worktree_data {
            println!(
                "{:<20} {:<15} {:<15} {:<50}",
                data.name, data.name, data.status, data.ports_display
            );
        }

        Ok(())
    }

    fn list_worktrees_json(&self) -> Result<()> {
        let worktree_data = self.collect_worktree_data()?;

        let output: HashMap<&str, &WorktreeDisplayData> = worktree_data
            .iter()
            .map(|data| (data.name.as_str(), data))
            .collect();

        let json = serde_json::to_string_pretty(&output)
            .context("Failed to serialize worktree data to JSON")?;
        println!("{}", json);
        Ok(())
    }

    fn list_worktrees_yaml(&self) -> Result<()> {
        let worktree_data = self.collect_worktree_data()?;

        let output: HashMap<&str, &WorktreeDisplayData> = worktree_data
            .iter()
            .map(|data| (data.name.as_str(), data))
            .collect();

        let yaml = serde_yaml::to_string(&output)
            .context("Failed to serialize worktree data to YAML")?;
        print!("{}", yaml);
        Ok(())
    }

    /// Collect worktree data with validation status for display
    fn collect_worktree_data(&self) -> Result<Vec<WorktreeDisplayData>> {
        let mut data = Vec::new();

        for (name, worktree) in &self.config.branches_config.worktrees {
            let worktree_path = self
                .vibetree_parent
                .join(&self.config.project_config.branches_dir)
                .join(name);
            let validation = GitManager::validate_worktree_state(&worktree_path)?;

            let status = if !validation.exists {
                "Missing"
            } else if !validation.is_git_worktree {
                "Not Git"
            } else if !validation.has_env_file {
                "No Env"
            } else {
                "OK"
            };

            let ports_display = worktree
                .ports
                .iter()
                .map(|(service, port)| format!("{}:{}", service, port))
                .collect::<Vec<_>>()
                .join(", ");

            data.push(WorktreeDisplayData {
                name: name.clone(),
                status: status.to_string(),
                ports: worktree.ports.clone(),
                ports_display,
            });
        }

        Ok(data)
    }

    fn save_config(&self) -> Result<()> {
        self.config.save().context("Failed to save configuration")
    }

    // Getter methods to allow tests to access private fields
    pub fn get_services(&self) -> &std::collections::HashMap<String, u16> {
        &self.config.project_config.services
    }

    pub fn get_worktrees(&self) -> &std::collections::HashMap<String, WorktreeConfig> {
        &self.config.branches_config.worktrees
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
        let app = VibeTreeApp::with_parent(temp_dir.path().to_path_buf())?;
        Ok((temp_dir, app))
    }

    #[test]
    fn test_new_app() -> Result<()> {
        let (_temp_dir, app) = setup_test_app()?;
        assert_eq!(app.config.project_config.version, "1");
        assert_eq!(app.config.project_config.main_branch, "main");
        // Services should be empty by default - user specifies them during init
        assert!(app.config.project_config.services.is_empty());
        Ok(())
    }

    #[test]
    fn test_init() -> Result<()> {
        let (_temp_dir, mut app) = setup_test_app()?;

        let services = vec!["postgres".to_string(), "redis".to_string()];
        app.init(services.clone(), false)?;

        // Services should be updated after init
        // Verify services were configured
        assert!(VibeTreeConfig::get_project_config_path().unwrap().exists());
        assert!(app.config.project_config.services.contains_key("postgres"));
        assert!(app.config.project_config.services.contains_key("redis"));

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
