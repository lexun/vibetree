use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::{HashMap, HashSet};

use crate::config::{VariableConfig, VariableType, WorktreeConfig};
use crate::ports::PortManager;
use crate::template::{ComponentType, ParsedTemplate};

// Compile regex once for extracting numbers from template strings
static NUMBER_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\d+").expect("Failed to compile number regex")
});

pub struct VariableAllocator;

impl VariableAllocator {
    /// Allocate values for a worktree based on variable configuration and branch name
    pub fn allocate_values(
        variables: &[VariableConfig],
        branch_name: &str,
        existing_worktrees: &HashMap<String, WorktreeConfig>,
    ) -> Result<HashMap<String, String>> {
        let mut allocated = HashMap::new();

        for variable in variables {
            // Skip if we've already allocated this variable name (first match wins)
            if allocated.contains_key(&variable.name) {
                continue;
            }

            // Check if this variable matches the current branch
            if !Self::matches_branch(variable, branch_name)? {
                continue;
            }

            let value = Self::allocate_variable(variable, existing_worktrees)?;
            allocated.insert(variable.name.clone(), value);
        }

        Ok(allocated)
    }

    /// Check if a variable configuration matches a branch name
    fn matches_branch(variable: &VariableConfig, branch_name: &str) -> Result<bool> {
        match &variable.branch {
            Some(pattern) => {
                let regex = Regex::new(pattern).with_context(|| {
                    format!("Invalid regex pattern for variable '{}': {}", variable.name, pattern)
                })?;
                Ok(regex.is_match(branch_name))
            }
            None => Ok(true), // No pattern means match all branches
        }
    }

