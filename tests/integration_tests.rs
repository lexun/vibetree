use anyhow::Result;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;
use vibetree::{EnvFileGenerator, GitManager, VibeTreeApp};

/// Helper to set up a complete test environment with git repo and vibetree
struct IntegrationTestSetup {
    #[allow(dead_code)] // Needed to keep the temp directory alive
    temp_dir: TempDir,
    repo_path: PathBuf,
}

impl IntegrationTestSetup {
    /// Create a new integration test setup with:
    /// - Temporary directory containing the git repository
    /// - Git repository is the root directory (no subdirectory)
    /// - All worktrees will be in .vibetree/branches/ subdirectory
    /// - Initial commit to make it usable
    fn new() -> Result<Self> {
        let temp_dir = TempDir::new()?;
        let repo_path = temp_dir.path().to_path_buf(); // Git repo is the temp dir itself

        // Initialize git repository
        let output = Command::new("git")
            .args(["init"])
            .current_dir(&repo_path)
            .output()?;

        if !output.status.success() {
            anyhow::bail!(
                "Failed to initialize git repo: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        // Configure git user (required for commits)
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(&repo_path)
            .output()?;

        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&repo_path)
            .output()?;

        // Create initial file and commit
        fs::write(
            repo_path.join("README.md"),
            "# Test Repository\n\nThis is a test repo for vibetree integration tests.\n",
        )?;
        fs::write(repo_path.join(".gitignore"), "# Initial gitignore\n*.log\n")?;

        Command::new("git")
            .args(["add", "."])
            .current_dir(&repo_path)
            .output()?;

        let commit_output = Command::new("git")
            .args(["commit", "-m", "Initial commit"])
            .current_dir(&repo_path)
            .output()?;

        if !commit_output.status.success() {
            anyhow::bail!(
                "Failed to create initial commit: {}",
                String::from_utf8_lossy(&commit_output.stderr)
            );
        }

        Ok(Self {
            temp_dir,
            repo_path,
        })
    }

    /// Create a VibeTreeApp instance for this test setup
    fn create_app(&self) -> Result<VibeTreeApp> {
        // Use with_parent to avoid global environment variable conflicts
        VibeTreeApp::with_parent(self.repo_path.clone())
    }

    /// Get the path to the vibetree config file
    fn config_path(&self) -> PathBuf {
        self.repo_path.join("vibetree.toml")
    }

    /// Helper to check if a worktree directory exists
    fn worktree_exists(&self, name: &str) -> bool {
        self.repo_path
            .join(".vibetree")
            .join("branches")
            .join(name)
            .exists()
    }

    /// Helper to check if env file exists for a worktree
    fn env_file_exists(&self, name: &str) -> bool {
        self.repo_path
            .join(".vibetree")
            .join("branches")
            .join(name)
            .join(".vibetree")
            .join("env")
            .exists()
    }

    /// Helper to read env file contents
    fn read_env_file(&self, name: &str) -> Result<String> {
        let env_path = self
            .repo_path
            .join(".vibetree")
            .join("branches")
            .join(name)
            .join(".vibetree")
            .join("env");
        Ok(fs::read_to_string(env_path)?)
    }

    /// Helper to check if .gitignore contains .vibetree/
    fn gitignore_has_vibetree(&self, name: &str) -> Result<bool> {
        let gitignore_path = self
            .repo_path
            .join(".vibetree")
            .join("branches")
            .join(name)
            .join(".gitignore");
        if !gitignore_path.exists() {
            return Ok(false);
        }
        let content = fs::read_to_string(gitignore_path)?;
        Ok(content.contains(".vibetree/"))
    }

    /// Helper to run git commands in the main repo
    fn run_git_cmd(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.repo_path)
            .output()?;

        if !output.status.success() {
            anyhow::bail!(
                "Git command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

#[test]
fn test_complete_vibetree_workflow() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Step 1: Initialize vibetree with custom variables
    let variables = vec![
        "postgres".to_string(),
        "redis".to_string(),
        "api".to_string(),
    ];
    app.init(variables.clone(), false)?;

    // Verify config file was created
    assert!(setup.config_path().exists());

    // Verify variables were configured
    assert_eq!(app.get_variables().len(), 3);
    for variable in &variables {
        let expected_env_var = variable.to_uppercase();
        assert!(
            app.get_variables()
                .iter()
                .any(|v| v.name == expected_env_var)
        );
    }

    // Step 2: Create first worktree
    app.add_worktree(
        "feature-auth".to_string(),
        None,  // from main branch
        None,  // auto-allocate ports
        false, // not dry run
    )?;

    // Verify worktree was created
    assert!(setup.worktree_exists("feature-auth"));
    assert!(setup.env_file_exists("feature-auth"));

    // Verify env file content
    let env_content = setup.read_env_file("feature-auth")?;
    assert!(env_content.contains("POSTGRES="));
    assert!(env_content.contains("REDIS="));
    assert!(env_content.contains("API="));
    assert!(env_content.contains("# Generated by vibetree"));

    // Verify git worktree was created
    let worktrees_output = setup.run_git_cmd(&["worktree", "list"])?;
    assert!(worktrees_output.contains("feature-auth"));

    // Step 3: Create second worktree with custom values
    let custom_values = vec![5555, 6666, 7777];
    app.add_worktree(
        "feature-payments".to_string(),
        Some("main".to_string()), // explicitly from main
        Some(custom_values.clone()),
        false,
    )?;

    // Verify second worktree
    assert!(setup.worktree_exists("feature-payments"));
    assert!(setup.env_file_exists("feature-payments"));

    let env_content2 = setup.read_env_file("feature-payments")?;
    assert!(env_content2.contains("POSTGRES=5555"));
    assert!(env_content2.contains("REDIS=6666"));
    assert!(env_content2.contains("API=7777"));

    // Step 4: Test dry run creation
    app.add_worktree(
        "feature-dryrun".to_string(),
        None,
        None,
        true, // dry run
    )?;

    // Verify dry run didn't create actual worktree
    assert!(!setup.worktree_exists("feature-dryrun"));

    // Step 5: Test list worktrees
    app.list_worktrees(None)?; // Should not panic

    // Verify configuration state
    assert_eq!(app.get_worktrees().len(), 3); // main + feature-auth and feature-payments
    assert!(app.get_worktrees().contains_key("feature-auth"));
    assert!(app.get_worktrees().contains_key("feature-payments"));

    // Step 6: Test gitignore suggestion
    // feature-auth should not have .vibetree/ in gitignore yet
    assert!(!setup.gitignore_has_vibetree("feature-auth")?);

    // Add .vibetree/ to gitignore
    EnvFileGenerator::add_to_gitignore(
        &setup
            .repo_path
            .join(".vibetree")
            .join("branches")
            .join("feature-auth"),
    )?;
    assert!(setup.gitignore_has_vibetree("feature-auth")?);

    // Step 7: Remove a worktree
    app.remove_worktree_for_test(
        "feature-payments".to_string(),
        false, // not forced
        false, // don't keep branch
    )?;

    // Verify worktree was removed
    assert!(!setup.worktree_exists("feature-payments"));
    assert_eq!(app.get_worktrees().len(), 2); // main + feature-auth
    assert!(!app.get_worktrees().contains_key("feature-payments"));

    // Verify git worktree was cleaned up
    let worktrees_output_after = setup.run_git_cmd(&["worktree", "list"])?;
    assert!(!worktrees_output_after.contains("feature-payments"));

    // Step 8: Verify remaining worktree still works
    assert!(setup.worktree_exists("feature-auth"));
    assert!(app.get_worktrees().contains_key("feature-auth"));

    Ok(())
}

#[test]
fn test_port_conflict_detection() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize with variables
    app.init(vec!["postgres".to_string(), "redis".to_string()], false)?;

    // Create first worktree
    app.add_worktree("branch1".to_string(), None, None, false)?;

    // Try to create second worktree with conflicting ports
    let first_worktree = &app.get_worktrees()["branch1"];
    let conflicting_ports: Vec<u16> = first_worktree.values.values().cloned().collect();

    // This should fail due to port conflicts
    let result = app.add_worktree("branch2".to_string(), None, Some(conflicting_ports), false);

    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("already allocated")
    );

    // Verify second worktree was not created
    assert!(!setup.worktree_exists("branch2"));
    assert_eq!(app.get_worktrees().len(), 2); // main + branch1

    Ok(())
}

#[test]
fn test_worktree_validation() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize and create worktree
    app.init(vec!["postgres".to_string()], false)?;
    app.add_worktree("test-branch".to_string(), None, None, false)?;

