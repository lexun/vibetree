use anyhow::Result;
use std::collections::{HashMap, HashSet};

use crate::config::{VariableConfig, VibeTreeConfig};
use crate::ports::PortManager;

pub struct ConfigValidator;

impl ConfigValidator {
    /// Validate the entire configuration for consistency and conflicts
    pub fn validate_config(config: &VibeTreeConfig) -> Result<ValidationResult> {
        let mut result = ValidationResult::new();

        // Validate project configuration
        Self::validate_project_config(&config.project_config.variables, &mut result);

        // Validate worktree configurations
        Self::validate_worktree_configs(config, &mut result);

        // Validate value allocations
        Self::validate_value_allocations(config, &mut result);

        Ok(result)
    }

    /// Validate project-level variable configuration
    fn validate_project_config(variables: &[VariableConfig], result: &mut ValidationResult) {
        // Check for duplicate variable names
        let mut seen_names = HashSet::new();
        let mut seen_ports = HashSet::new();

        for variable in variables {
            // Check for duplicate variable names
            if !seen_names.insert(&variable.name) {
                result.add_error(format!("Duplicate variable name: '{}'", variable.name));
            }

            // Check for duplicate default values
            if !seen_ports.insert(variable.default_value) {
                result.add_error(format!(
                    "Duplicate default value: {} (used by '{}')",
                    variable.default_value, variable.name
                ));
            }

            // Validate value is in valid range
            if variable.default_value == 0 {
                result.add_error(format!("Invalid value 0 for variable '{}'", variable.name));
            }

            // Check for system reserved ports (still relevant for port variables)
            let reserved_ports = PortManager::get_system_reserved_ports();
            if reserved_ports.contains(&variable.default_value) {
                result.add_warning(format!(
                    "Variable '{}' uses system reserved port {}",
                    variable.name, variable.default_value
                ));
            }

            // Validate variable name format
            if !Self::is_valid_env_var_name(&variable.name) {
                result.add_warning(format!(
                    "Variable name '{}' doesn't follow typical environment variable conventions",
                    variable.name
                ));
            }
        }
    }

    /// Validate worktree configurations
    fn validate_worktree_configs(config: &VibeTreeConfig, result: &mut ValidationResult) {
        let project_var_names: HashSet<_> = config
            .project_config
            .variables
            .iter()
            .map(|v| &v.name)
            .collect();

        for (worktree_name, worktree_config) in &config.branches_config.worktrees {
            let worktree_var_names: HashSet<_> = worktree_config.values.keys().collect();

            // Check if worktree has variables that don't exist in project config
            for var_name in &worktree_var_names {
                if !project_var_names.contains(var_name) {
                    result.add_error(format!(
                        "Worktree '{}' has variable '{}' not defined in project configuration",
                        worktree_name, var_name
                    ));
                }
            }

            // Check if worktree is missing variables from project config
            for var_name in &project_var_names {
                if !worktree_var_names.contains(var_name) {
                    result.add_warning(format!(
                        "Worktree '{}' is missing variable '{}' from project configuration",
                        worktree_name, var_name
                    ));
                }
            }

            // Validate branch name
            if worktree_name.contains('/') || worktree_name.contains('\\') {
                result.add_warning(format!(
                    "Worktree name '{}' contains path separators which may cause issues",
                    worktree_name
                ));
            }
        }
    }

    /// Validate value allocations across all worktrees
    fn validate_value_allocations(config: &VibeTreeConfig, result: &mut ValidationResult) {
        let mut port_usage: HashMap<u16, Vec<String>> = HashMap::new();

        // Collect all value usage
        for (worktree_name, worktree_config) in &config.branches_config.worktrees {
            for (var_name, &value) in &worktree_config.values {
                port_usage
                    .entry(value)
                    .or_insert_with(Vec::new)
                    .push(format!("{}:{}", worktree_name, var_name));
            }
        }

        // Check for value conflicts
        for (value, usage) in port_usage {
            if usage.len() > 1 {
                result.add_error(format!(
                    "Value {} is used by multiple services: {}",
                    value,
                    usage.join(", ")
                ));
            }
        }
    }

