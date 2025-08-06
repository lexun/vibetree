use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::OutputFormat;
use crate::config::VibeTreeConfig;
use crate::git::GitManager;

/// Helper struct for formatting worktree data across different output formats
#[derive(Debug, Serialize)]
pub struct WorktreeDisplayData {
    pub name: String,
    pub status: String,
    pub ports: HashMap<String, u16>,
    #[serde(skip)]
    pub ports_display: String,
}

pub struct DisplayManager<'a> {
    config: &'a VibeTreeConfig,
    vibetree_parent: &'a PathBuf,
}

impl<'a> DisplayManager<'a> {
    pub fn new(config: &'a VibeTreeConfig, vibetree_parent: &'a PathBuf) -> Self {
        Self {
            config,
            vibetree_parent,
        }
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

        let yaml =
            serde_yaml::to_string(&output).context("Failed to serialize worktree data to YAML")?;
        print!("{}", yaml);
        Ok(())
    }

    /// Collect worktree data with validation status for display
    pub fn collect_worktree_data(&self) -> Result<Vec<WorktreeDisplayData>> {
        let mut data = Vec::new();

        for (name, worktree) in &self.config.branches_config.worktrees {
            let worktree_path = if *name == self.config.project_config.main_branch {
                // Main branch lives at repo root
                self.vibetree_parent.clone()
            } else {
                // Other branches live in branches directory
                self.vibetree_parent
                    .join(&self.config.project_config.branches_dir)
                    .join(name)
            };
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
}