    let worktree_path = setup
        .repo_path
        .join(".vibetree")
        .join("branches")
        .join("test-branch");

    // Test validation of complete worktree
    let validation = GitManager::validate_worktree_state(&worktree_path)?;
    assert!(validation.exists);
    assert!(validation.is_git_worktree);
    assert!(validation.has_vibetree_dir);
    assert!(validation.has_env_file);
    assert!(validation.branch_name.is_some());

    // Test validation of missing directory
    let missing_path = setup.repo_path.join("nonexistent");
    let validation_missing = GitManager::validate_worktree_state(&missing_path)?;
    assert!(!validation_missing.exists);
    assert!(!validation_missing.is_git_worktree);

    Ok(())
}

#[test]
fn test_error_handling() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    app.init(vec!["postgres".to_string()], false)?;

    // Test creating worktree with empty name
    let result = app.add_worktree("".to_string(), None, None, false);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("empty"));

    // Test creating duplicate worktree
    app.add_worktree("test".to_string(), None, None, false)?;
    let result = app.add_worktree("test".to_string(), None, None, false);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("already exists"));

    // Test removing non-existent worktree
    let result = app.remove_worktree_for_test("nonexistent".to_string(), false, false);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("does not exist"));

    // Test wrong number of custom ports
    let result = app.add_worktree(
        "test2".to_string(),
        None,
        Some(vec![5432]), // Only 1 port for 1 service is correct
        false,
    );
    // This should succeed since we have 1 port for 1 service
    assert!(result.is_ok());

    // Now test with wrong number of ports
    let result = app.add_worktree(
        "test3".to_string(),
        None,
        Some(vec![5432, 6379]), // 2 ports for 1 service should fail
        false,
    );
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Expected 1 values")
    );

    Ok(())
}

