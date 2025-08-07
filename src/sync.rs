use anyhow::{Context, Result};
use log::info;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::config::VibeTreeConfig;
use crate::env::EnvFileGenerator;
use crate::git::{DiscoveredWorktree, GitManager};

pub struct SyncManager<'a> {
    config: &'a mut VibeTreeConfig,
    vibetree_parent: &'a PathBuf,
}

impl<'a> SyncManager<'a> {
    pub fn new(config: &'a mut VibeTreeConfig, vibetree_parent: &'a PathBuf) -> Self {
        Self {
            config,
            vibetree_parent,
        }
    }

    /// Synchronize configuration and discover orphaned worktrees
    pub fn sync(&mut self, dry_run: bool) -> Result<()> {
        info!("Synchronizing vibetree configuration");

        let repo_path = GitManager::find_repo_root(self.vibetree_parent)
            .context("Not inside a git repository")?;

        // Discover all git worktrees
        let discovered_worktrees = GitManager::discover_worktrees(&repo_path)?;
        let branches_dir = self
            .vibetree_parent
            .join(&self.config.project_config.branches_dir);

        let sync_plan = self.analyze_sync_needs(&discovered_worktrees, &branches_dir)?;

        if !sync_plan.needs_changes() {
            println!("[✓] Configuration is synchronized");
            self.update_env_files(&branches_dir)?;
            return Ok(());
        }

        // Report what would be done
        sync_plan.report();

        if dry_run {
            println!("[?] Dry run - no changes made");
            return Ok(());
        }

        // Apply changes
        self.apply_sync_changes(sync_plan, &branches_dir)?;

        Ok(())
    }

    fn analyze_sync_needs(
        &self,
        discovered_worktrees: &[DiscoveredWorktree],
        branches_dir: &PathBuf,
    ) -> Result<SyncPlan> {
        let mut plan = SyncPlan::new();

        // Check for orphaned git worktrees (not in our config)
        for discovered in discovered_worktrees {
            if let Some(branch_name) = &discovered.branch {
                // Skip bare and detached worktrees
                if discovered.is_bare || discovered.is_detached {
                    continue;
                }

                // Handle main worktree (at repo root) and branch worktrees (in branches dir) differently
                let is_main_worktree = branch_name == &self.config.project_config.main_branch;

                // Use canonical paths to handle symlink differences (like /var vs /private/var on macOS)
                let is_branch_worktree =
                    match (discovered.path.canonicalize(), branches_dir.canonicalize()) {
                        (Ok(discovered_canonical), Ok(branches_canonical)) => {
                            discovered_canonical.starts_with(&branches_canonical)
                        }
                        _ => discovered.path.starts_with(branches_dir), // fallback to original logic
                    };

                if (is_main_worktree || is_branch_worktree)
                    && !self
                        .config
                        .branches_config
                        .worktrees
                        .contains_key(branch_name)
                {
                    plan.orphaned_worktrees
                        .push((branch_name.clone(), discovered.path.clone()));
                }
            }
        }

        // Check for missing worktrees (in config but not in git)
        for (branch_name, _) in &self.config.branches_config.worktrees {
            // Simply check if this branch exists anywhere in git worktrees
            let found = discovered_worktrees
                .iter()
                .any(|wt| wt.branch.as_ref() == Some(branch_name));

            if !found {
                plan.missing_worktrees.push(branch_name.clone());
            }
        }

        // Check for config mismatches (variable changes)
        for (branch_name, worktree_config) in &self.config.branches_config.worktrees {
            // Check if all configured variables exist in current project config
            let current_var_names: std::collections::HashSet<_> = self
                .config
                .project_config
                .variables
                .iter()
                .map(|v| &v.name)
                .collect();
            let worktree_var_names: std::collections::HashSet<_> =
                worktree_config.values.keys().collect();

            if current_var_names != worktree_var_names {
                plan.config_mismatches.push(branch_name.clone());
            }
        }

        Ok(plan)
    }