    /// Check if a variable name follows typical environment variable conventions
    fn is_valid_env_var_name(name: &str) -> bool {
        if name.is_empty() {
            return false;
        }

        // Should start with letter or underscore
        if !name.chars().next().unwrap().is_ascii_alphabetic() && !name.starts_with('_') {
            return false;
        }

        // Should contain only alphanumeric characters and underscores
        name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    }

    /// Quick validation to check if basic configuration is valid
    pub fn quick_validate(config: &VibeTreeConfig) -> bool {
        // Check for basic consistency
        let project_var_count = config.project_config.variables.len();

        // All worktrees should have the same number of value mappings
        for worktree_config in config.branches_config.worktrees.values() {
            if worktree_config.values.len() != project_var_count {
                return false;
            }
        }

        true
    }
}

#[derive(Debug)]
pub struct ValidationResult {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl ValidationResult {
    fn new() -> Self {
        Self {
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn add_error(&mut self, error: String) {
        self.errors.push(error);
    }

    fn add_warning(&mut self, warning: String) {
        self.warnings.push(warning);
    }

    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }

    pub fn report(&self) {
        if !self.errors.is_empty() {
            println!("Configuration errors:");
            for error in &self.errors {
                println!("  [✗] {}", error);
            }
        }

        if !self.warnings.is_empty() {
            println!("Configuration warnings:");
            for warning in &self.warnings {
                println!("  [⚠] {}", warning);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{VibeTreeBranchesConfig, VibeTreeProjectConfig, WorktreeConfig};

    #[test]
    fn test_valid_env_var_name() {
        assert!(ConfigValidator::is_valid_env_var_name("POSTGRES_PORT"));
        assert!(ConfigValidator::is_valid_env_var_name("_PRIVATE_VAR"));
        assert!(ConfigValidator::is_valid_env_var_name("API_V2_PORT"));

        assert!(!ConfigValidator::is_valid_env_var_name(""));
        assert!(!ConfigValidator::is_valid_env_var_name("123_PORT"));
        assert!(!ConfigValidator::is_valid_env_var_name("PORT-NAME"));
        assert!(!ConfigValidator::is_valid_env_var_name("port.name"));
    }

    #[test]
    fn test_duplicate_variable_detection() {
        let variables = vec![
            VariableConfig {
                name: "POSTGRES_PORT".to_string(),
                default_value: 5432,
            },
            VariableConfig {
                name: "POSTGRES_PORT".to_string(), // Duplicate name
                default_value: 5433,
            },
        ];

        let mut result = ValidationResult::new();
        ConfigValidator::validate_project_config(&variables, &mut result);

        assert!(!result.is_valid());
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.contains("Duplicate variable name"))
        );
    }

    #[test]
    fn test_port_conflict_detection() {
        use std::collections::HashMap;

        let mut config = VibeTreeConfig::default();
        config.project_config = VibeTreeProjectConfig {
            version: "1".to_string(),
            variables: vec![VariableConfig {
                name: "POSTGRES_PORT".to_string(),
                default_value: 5432,
            }],
            main_branch: "main".to_string(),
            branches_dir: "branches".to_string(),
            env_file_path: ".vibetree/env".to_string(),
        };

        // Create two worktrees with conflicting value assignments
        let mut worktree1_values = HashMap::new();
        worktree1_values.insert("POSTGRES_PORT".to_string(), 5432);

        let mut worktree2_values = HashMap::new();
        worktree2_values.insert("POSTGRES_PORT".to_string(), 5432); // Same value!

        config.branches_config = VibeTreeBranchesConfig {
            version: "1".to_string(),
            worktrees: HashMap::from([
                (
                    "branch1".to_string(),
                    WorktreeConfig {
                        values: worktree1_values,
                    },
                ),
                (
                    "branch2".to_string(),
                    WorktreeConfig {
                        values: worktree2_values,
                    },
                ),
            ]),
        };

        let result = ConfigValidator::validate_config(&config).unwrap();
        assert!(!result.is_valid());
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.contains("Value 5432 is used by multiple services"))
        );
    }
}