#[test]
fn test_config_persistence() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;

    // Create and configure first app instance
    {
        let mut app1 = setup.create_app()?;
        app1.init(vec!["postgres".to_string(), "redis".to_string()], false)?;
        app1.add_worktree("persistent-test".to_string(), None, None, false)?;

        assert_eq!(app1.get_worktrees().len(), 2); // main + persistent-test
    } // app1 goes out of scope

    // Create new app instance - should load existing config
    {
        let app2 = setup.create_app()?;

        // Verify configuration was persisted and loaded
        assert_eq!(app2.get_variables().len(), 2);
        assert_eq!(app2.get_worktrees().len(), 2); // main + persistent-test
        assert!(app2.get_worktrees().contains_key("persistent-test"));
        assert!(app2.get_variables().iter().any(|v| v.name == "POSTGRES"));
        assert!(app2.get_variables().iter().any(|v| v.name == "REDIS"));
    }

    Ok(())
}

#[test]
fn test_serviceless_vibetree_workflow() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Step 1: Initialize vibetree with no services (empty services list)
    app.init(vec![], false)?;

    // Verify config file was created
    assert!(setup.config_path().exists());

    // Verify no variables were configured
    assert_eq!(app.get_variables().len(), 0);

    // Step 2: Create worktree without any variables/ports
    app.add_worktree(
        "feature-no-variables".to_string(),
        None,  // from main branch
        None,  // no ports needed
        false, // not dry run
    )?;

    // Verify worktree was created
    assert!(setup.worktree_exists("feature-no-variables"));
    assert!(setup.env_file_exists("feature-no-variables"));

    // Verify env file content (should only have basic vibetree header, no port variables)
    let env_content = setup.read_env_file("feature-no-variables")?;
    assert!(env_content.contains("# Generated by vibetree"));
    // Should NOT contain any port variables
    assert!(!env_content.contains("_PORT="));

    // Verify git worktree was created
    let worktrees_output = setup.run_git_cmd(&["worktree", "list"])?;
    assert!(worktrees_output.contains("feature-no-variables"));

    // Step 3: Create second worktree (should also work fine)
    app.add_worktree(
        "another-feature".to_string(),
        Some("main".to_string()),
        None, // no custom ports
        false,
    )?;

    // Verify second worktree
    assert!(setup.worktree_exists("another-feature"));
    assert!(setup.env_file_exists("another-feature"));

    // Step 4: List worktrees should work without services
    app.list_worktrees(None)?; // Should not panic

    // Verify configuration state - now includes main branch due to sync
    assert_eq!(app.get_worktrees().len(), 3);
    assert!(app.get_worktrees().contains_key("feature-no-variables"));
    assert!(app.get_worktrees().contains_key("another-feature"));
    assert!(app.get_worktrees().contains_key("main"));

    // Both worktrees should have empty ports
    assert_eq!(app.get_worktrees()["feature-no-variables"].values.len(), 0);
    assert_eq!(app.get_worktrees()["another-feature"].values.len(), 0);

    // Step 5: Remove a worktree (should work same as with variables)
    app.remove_worktree_for_test(
        "another-feature".to_string(),
        false, // not forced
        false, // don't keep branch
    )?;

    // Verify worktree was removed
    assert!(!setup.worktree_exists("another-feature"));
    assert_eq!(app.get_worktrees().len(), 2); // main + feature-no-variables remain
    assert!(!app.get_worktrees().contains_key("another-feature"));

    Ok(())
}