    /// Allocate a value for a single variable
    fn allocate_variable(
        variable: &VariableConfig,
        existing_worktrees: &HashMap<String, WorktreeConfig>,
    ) -> Result<String> {
        let value = variable.value.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Variable '{}' must have a 'value' field", variable.name)
        })?;

        match value {
            // Bare number - use type field (or heuristic with warning)
            toml::Value::Integer(num) => {
                let num = u16::try_from(*num).with_context(|| {
                    format!(
                        "Variable '{}' has invalid port/int value: {} (must be 0-65535)",
                        variable.name, num
                    )
                })?;

                match &variable.r#type {
                    Some(VariableType::Port) => {
                        Self::allocate_port_component(num, existing_worktrees)
                    }
                    Some(VariableType::Int) => {
                        Ok(Self::allocate_int_component(num, existing_worktrees))
                    }
                    None => {
                        // Apply heuristic and warn
                        let inferred_type = if variable.name.to_lowercase().contains("port") {
                            "port"
                        } else {
                            "int"
                        };
                        eprintln!(
                            "WARNING: Variable '{}' has a bare number value but no 'type' field. \
                             Treating as '{}' based on name. Add type = \"{}\" to your config.",
                            variable.name, inferred_type, inferred_type
                        );

                        if inferred_type == "port" {
                            Self::allocate_port_component(num, existing_worktrees)
                        } else {
                            Ok(Self::allocate_int_component(num, existing_worktrees))
                        }
                    }
                }
            }

            // String - could be template or static
            toml::Value::String(s) => {
                let template = ParsedTemplate::parse(s)?;

                // If no components, it's a static string
                if !template.has_components() {
                    return Ok(s.clone());
                }

                // Allocate values for each component
                let mut allocated_components = HashMap::new();
                for (idx, component) in template.components.iter().enumerate() {
                    let value = match &component.component_type {
                        ComponentType::Port(base_port) => {
                            Self::allocate_port_component(*base_port, existing_worktrees)?
                        }
                        ComponentType::Int(base_int) => {
                            Self::allocate_int_component(*base_int, existing_worktrees)
                        }
                    };
                    allocated_components.insert(idx, value);
                }

                // Resolve the template with allocated values
                template.resolve(&allocated_components)
            }

            // Unsupported type
            _ => Err(anyhow::anyhow!(
                "Variable '{}' has unsupported value type. Expected integer or string, got: {:?}",
                variable.name,
                value
            )),
        }
    }

    /// Allocate a port component - finds next available port
    fn allocate_port_component(
        base_port: u16,
        existing_worktrees: &HashMap<String, WorktreeConfig>,
    ) -> Result<String> {
        let used_ports = Self::get_all_used_ports(existing_worktrees);
        let mut port = base_port;

        // Find next available port that's not used and is actually available
        loop {
            if !used_ports.contains(&port) && PortManager::check_port_availability(port) {
                return Ok(port.to_string());
            }
            port = port
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("Port overflow while allocating from {}", base_port))?;
        }
    }

    /// Allocate an integer component - simple increment based on worktree count
    fn allocate_int_component(
        base_int: u16,
        existing_worktrees: &HashMap<String, WorktreeConfig>,
    ) -> String {
        // Collect all used integer values to avoid collisions after deletion
        let mut used_ints = std::collections::HashSet::new();
        for worktree in existing_worktrees.values() {
            for value in worktree.values.values() {
                // Try to parse as simple integer
                if let Ok(int) = value.parse::<u16>() {
                    used_ints.insert(int);
                    continue;
                }

                // Also extract integers from template strings
                // Example: "server_9000_v10" should extract both 9000 and 10
                for cap in NUMBER_REGEX.find_iter(value) {
                    if let Ok(int) = cap.as_str().parse::<u16>() {
                        used_ints.insert(int);
                    }
                }
            }
        }

        // Find the next available integer starting from base_int
        let mut value = base_int;
        while used_ints.contains(&value) {
            value = value.saturating_add(1);
        }
        value.to_string()
    }

    /// Extract all ports that are currently in use across all worktrees
    fn get_all_used_ports(existing_worktrees: &HashMap<String, WorktreeConfig>) -> HashSet<u16> {
        let mut used = HashSet::new();
        for worktree in existing_worktrees.values() {
            for value in worktree.values.values() {
                // Try to parse as simple port number
                if let Ok(port) = value.parse::<u16>() {
                    used.insert(port);
                    continue;
                }

                // Also extract port numbers from template strings
                // Example: "server_9000_v10" should extract 9000
                // We use a regex to find all numbers that could be ports
                for cap in NUMBER_REGEX.find_iter(value) {
                    if let Ok(port) = cap.as_str().parse::<u16>() {
                        used.insert(port);
                    }
                }
            }
        }
        used
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocate_static_value() -> Result<()> {
        let variables = vec![VariableConfig {
            name: "ENVIRONMENT".to_string(),
            value: Some(toml::Value::String("production".to_string())),
            r#type: None,
            branch: None,
        }];

        let existing = HashMap::new();
        let allocated = VariableAllocator::allocate_values(&variables, "main", &existing)?;

        assert_eq!(allocated.get("ENVIRONMENT"), Some(&"production".to_string()));
        Ok(())
    }

    #[test]
    fn test_allocate_port_component() -> Result<()> {
        let variables = vec![VariableConfig {
            name: "WEB_PORT".to_string(),
            value: Some(toml::Value::String("{port:53000}".to_string())), // Use high port to avoid conflicts
            r#type: None,
            branch: None,
        }];

        let existing = HashMap::new();
        let allocated = VariableAllocator::allocate_values(&variables, "main", &existing)?;

        // Should allocate an available port (base or higher if base is taken)
        let port_str = allocated.get("WEB_PORT").unwrap();
        let port: u16 = port_str.parse().unwrap();
        assert!(port >= 53000, "Port should be >= 53000, got {}", port);
        Ok(())
    }

    #[test]
    fn test_allocate_int_component() -> Result<()> {
        let variables = vec![VariableConfig {
            name: "INSTANCE".to_string(),
            value: Some(toml::Value::String("{int:1}".to_string())),
            r#type: None,
            branch: None,
        }];

        let existing = HashMap::new();
        let allocated = VariableAllocator::allocate_values(&variables, "main", &existing)?;

        assert_eq!(allocated.get("INSTANCE"), Some(&"1".to_string()));
        Ok(())
    }

    #[test]
    fn test_allocate_template_with_multiple_components() -> Result<()> {
        let variables = vec![VariableConfig {
            name: "SERVICE_NAME".to_string(),
            value: Some(toml::Value::String("server_{port:53100}_v{int:2}".to_string())), // Use high port
            r#type: None,
            branch: None,
        }];

        let existing = HashMap::new();
        let allocated = VariableAllocator::allocate_values(&variables, "main", &existing)?;

        // Verify the pattern matches (port may vary if 53100 is taken)
        let service_name = allocated.get("SERVICE_NAME").unwrap();
        assert!(service_name.starts_with("server_"));
        assert!(service_name.ends_with("_v2"));
        // Extract and verify port number
        let parts: Vec<&str> = service_name.split('_').collect();
        let port: u16 = parts[1].parse().unwrap();
        assert!(port >= 53100, "Port should be >= 53100, got {}", port);
        Ok(())
    }

    #[test]
    fn test_branch_pattern_matching() -> Result<()> {
        let variables = vec![
            VariableConfig {
                name: "ENVIRONMENT".to_string(),
                value: Some(toml::Value::String("production".to_string())),
                r#type: None,
                branch: Some("main".to_string()),
            },
            VariableConfig {
                name: "ENVIRONMENT".to_string(),
                value: Some(toml::Value::String("development".to_string())),
                r#type: None,
                branch: None,
            },
        ];

        let existing = HashMap::new();

        // Test main branch gets production
        let main_allocated = VariableAllocator::allocate_values(&variables, "main", &existing)?;
        assert_eq!(
            main_allocated.get("ENVIRONMENT"),
            Some(&"production".to_string())
        );

        // Test feature branch gets development (first match wins)
        let feature_allocated =
            VariableAllocator::allocate_values(&variables, "feature-auth", &existing)?;
        assert_eq!(
            feature_allocated.get("ENVIRONMENT"),
            Some(&"development".to_string())
        );

        Ok(())
    }

    #[test]
    fn test_bare_number_with_type() -> Result<()> {
        let variables = vec![VariableConfig {
            name: "PORT".to_string(),
            value: Some(toml::Value::Integer(54000)), // Use high port to avoid conflicts
            r#type: Some(VariableType::Port),
            branch: None,
        }];

        let existing = HashMap::new();
        let allocated = VariableAllocator::allocate_values(&variables, "main", &existing)?;

        // Port should be allocated at 54000 or higher if that's taken
        let port: u16 = allocated.get("PORT").unwrap().parse().unwrap();
        assert!(port >= 54000, "Port should be >= 54000, got {}", port);
        Ok(())
    }

    #[test]
    fn test_port_conflict_resolution() -> Result<()> {
        let variables = vec![VariableConfig {
            name: "WEB_PORT".to_string(),
            value: Some(toml::Value::String("{port:3000}".to_string())),
            r#type: None,
            branch: None,
        }];

        // Create existing worktree using port 3000
        let mut existing = HashMap::new();
        let mut main_values = HashMap::new();
        main_values.insert("WEB_PORT".to_string(), "3000".to_string());
        existing.insert(
            "main".to_string(),
            WorktreeConfig {
                values: main_values,
            },
        );

        let allocated = VariableAllocator::allocate_values(&variables, "feature-1", &existing)?;

        // Should allocate next available port (3001 or higher)
        let port_str = allocated.get("WEB_PORT").unwrap();
        let port: u16 = port_str.parse().unwrap();
        assert!(port > 3000);
        Ok(())
    }

    #[test]
    fn test_int_allocation_after_deletion() -> Result<()> {
        // Regression test for integer collision bug after deletion
        let variables = vec![VariableConfig {
            name: "INSTANCE".to_string(),
            value: Some(toml::Value::String("{int:1}".to_string())),
            r#type: None,
            branch: None,
        }];

        // Create first worktree
        let existing1 = HashMap::new();
        let allocated1 = VariableAllocator::allocate_values(&variables, "branch1", &existing1)?;
        assert_eq!(allocated1.get("INSTANCE"), Some(&"1".to_string()));

        // Create second worktree
        let mut existing2 = HashMap::new();
        let mut values1 = HashMap::new();
        values1.insert("INSTANCE".to_string(), "1".to_string());
        existing2.insert("branch1".to_string(), WorktreeConfig { values: values1 });
        let allocated2 = VariableAllocator::allocate_values(&variables, "branch2", &existing2)?;
        assert_eq!(allocated2.get("INSTANCE"), Some(&"2".to_string()));

        // Simulate deletion of branch1 and creation of branch3
        // This should get "1" again (filling the gap)
        let mut existing3 = HashMap::new();
        let mut values2 = HashMap::new();
        values2.insert("INSTANCE".to_string(), "2".to_string());
        existing3.insert("branch2".to_string(), WorktreeConfig { values: values2 });
        let allocated3 = VariableAllocator::allocate_values(&variables, "branch3", &existing3)?;
        assert_eq!(
            allocated3.get("INSTANCE"),
            Some(&"1".to_string()),
            "Should reuse the lowest available integer"
        );

        // Create branch4 - should get "3" not "2" (no collision)
        let mut existing4 = HashMap::new();
        let mut values2b = HashMap::new();
        values2b.insert("INSTANCE".to_string(), "2".to_string());
        existing4.insert("branch2".to_string(), WorktreeConfig { values: values2b.clone() });
        let mut values3 = HashMap::new();
        values3.insert("INSTANCE".to_string(), "1".to_string());
        existing4.insert("branch3".to_string(), WorktreeConfig { values: values3 });
        let allocated4 = VariableAllocator::allocate_values(&variables, "branch4", &existing4)?;
        assert_eq!(
            allocated4.get("INSTANCE"),
            Some(&"3".to_string()),
            "Should allocate next available integer without collision"
        );

        Ok(())
    }
}
