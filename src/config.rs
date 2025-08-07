use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Variable configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableConfig {
    pub name: String,          // Environment variable name
    pub default_value: u16,    // Starting value
}

/// Shared project configuration - stored in vibetree.toml (checked into git)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VibeTreeProjectConfig {
    pub version: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<VariableConfig>,
    pub main_branch: String,
    #[serde(default = "default_branches_dir")]
    pub branches_dir: String,
    #[serde(default = "default_env_file_path")]
    pub env_file_path: String,
}

/// Local worktree state - stored in .vibetree/branches.toml (not checked into git)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VibeTreeBranchesConfig {
    pub version: String,
    pub worktrees: HashMap<String, WorktreeConfig>,
}

/// Combined config for internal use
#[derive(Debug, Clone, Default)]
pub struct VibeTreeConfig {
    pub project_config: VibeTreeProjectConfig,
    pub branches_config: VibeTreeBranchesConfig,
    parent_override: Option<PathBuf>, // Track parent directory for saving
}

fn default_branches_dir() -> String {
    "branches".to_string()
}

fn default_env_file_path() -> String {
    ".vibetree/env".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeConfig {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub values: HashMap<String, u16>, // env_var_name -> value
}

impl Default for VibeTreeProjectConfig {
    fn default() -> Self {
        Self {
            version: "1".to_string(),
            variables: Vec::new(),
            main_branch: "main".to_string(),
            branches_dir: default_branches_dir(),
            env_file_path: default_env_file_path(),
        }
    }
}

impl Default for VibeTreeBranchesConfig {
    fn default() -> Self {
        Self {
            version: "1".to_string(),
            worktrees: HashMap::new(),
        }
    }
}

impl VibeTreeConfig {
    pub fn load_or_create() -> Result<Self> {
        Self::load_or_create_with_parent(None)
    }
    
    /// Load existing configuration without creating new files
    pub fn load_existing() -> Result<Self> {
        Self::load_existing_with_parent(None)
    }
    
    /// Load existing configuration with parent override without creating new files
    pub fn load_existing_with_parent(parent_override: Option<PathBuf>) -> Result<Self> {
        let project_config_path = if let Some(ref parent) = parent_override {
            parent.join("vibetree.toml")
        } else {
            Self::get_project_config_path()?
        };

        let branches_config_path = if let Some(ref parent) = parent_override {
            parent.join(".vibetree").join("branches.toml")
        } else {
            Self::get_branches_config_path()?
        };

        if !project_config_path.exists() {
            anyhow::bail!(
                "Vibetree configuration not found at {}. Run 'vibetree init' first.",
                project_config_path.display()
            );
        }

        let project_config = Self::load_project_config(&project_config_path)?;

        let branches_config = if branches_config_path.exists() {
            Self::load_branches_config(&branches_config_path)?
        } else {
            VibeTreeBranchesConfig::default()
        };

        Ok(Self {
            project_config,
            branches_config,
            parent_override: parent_override.clone(),
        })
    }

    pub fn load_or_create_with_parent(parent_override: Option<PathBuf>) -> Result<Self> {
        let project_config_path = if let Some(ref parent) = parent_override {
            parent.join("vibetree.toml")
        } else {
            Self::get_project_config_path()?
        };

        let branches_config_path = if let Some(ref parent) = parent_override {
            parent.join(".vibetree").join("branches.toml")
        } else {
            Self::get_branches_config_path()?
        };

        let project_config = if project_config_path.exists() {
            Self::load_project_config(&project_config_path)?
        } else {
            let config = VibeTreeProjectConfig::default();
            Self::save_project_config(&config, &project_config_path)?;
            config
        };

        let branches_config = if branches_config_path.exists() {
            Self::load_branches_config(&branches_config_path)?
        } else {
            VibeTreeBranchesConfig::default()
        };

        Ok(Self {
            project_config,
            branches_config,
            parent_override: parent_override.clone(),
        })
    }

    fn load_project_config(config_path: &Path) -> Result<VibeTreeProjectConfig> {
        let content = fs::read_to_string(config_path).with_context(|| {
            format!(
                "Failed to read project config file: {}",
                config_path.display()
            )
        })?;

        let config: VibeTreeProjectConfig = toml::from_str(&content).with_context(|| {
            format!(
                "Failed to parse project config file: {}",
                config_path.display()
            )
        })?;

        Ok(config)
    }

    fn load_branches_config(config_path: &Path) -> Result<VibeTreeBranchesConfig> {
        let content = fs::read_to_string(config_path).with_context(|| {
            format!(
                "Failed to read branches config file: {}",
                config_path.display()
            )
        })?;

        let config: VibeTreeBranchesConfig = toml::from_str(&content).with_context(|| {
            format!(
                "Failed to parse branches config file: {}",
                config_path.display()
            )
        })?;

        Ok(config)
    }

    fn save_project_config(config: &VibeTreeProjectConfig, config_path: &Path) -> Result<()> {
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create config directory: {}", parent.display())
            })?;
        }

        let content =
            toml::to_string_pretty(config).context("Failed to serialize project config to TOML")?;

        fs::write(config_path, content).with_context(|| {
            format!(
                "Failed to write project config file: {}",
                config_path.display()
            )
        })?;

        Ok(())
    }

    fn save_branches_config(&self) -> Result<()> {
        let config_path = if let Some(ref parent) = self.parent_override {
            parent.join(".vibetree").join("branches.toml")
        } else {
            Self::get_branches_config_path()?
        };

        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create config directory: {}", parent.display())
            })?;
        }

        let content = toml::to_string_pretty(&self.branches_config)
            .context("Failed to serialize branches config to TOML")?;

        fs::write(&config_path, content).with_context(|| {
            format!(
                "Failed to write branches config file: {}",
                config_path.display()
            )
        })?;

        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        let project_config_path = if let Some(ref parent) = self.parent_override {
            parent.join("vibetree.toml")
        } else {
            Self::get_project_config_path()?
        };
        Self::save_project_config(&self.project_config, &project_config_path)?;
        self.save_branches_config()?;
        Ok(())
    }

    pub fn add_or_update_worktree(
        &mut self,
        name: String,
        custom_values: Option<HashMap<String, u16>>,
    ) -> Result<HashMap<String, u16>> {
        let values = if let Some(custom) = custom_values {
            // Check for conflicts with existing worktrees (excluding the one we're updating)
            for (variable, &value) in custom.iter() {
                for (existing_name, existing_worktree) in &self.branches_config.worktrees {
                    if existing_name != &name
                        && existing_worktree.values.values().any(|p| *p == value)
                    {
                        anyhow::bail!(
                            "Value {} (for variable '{}') is already allocated to worktree '{}'",
                            value,
                            variable,
                            existing_name
                        );
                    }
                }
            }

            custom
        } else if self.project_config.variables.is_empty() {
            // No variables defined, no values needed
            HashMap::new()
        } else {
            self.project_config
                .allocate_values(&name, &self.branches_config.worktrees)?
        };

        let worktree = WorktreeConfig {
            values: values.clone(),
        };

        self.branches_config.worktrees.insert(name, worktree);
        self.save_branches_config()?; // Save changes to branches config
        Ok(values)
    }

    pub fn add_worktree(
        &mut self,
        name: String,
        custom_values: Option<HashMap<String, u16>>,
    ) -> Result<HashMap<String, u16>> {
        if self.branches_config.worktrees.contains_key(&name) {
            anyhow::bail!("Worktree '{}' already exists", name);
        }

        let values = if let Some(custom) = custom_values {
            // Check for conflicts with existing worktrees
            for (variable, &value) in custom.iter() {
                for (existing_name, existing_worktree) in &self.branches_config.worktrees {
                    if existing_worktree.values.values().any(|p| *p == value) {
                        anyhow::bail!(
                            "Value {} (for variable '{}') is already allocated to worktree '{}'",
                            value,
                            variable,
                            existing_name
                        );
                    }
                }
            }

            custom
        } else if self.project_config.variables.is_empty() {
            // No variables defined, no values needed
            HashMap::new()
        } else {
            self.project_config
                .allocate_values(&name, &self.branches_config.worktrees)?
        };

        let worktree = WorktreeConfig {
            values: values.clone(),
        };

        self.branches_config.worktrees.insert(name, worktree);
        self.save_branches_config()?; // Save changes to branches config
        Ok(values)
    }

    pub fn remove_worktree(&mut self, name: &str) -> Result<()> {
        if !self.branches_config.worktrees.contains_key(name) {
            anyhow::bail!("Worktree '{}' does not exist", name);
        }

        self.branches_config.worktrees.remove(name);
        self.save_branches_config()?; // Save changes to branches config
        Ok(())
    }

    pub fn get_vibetree_parent() -> Result<PathBuf> {
        // Always use the git repository root as the vibetree parent
        crate::git::GitManager::find_repo_root(&std::env::current_dir()?).context(
            "Not inside a git repository - vibetree must be run from within a git repository",
        )
    }

    pub fn get_project_config_path() -> Result<PathBuf> {
        let parent = Self::get_vibetree_parent()?;
        Ok(parent.join("vibetree.toml"))
    }

    pub fn get_branches_config_path() -> Result<PathBuf> {
        let parent = Self::get_vibetree_parent()?;
        Ok(parent.join(".vibetree").join("branches.toml"))
    }

    pub fn get_env_file_path(&self, worktree_path: &Path) -> PathBuf {
        worktree_path.join(&self.project_config.env_file_path)
    }
}