#[test]
fn test_sync_orphaned_worktree_discovery() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize with variables
    app.init(vec!["postgres".to_string(), "redis".to_string()], false)?;

    // Create a worktree through vibetree normally
    app.add_worktree("normal-branch".to_string(), None, None, false)?;

    // Create an "orphaned" worktree directly with git (bypassing vibetree)
    let branches_dir = setup.repo_path.join(".vibetree").join("branches");
    std::fs::create_dir_all(&branches_dir)?;

    setup.run_git_cmd(&[
        "worktree",
        "add",
        "-b",
        "orphaned-branch",
        ".vibetree/branches/orphaned-branch",
        "main",
    ])?;

    // Verify orphaned worktree exists in git but not in config
    assert!(
        setup
            .repo_path
            .join(".vibetree")
            .join("branches")
            .join("orphaned-branch")
            .exists()
    );
    assert!(!app.get_worktrees().contains_key("orphaned-branch"));

    // Run sync - should discover orphaned worktree
    app.sync(false)?;

    // Verify orphaned worktree is now in config
    assert!(app.get_worktrees().contains_key("orphaned-branch"));

    // Verify it got ports allocated (should be different from normal-branch)
    let orphaned_ports = &app.get_worktrees()["orphaned-branch"].values;
    let normal_ports = &app.get_worktrees()["normal-branch"].values;

    assert_eq!(orphaned_ports.len(), 2); // postgres and redis ports
    assert_ne!(orphaned_ports["POSTGRES"], normal_ports["POSTGRES"]);
    assert_ne!(orphaned_ports["REDIS"], normal_ports["REDIS"]);

    // Verify env file was created
    let env_path = setup
        .repo_path
        .join(".vibetree")
        .join("branches")
        .join("orphaned-branch")
        .join(".vibetree")
        .join("env");
    assert!(env_path.exists());

    let env_content = std::fs::read_to_string(env_path)?;
    assert!(env_content.contains("POSTGRES="));
    assert!(env_content.contains("REDIS="));

    Ok(())
}

#[test]
fn test_sync_missing_worktree_cleanup() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize and create worktree
    app.init(vec!["postgres".to_string()], false)?;
    app.add_worktree("temp-branch".to_string(), None, None, false)?;

    // Verify worktree exists in both git and config
    assert!(setup.worktree_exists("temp-branch"));
    assert!(app.get_worktrees().contains_key("temp-branch"));

    // Manually remove the git worktree (simulating external deletion)
    setup.run_git_cmd(&[
        "worktree",
        "remove",
        "--force",
        ".vibetree/branches/temp-branch",
    ])?;

    // Verify git worktree is gone but still in config
    assert!(!setup.worktree_exists("temp-branch"));
    assert!(app.get_worktrees().contains_key("temp-branch"));

    // Run sync - should clean up missing worktree
    app.sync(false)?;

    // Verify worktree was removed from config
    assert!(!app.get_worktrees().contains_key("temp-branch"));

    Ok(())
}