    fn apply_sync_changes(&mut self, plan: SyncPlan, branches_dir: &PathBuf) -> Result<()> {
        let mut sync_errors = Vec::new();

        // Add orphaned worktrees to config
        for (branch_name, worktree_path) in plan.orphaned_worktrees {
            println!(
                "[+] Adding orphaned worktree '{}' to configuration",
                branch_name
            );

            let ports = if branch_name == self.config.project_config.main_branch {
                self.add_main_worktree(&branch_name, &mut sync_errors)?
            } else {
                // For other worktrees, allocate ports normally
                match self.config.add_worktree(branch_name.clone(), None) {
                    Ok(ports) => ports,
                    Err(e) => {
                        sync_errors
                            .push(format!("Failed to add worktree '{}': {}", branch_name, e));
                        continue;
                    }
                }
            };

            // Generate env file for the discovered worktree
            let env_file_path = self.config.get_env_file_path(&worktree_path);
            if let Err(e) =
                EnvFileGenerator::generate_env_file(&env_file_path, &branch_name, &ports)
            {
                sync_errors.push(format!(
                    "Failed to generate env file for '{}': {}",
                    branch_name, e
                ));
            } else {
                println!(
                    "  [+] Generated environment file at {}",
                    env_file_path.display()
                );
            }
        }

        // Remove missing worktrees from config
        for branch_name in plan.missing_worktrees {
            println!(
                "[-] Removing missing worktree '{}' from configuration",
                branch_name
            );
            if let Err(e) = self.config.remove_worktree(&branch_name) {
                sync_errors.push(format!(
                    "Failed to remove worktree '{}': {}",
                    branch_name, e
                ));
            }
        }

        // Update config mismatches and regenerate env files for all worktrees
        for branch_name in plan.config_mismatches {
            println!("[~] Updating variable configuration for '{}'", branch_name);
            match self
                .config
                .add_or_update_worktree(branch_name.clone(), None)
            {
                Ok(ports) => {
                    // Update env file with new port configuration
                    let worktree_path = if branch_name == self.config.project_config.main_branch {
                        self.vibetree_parent.clone()
                    } else {
                        branches_dir.join(&branch_name)
                    };
                    let env_file_path = self.config.get_env_file_path(&worktree_path);
                    if let Err(e) =
                        EnvFileGenerator::generate_env_file(&env_file_path, &branch_name, &ports)
                    {
                        sync_errors.push(format!(
                            "Failed to update env file for '{}': {}",
                            branch_name, e
                        ));
                    } else {
                        println!(
                            "  [~] Updated environment file at {}",
                            env_file_path.display()
                        );
                    }
                }
                Err(e) => {
                    sync_errors.push(format!(
                        "Failed to update worktree '{}': {}",
                        branch_name, e
                    ));
                }
            }
        }

        // Also regenerate env files for all worktrees that had their ports changed
        self.regenerate_all_env_files(branches_dir, &mut sync_errors)?;

        // Save configuration
        if let Err(e) = self.config.save() {
            sync_errors.push(format!("Failed to save configuration: {}", e));
        }

        if sync_errors.is_empty() {
            println!("[✓] Synchronization completed successfully");
        } else {
            println!(
                "[!] Synchronization completed with {} errors:",
                sync_errors.len()
            );
            for error in sync_errors {
                println!("  [✗] {}", error);
            }
        }

        Ok(())
    }

    fn add_main_worktree(
        &mut self,
        branch_name: &str,
        sync_errors: &mut Vec<String>,
    ) -> Result<HashMap<String, u16>> {
        // For main branch, we need to ensure it gets the base ports
        // First, temporarily remove any conflicting worktree that has those ports
        let mut conflicting_worktree = None;
        let base_ports: std::collections::HashSet<u16> = self
            .config
            .project_config
            .variables
            .iter()
            .map(|v| v.default_value)
            .collect();

        for (existing_name, existing_config) in &self.config.branches_config.worktrees {
            if existing_name != branch_name {
                let existing_ports: std::collections::HashSet<u16> =
                    existing_config.values.values().cloned().collect();
                if !base_ports.is_disjoint(&existing_ports) {
                    conflicting_worktree = Some(existing_name.clone());
                    break;
                }
            }
        }

        // If there's a conflict, reassign the conflicting worktree first
        if let Some(conflicting_name) = conflicting_worktree {
            println!(
                "  [~] Reassigning ports for '{}' to avoid conflict with main branch",
                conflicting_name
            );
            match self
                .config
                .add_or_update_worktree(conflicting_name.clone(), None)
            {
                Ok(_) => {}
                Err(e) => {
                    sync_errors.push(format!(
                        "Failed to reassign ports for '{}': {}",
                        conflicting_name, e
                    ));
                }
            }
        }

        // Now assign base ports to main branch
        let mut main_ports = HashMap::new();
        for variable in &self.config.project_config.variables {
            main_ports.insert(variable.name.clone(), variable.default_value);
        }

        match self
            .config
            .add_or_update_worktree(branch_name.to_string(), Some(main_ports))
        {
            Ok(ports) => Ok(ports),
            Err(e) => {
                sync_errors.push(format!("Failed to add main worktree: {}", e));
                // Return empty ports to continue processing
                Ok(HashMap::new())
            }
        }
    }