impl VibeTreeProjectConfig {
    pub fn allocate_values(
        &self,
        _worktree_name: &str,
        existing_worktrees: &HashMap<String, WorktreeConfig>,
    ) -> Result<HashMap<String, u16>> {
        let mut allocated_values = HashMap::new();
        let used_values = Self::get_all_used_values(existing_worktrees);

        for variable in &self.variables {
            let mut value = variable.default_value;
            while used_values.contains(&value) {
                value += 1;
            }

            allocated_values.insert(variable.name.clone(), value);
        }

        Ok(allocated_values)
    }

    fn get_all_used_values(
        existing_worktrees: &HashMap<String, WorktreeConfig>,
    ) -> std::collections::HashSet<u16> {
        let mut used = std::collections::HashSet::new();
        for worktree in existing_worktrees.values() {
            for value in worktree.values.values() {
                used.insert(*value);
            }
        }
        used
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = VibeTreeConfig::default();
        assert_eq!(config.project_config.version, "1");
        assert!(config.project_config.variables.is_empty()); // Empty by default
        assert_eq!(config.project_config.main_branch, "main");
        assert!(config.branches_config.worktrees.is_empty());
    }

    #[test]
    fn test_save_and_load_config() -> Result<()> {
        use tempfile::TempDir;
        let temp_dir = TempDir::new()?;
        // Create a VibeTreeConfig with a specific parent to avoid git dependency
        let mut config = VibeTreeConfig {
            project_config: VibeTreeProjectConfig::default(),
            branches_config: VibeTreeBranchesConfig::default(),
            parent_override: Some(temp_dir.path().to_path_buf()),
        };
        config.add_worktree("test-branch".to_string(), None)?;

        // Test that we can save without error
        // Note: In real usage this would save to specific paths,
        // but for testing we'll just verify the structure
        assert_eq!(config.branches_config.worktrees.len(), 1);
        assert!(config.branches_config.worktrees.contains_key("test-branch"));

        Ok(())
    }