#[test]
fn test_sync_config_variable_changes() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize with initial variables
    app.init(vec!["postgres".to_string()], false)?;
    app.add_worktree("test-branch".to_string(), None, None, false)?;

    // Verify initial state
    let initial_ports = app.get_worktrees()["test-branch"].values.clone();
    assert_eq!(initial_ports.len(), 1);
    assert!(initial_ports.contains_key("POSTGRES"));

    // Simulate config change by directly modifying the project config
    // (In real usage, user would edit vibetree.toml and we'd reload)
    app.get_config_mut()
        .project_config
        .variables
        .push(vibetree::VariableConfig {
            name: "REDIS".to_string(),
            default_value: 6379,
        });

    // Run sync - should detect variable mismatch and update
    app.sync(false)?;

    // Verify worktree now has both variables
    let updated_ports = &app.get_worktrees()["test-branch"].values;
    assert_eq!(updated_ports.len(), 2);
    assert!(updated_ports.contains_key("POSTGRES"));
    assert!(updated_ports.contains_key("REDIS"));

    // Verify env file was updated
    let env_content = setup.read_env_file("test-branch")?;
    assert!(env_content.contains("POSTGRES="));
    assert!(env_content.contains("REDIS="));

    Ok(())
}

#[test]
fn test_sync_main_worktree_handling() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize (this adds main branch to config)
    app.init(vec!["postgres".to_string()], false)?;

    // Verify main branch is in config with base ports
    assert!(app.get_worktrees().contains_key("main"));
    assert_eq!(app.get_worktrees()["main"].values["POSTGRES"], 8000);

    // Manually remove main from config (simulating corrupted state)
    app.get_config_mut()
        .branches_config
        .worktrees
        .remove("main");
    assert!(!app.get_worktrees().contains_key("main"));

    // Create another worktree that might get the base ports
    app.add_worktree("other-branch".to_string(), None, None, false)?;
    let other_postgres_port = app.get_worktrees()["other-branch"].values["POSTGRES"];

    // Run sync - should re-add main branch and handle port conflicts
    app.sync(false)?;

    // Verify main branch is back in config with base ports
    assert!(app.get_worktrees().contains_key("main"));
    assert_eq!(app.get_worktrees()["main"].values["POSTGRES"], 8000);

    // Verify other branch got reassigned ports if there was a conflict
    let other_new_port = app.get_worktrees()["other-branch"].values["POSTGRES"];
    if other_postgres_port == 8000 {
        // There was a conflict, other branch should have been reassigned
        assert_ne!(other_new_port, 8000);
    }

    // Verify main branch env file exists at repo root
    let main_env_path = setup.repo_path.join(".vibetree").join("env");
    assert!(main_env_path.exists());

    Ok(())
}

#[test]
fn test_sync_dry_run() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize and create a worktree
    app.init(vec!["postgres".to_string()], false)?;
    app.add_worktree("test-branch".to_string(), None, None, false)?;

    // Create orphaned worktree
    setup.run_git_cmd(&[
        "worktree",
        "add",
        "-b",
        "orphaned",
        ".vibetree/branches/orphaned",
        "main",
    ])?;

    // Remove main from config to test multiple sync operations
    app.get_config_mut()
        .branches_config
        .worktrees
        .remove("main");

    let initial_worktree_count = app.get_worktrees().len();

    // Run dry run sync
    app.sync(true)?;

    // Verify no changes were made
    assert_eq!(app.get_worktrees().len(), initial_worktree_count);
    assert!(!app.get_worktrees().contains_key("orphaned"));
    assert!(!app.get_worktrees().contains_key("main"));

    // Run actual sync
    app.sync(false)?;

    // Verify changes were made
    assert!(app.get_worktrees().contains_key("orphaned"));
    assert!(app.get_worktrees().contains_key("main"));

    Ok(())
}

#[test]
fn test_list_main_worktree_status() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize (adds main to config)
    app.init(vec!["postgres".to_string()], false)?;

    // Create a branch worktree
    app.add_worktree("test-branch".to_string(), None, None, false)?;

    // Use the public method to check list functionality
    let worktree_data = app.collect_worktree_data()?;

    // Find main and test-branch in the data
    let main_entry = worktree_data.iter().find(|w| w.name == "main");
    let branch_entry = worktree_data.iter().find(|w| w.name == "test-branch");

    assert!(main_entry.is_some(), "Main worktree should be listed");
    assert!(branch_entry.is_some(), "Branch worktree should be listed");

    // Main should show as OK (not Missing)
    assert_eq!(main_entry.unwrap().status, "OK");
    assert_eq!(branch_entry.unwrap().status, "OK");

    // Both should have port allocations
    assert!(!main_entry.unwrap().values.is_empty());
    assert!(!branch_entry.unwrap().values.is_empty());

    Ok(())
}