    fn update_env_files(&self, branches_dir: &PathBuf) -> Result<()> {
        // Even if no config changes, ensure all env files are up to date
        let mut env_errors = Vec::new();
        for (branch_name, worktree_config) in &self.config.branches_config.worktrees {
            let worktree_path = if *branch_name == self.config.project_config.main_branch {
                self.vibetree_parent.clone()
            } else {
                branches_dir.join(branch_name)
            };
            let env_file_path = self.config.get_env_file_path(&worktree_path);

            // Always regenerate env files to ensure they're current
            if worktree_path.exists() {
                if let Err(e) = EnvFileGenerator::generate_env_file(
                    &env_file_path,
                    branch_name,
                    &worktree_config.values,
                ) {
                    env_errors.push(format!(
                        "Failed to update env file for '{}': {}",
                        branch_name, e
                    ));
                }
            }
        }

        if !env_errors.is_empty() {
            println!(
                "[!] Environment file synchronization completed with {} errors:",
                env_errors.len()
            );
            for error in env_errors {
                println!("  [✗] {}", error);
            }
        }

        Ok(())
    }

    fn regenerate_all_env_files(
        &self,
        branches_dir: &PathBuf,
        sync_errors: &mut Vec<String>,
    ) -> Result<()> {
        for (branch_name, worktree_config) in &self.config.branches_config.worktrees {
            let worktree_path = if *branch_name == self.config.project_config.main_branch {
                self.vibetree_parent.clone()
            } else {
                branches_dir.join(branch_name)
            };
            let env_file_path = self.config.get_env_file_path(&worktree_path);

            // Only update if the env file exists or if the worktree directory exists
            if env_file_path.exists() || worktree_path.exists() {
                if let Err(e) = EnvFileGenerator::generate_env_file(
                    &env_file_path,
                    branch_name,
                    &worktree_config.values,
                ) {
                    sync_errors.push(format!(
                        "Failed to update env file for '{}': {}",
                        branch_name, e
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
struct SyncPlan {
    orphaned_worktrees: Vec<(String, PathBuf)>,
    missing_worktrees: Vec<String>,
    config_mismatches: Vec<String>,
}

impl SyncPlan {
    fn new() -> Self {
        Self {
            orphaned_worktrees: Vec::new(),
            missing_worktrees: Vec::new(),
            config_mismatches: Vec::new(),
        }
    }

    fn needs_changes(&self) -> bool {
        !self.orphaned_worktrees.is_empty()
            || !self.missing_worktrees.is_empty()
            || !self.config_mismatches.is_empty()
    }

    fn report(&self) {
        println!("[!] Synchronization needed:");

        if !self.orphaned_worktrees.is_empty() {
            println!("  [+] Orphaned worktrees to add to config:");
            for (branch, path) in &self.orphaned_worktrees {
                println!("    {} ({})", branch, path.display());
            }
        }

        if !self.missing_worktrees.is_empty() {
            println!("  [-] Missing worktrees to remove from config:");
            for branch in &self.missing_worktrees {
                println!("    {}", branch);
            }
        }

        if !self.config_mismatches.is_empty() {
            println!("  [~] Worktrees with outdated variable configuration:");
            for branch in &self.config_mismatches {
                println!("    {}", branch);
            }
        }
    }
}