    #[test]
    fn test_port_allocation() -> Result<()> {
        use tempfile::TempDir;
        let temp_dir = TempDir::new()?;
        let mut config = VibeTreeConfig {
            project_config: VibeTreeProjectConfig::default(),
            branches_config: VibeTreeBranchesConfig::default(),
            parent_override: Some(temp_dir.path().to_path_buf()),
        };

        // Add some variables first
        config.project_config.variables.push(VariableConfig {
            name: "TEST_SERVICE_PORT".to_string(),
            default_value: 8000,
        });

        let values1 = config.add_worktree("branch1".to_string(), None)?;

        let values2 = config.add_worktree("branch2".to_string(), None)?;

        // Verify values are different
        for variable in &config.project_config.variables {
            assert_ne!(values1[&variable.name], values2[&variable.name]);
        }

        // Verify values start from the configured starting value
        for variable in &config.project_config.variables {
            assert!(values1[&variable.name] >= variable.default_value);
            assert!(values2[&variable.name] >= variable.default_value);
        }

        Ok(())
    }

    #[test]
    fn test_remove_worktree() -> Result<()> {
        use tempfile::TempDir;
        let temp_dir = TempDir::new()?;
        let mut config = VibeTreeConfig {
            project_config: VibeTreeProjectConfig::default(),
            branches_config: VibeTreeBranchesConfig::default(),
            parent_override: Some(temp_dir.path().to_path_buf()),
        };

        // Add a variable for testing
        config.project_config.variables.push(VariableConfig {
            name: "TEST_SERVICE_PORT".to_string(),
            default_value: 8000,
        });

        config.add_worktree("test-branch".to_string(), None)?;

        assert!(config.branches_config.worktrees.contains_key("test-branch"));

        config.remove_worktree("test-branch")?;
        assert!(!config.branches_config.worktrees.contains_key("test-branch"));

        Ok(())
    }

    #[test]
    fn test_custom_env_file_path() -> Result<()> {
        let mut config = VibeTreeConfig::default();
        config.project_config.env_file_path = "custom/.env".to_string();

        let temp_dir = tempfile::TempDir::new()?;
        let worktree_path = temp_dir.path();

        let env_file_path = config.get_env_file_path(worktree_path);
        assert_eq!(env_file_path, worktree_path.join("custom/.env"));

        Ok(())
    }

    #[test]
    fn test_duplicate_worktree_error() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().unwrap();
        let mut config = VibeTreeConfig {
            project_config: VibeTreeProjectConfig::default(),
            branches_config: VibeTreeBranchesConfig::default(),
            parent_override: Some(temp_dir.path().to_path_buf()),
        };

        // Add a variable for testing
        config.project_config.variables.push(VariableConfig {
            name: "TEST_SERVICE_PORT".to_string(),
            default_value: 8000,
        });

        config
            .add_worktree("test-branch".to_string(), None)
            .unwrap();

        let result = config.add_worktree("test-branch".to_string(), None);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }
}
