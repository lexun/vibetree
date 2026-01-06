use anyhow::Result;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Once;
use tempfile::TempDir;
use vibetree::{config, EnvFileGenerator, GitManager, VariableConfig, VibeTreeApp};

// Set up test environment once - skip shell spawning in tests
static INIT: Once = Once::new();

// Counter for generating unique port ranges per test
// Starts at 55000 and increments by 100 for each test to avoid port conflicts
// Using high ports (55000+) to minimize collision with system services
static PORT_COUNTER: AtomicU16 = AtomicU16::new(55000);

fn setup_test_env() {
    INIT.call_once(|| {
        // SAFETY: This is called once at test setup before any threads are spawned,
        // so there are no concurrent reads of these environment variables.
        unsafe {
            // Skip spawning interactive shells in tests
            std::env::set_var("VIBETREE_SKIP_SHELL", "1");
            // Skip system port availability checks in tests (concurrent tests may
            // temporarily occupy ports, causing false "port in use" errors)
            std::env::set_var("VIBETREE_TESTING", "1");
        }
    });
}

/// Get a unique port base for this test (increments by 100 each call)
/// Each test gets a range of 100 ports starting from this base
fn get_unique_port_base() -> u16 {
    PORT_COUNTER.fetch_add(100, Ordering::SeqCst)
}

/// Helper to set up a complete test environment with git repo and vibetree
struct IntegrationTestSetup {
    #[allow(dead_code)] // Needed to keep the temp directory alive
    temp_dir: TempDir,
    repo_path: PathBuf,
    /// Unique port base for this test (each test gets 100 ports starting here)
    port_base: u16,
}

impl IntegrationTestSetup {
    /// Create a new integration test setup with:
    /// - Temporary directory containing the git repository
    /// - Git repository is the root directory (no subdirectory)
    /// - All worktrees will be in .vibetree/branches/ subdirectory
    /// - Initial commit to make it usable
    fn new() -> Result<Self> {
        // Ensure test environment is set up (skip shell spawning)
        setup_test_env();

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
            port_base: get_unique_port_base(),
        })
    }

    /// Create a VibeTreeApp instance for this test setup
    fn create_app(&self) -> Result<VibeTreeApp> {
        // Use with_parent to avoid global environment variable conflicts
        VibeTreeApp::with_parent(self.repo_path.clone())
    }

    /// Get a variable spec with a unique port for this test
    /// e.g., "POSTGRES_PORT:50100" instead of "postgres" which defaults to 5432
    fn var(&self, name: &str, offset: u16) -> String {
        format!("{}:{}", name.to_uppercase(), self.port_base + offset)
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

    // Step 1: Initialize vibetree with custom variables (using unique ports)
    let variables = vec![
        setup.var("postgres", 0),
        setup.var("redis", 1),
        setup.var("api", 2),
    ];
    app.init(variables.clone())?;

    // Verify config file was created
    assert!(setup.config_path().exists());

    // Verify variables were configured
    assert_eq!(app.get_variables().len(), 3);
    let expected_names = ["POSTGRES", "REDIS", "API"];
    for name in expected_names {
        assert!(
            app.get_variables().iter().any(|v| v.name == name),
            "Variable {} should exist",
            name
        );
    }

    // Step 2: Create first worktree
    app.add_worktree(
        "feature-auth".to_string(),
        None,  // from main branch
        None,  // auto-allocate ports
        false, // not dry run
        false, // don't switch
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
    let custom_values = vec!["5555".to_string(), "6666".to_string(), "7777".to_string()];
    app.add_worktree(
        "feature-payments".to_string(),
        Some("main".to_string()), // explicitly from main
        Some(custom_values.clone()),
        false, // not dry run
        false, // don't switch
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
        true,  // dry run
        false, // don't switch
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

    // Initialize with unique ports for this test
    app.init(vec![setup.var("postgres", 0), setup.var("redis", 1)])?;

    // Create first worktree
    app.add_worktree("branch1".to_string(), None, None, false, false)?;

    // Try to create second worktree with conflicting ports
    let first_worktree = &app.get_worktrees()["branch1"];
    let conflicting_ports: Vec<String> = first_worktree.values.values().cloned().collect();

    // This should fail due to port conflicts
    let result = app.add_worktree(
        "branch2".to_string(),
        None,
        Some(conflicting_ports),
        false,
        false,
    );

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

// Note: We no longer test system port unavailability detection directly since:
// 1. Each test uses unique high ports (50000+) via get_unique_port_base()
// 2. Port collision between tests is avoided by design
// 3. The port checking code in lib.rs is still active and will catch real conflicts
// The previous test was inherently flaky due to race conditions with system services

#[test]
fn test_worktree_validation() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize and create worktree
    app.init(vec![setup.var("postgres", 0)])?;
    app.add_worktree("test-branch".to_string(), None, None, false, false)?;

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

    app.init(vec![setup.var("postgres", 0)])?;

    // Test creating worktree with empty name
    let result = app.add_worktree("".to_string(), None, None, false, false);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("empty"));

    // Test creating duplicate worktree
    app.add_worktree("test".to_string(), None, None, false, false)?;
    let result = app.add_worktree("test".to_string(), None, None, false, false);
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
        Some(vec!["5432".to_string()]), // Only 1 port for 1 service is correct
        false,                          // not dry run
        false,                          // don't switch
    );
    // This should succeed since we have 1 port for 1 service
    assert!(result.is_ok());

    // Now test with wrong number of ports
    let result = app.add_worktree(
        "test3".to_string(),
        None,
        Some(vec!["5432".to_string(), "6379".to_string()]), // 2 ports for 1 service should fail
        false, // not dry run
        false, // don't switch
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
        app1.init(vec![setup.var("postgres", 0), setup.var("redis", 1)])?;
        app1.add_worktree("persistent-test".to_string(), None, None, false, false)?;

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
    app.init(vec![])?;

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
        false, // don't switch
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
        None,  // no custom ports
        false, // not dry run
        false, // don't switch
    )?;

    // Verify second worktree
    assert!(setup.worktree_exists("another-feature"));
    assert!(setup.env_file_exists("another-feature"));

    // Step 4: List worktrees should work without services
    app.list_worktrees(None)?; // Should not panic

    // Verify configuration state - now includes main branch due to repair
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
fn test_repair_orphaned_worktree_discovery() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize with unique ports
    app.init(vec![setup.var("postgres", 0), setup.var("redis", 1)])?;

    // Create a worktree through vibetree normally
    app.add_worktree("normal-branch".to_string(), None, None, false, false)?;

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

    // Run repair - should discover orphaned worktree
    app.repair(false)?;

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
fn test_repair_missing_worktree_cleanup() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize with unique port
    app.init(vec![setup.var("postgres", 0)])?;
    app.add_worktree("temp-branch".to_string(), None, None, false, false)?;

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

    // Run repair - should clean up missing worktree
    app.repair(false)?;

    // Verify worktree was removed from config
    assert!(!app.get_worktrees().contains_key("temp-branch"));

    Ok(())
}

#[test]
fn test_repair_config_variable_changes() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize with initial variables
    app.init(vec![setup.var("postgres", 0)])?;
    app.add_worktree("test-branch".to_string(), None, None, false, false)?;

    // Verify initial state
    let initial_ports = app.get_worktrees()["test-branch"].values.clone();
    assert_eq!(initial_ports.len(), 1);
    assert!(initial_ports.contains_key("POSTGRES"));

    // Simulate config change by directly modifying the project config
    // (In real usage, user would edit vibetree.toml and we'd reload)
    app.get_config_mut()
        .project_config
        .variables
        .push(VariableConfig {
            name: "REDIS".to_string(),
            value: Some(toml::Value::Integer(6379)),
            r#type: Some(config::VariableType::Port),
            branch: None,
        });

    // Run repair - should detect variable mismatch and update
    app.repair(false)?;

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
fn test_repair_main_worktree_handling() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize (this adds main branch to config)
    app.init(vec![setup.var("postgres", 0)])?;

    // Verify main branch is in config with allocated ports
    assert!(app.get_worktrees().contains_key("main"));
    let initial_main_port = app.get_worktrees()["main"].values["POSTGRES"].clone();
    assert!(!initial_main_port.is_empty());

    // Manually remove main from config (simulating corrupted state)
    app.get_config_mut()
        .branches_config
        .worktrees
        .remove("main");
    assert!(!app.get_worktrees().contains_key("main"));

    // Create another worktree
    app.add_worktree("other-branch".to_string(), None, None, false, false)?;

    // Run repair - should re-add main branch
    app.repair(false)?;

    // Verify main branch is back in config with some port allocated
    assert!(app.get_worktrees().contains_key("main"));
    let repaired_main_port = &app.get_worktrees()["main"].values["POSTGRES"];
    assert!(!repaired_main_port.is_empty());

    // Verify both worktrees have different ports (no conflicts)
    let other_new_port = &app.get_worktrees()["other-branch"].values["POSTGRES"];
    assert_ne!(repaired_main_port, other_new_port);

    // Verify main branch env file exists at repo root
    let main_env_path = setup.repo_path.join(".vibetree").join("env");
    assert!(main_env_path.exists());

    Ok(())
}

#[test]
fn test_repair_dry_run() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize and create a worktree
    app.init(vec![setup.var("postgres", 0)])?;
    app.add_worktree("test-branch".to_string(), None, None, false, false)?;

    // Create orphaned worktree
    setup.run_git_cmd(&[
        "worktree",
        "add",
        "-b",
        "orphaned",
        ".vibetree/branches/orphaned",
        "main",
    ])?;

    // Remove main from config to test multiple repair operations
    app.get_config_mut()
        .branches_config
        .worktrees
        .remove("main");

    let initial_worktree_count = app.get_worktrees().len();

    // Run dry run repair
    app.repair(true)?;

    // Verify no changes were made
    assert_eq!(app.get_worktrees().len(), initial_worktree_count);
    assert!(!app.get_worktrees().contains_key("orphaned"));
    assert!(!app.get_worktrees().contains_key("main"));

    // Run actual repair
    app.repair(false)?;

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
    app.init(vec![setup.var("postgres", 0)])?;

    // Create a branch worktree
    app.add_worktree("test-branch".to_string(), None, None, false, false)?;

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

#[test]
fn test_switch_to_existing_worktree() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize and create a worktree
    app.init(vec![setup.var("web", 0)])?;
    app.add_worktree("feature-branch".to_string(), None, None, false, false)?;

    // Test switching to the worktree should succeed
    let result = app.switch_to_worktree("feature-branch".to_string());
    assert!(result.is_ok());

    Ok(())
}

#[test]
fn test_switch_to_nonexistent_worktree() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize without creating any worktrees
    app.init(vec![setup.var("web", 0)])?;

    // Test switching to non-existent worktree should fail
    let result = app.switch_to_worktree("nonexistent-branch".to_string());
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("does not exist"));

    Ok(())
}

#[test]
fn test_add_worktree_with_switch_flag() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize and create a worktree with switch flag
    app.init(vec![setup.var("web", 0)])?;

    // Test adding worktree with switch=true should succeed
    let result = app.add_worktree("feature-branch".to_string(), None, None, false, true);
    assert!(result.is_ok());

    Ok(())
}

// ============================================================================
// Merge command tests
// ============================================================================

#[test]
fn test_merge_happy_path() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize and create worktree
    app.init(vec![setup.var("postgres", 0)])?;

    // Commit vibetree config (required before merge checks)
    setup.run_git_cmd(&["add", "vibetree.toml", ".gitignore"])?;
    setup.run_git_cmd(&["commit", "-m", "Add vibetree config"])?;

    app.add_worktree("feature-branch".to_string(), None, None, false, false)?;

    // Make a commit in the feature branch
    let worktree_path = setup
        .repo_path
        .join(".vibetree")
        .join("branches")
        .join("feature-branch");
    fs::write(worktree_path.join("feature.txt"), "new feature")?;
    Command::new("git")
        .args(["add", "."])
        .current_dir(&worktree_path)
        .output()?;
    Command::new("git")
        .args(["commit", "-m", "Add feature"])
        .current_dir(&worktree_path)
        .output()?;

    // Merge the feature branch (without --remove)
    let result = app.merge_worktree(
        "feature-branch".to_string(),
        None,   // into main
        false,  // not squash
        false,  // not rebase
        false,  // don't remove after
    );
    assert!(result.is_ok());

    // Verify the merge happened (feature.txt should now be in main)
    assert!(setup.repo_path.join("feature.txt").exists());

    // Verify worktree still exists (since we didn't use --remove)
    assert!(setup.worktree_exists("feature-branch"));

    Ok(())
}

#[test]
fn test_merge_with_remove_flag() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize and create worktree
    app.init(vec![setup.var("postgres", 0)])?;

    // Commit vibetree config (required before merge checks)
    setup.run_git_cmd(&["add", "vibetree.toml", ".gitignore"])?;
    setup.run_git_cmd(&["commit", "-m", "Add vibetree config"])?;

    app.add_worktree("temp-feature".to_string(), None, None, false, false)?;

    // Make a commit in the feature branch
    let worktree_path = setup
        .repo_path
        .join(".vibetree")
        .join("branches")
        .join("temp-feature");
    fs::write(worktree_path.join("temp.txt"), "temp feature")?;
    Command::new("git")
        .args(["add", "."])
        .current_dir(&worktree_path)
        .output()?;
    Command::new("git")
        .args(["commit", "-m", "Add temp feature"])
        .current_dir(&worktree_path)
        .output()?;

    // Merge with --remove flag
    let result = app.merge_worktree(
        "temp-feature".to_string(),
        None,  // into main
        false, // not squash
        false, // not rebase
        true,  // remove after
    );
    assert!(result.is_ok());

    // Verify the merge happened
    assert!(setup.repo_path.join("temp.txt").exists());

    // Verify worktree was removed
    assert!(!setup.worktree_exists("temp-feature"));
    assert!(!app.get_worktrees().contains_key("temp-feature"));

    Ok(())
}

#[test]
fn test_merge_already_merged() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize and create worktree
    app.init(vec![setup.var("postgres", 0)])?;

    // Commit vibetree config (required before merge checks)
    setup.run_git_cmd(&["add", "vibetree.toml", ".gitignore"])?;
    setup.run_git_cmd(&["commit", "-m", "Add vibetree config"])?;

    app.add_worktree("already-merged".to_string(), None, None, false, false)?;

    // Make a commit in the feature branch
    let worktree_path = setup
        .repo_path
        .join(".vibetree")
        .join("branches")
        .join("already-merged");
    fs::write(worktree_path.join("merged.txt"), "merged content")?;
    Command::new("git")
        .args(["add", "."])
        .current_dir(&worktree_path)
        .output()?;
    Command::new("git")
        .args(["commit", "-m", "Add merged content"])
        .current_dir(&worktree_path)
        .output()?;

    // First merge
    app.merge_worktree(
        "already-merged".to_string(),
        None,
        false,
        false,
        false,
    )?;

    // Second merge should detect already-merged and succeed
    let result = app.merge_worktree(
        "already-merged".to_string(),
        None,
        false,
        false,
        false,
    );
    assert!(result.is_ok());

    Ok(())
}

#[test]
fn test_merge_rebase_happy_path() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize and create worktree
    app.init(vec![setup.var("postgres", 0)])?;

    // Commit vibetree config (required before merge checks)
    setup.run_git_cmd(&["add", "vibetree.toml", ".gitignore"])?;
    setup.run_git_cmd(&["commit", "-m", "Add vibetree config"])?;

    app.add_worktree("rebase-feature".to_string(), None, None, false, false)?;

    // Make a commit in the feature branch
    let worktree_path = setup
        .repo_path
        .join(".vibetree")
        .join("branches")
        .join("rebase-feature");
    fs::write(worktree_path.join("rebase.txt"), "rebase content")?;
    Command::new("git")
        .args(["add", "."])
        .current_dir(&worktree_path)
        .output()?;
    Command::new("git")
        .args(["commit", "-m", "Add rebase content"])
        .current_dir(&worktree_path)
        .output()?;

    // Merge with rebase
    let result = app.merge_worktree(
        "rebase-feature".to_string(),
        None,  // into main
        false, // not squash
        true,  // rebase
        false, // don't remove
    );
    assert!(result.is_ok(), "Rebase merge failed: {:?}", result);

    // Verify the merge happened
    assert!(setup.repo_path.join("rebase.txt").exists());

    Ok(())
}

#[test]
fn test_merge_cannot_merge_main() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    app.init(vec![setup.var("postgres", 0)])?;

    // Try to merge main into itself - should fail
    let result = app.merge_worktree(
        "main".to_string(),
        None,  // defaults to main
        false,
        false,
        false,
    );
    assert!(result.is_err());
    // When merging main into main (default target), we get "into itself" error
    assert!(result.unwrap_err().to_string().contains("into itself"));

    // Also test merging main into a different target - should fail with "main branch" error
    let mut app2 = setup.create_app()?;
    let result2 = app2.merge_worktree(
        "main".to_string(),
        Some("nonexistent".to_string()),  // different target
        false,
        false,
        false,
    );
    assert!(result2.is_err());
    // This should fail because we can't merge the main branch
    assert!(result2.unwrap_err().to_string().contains("Cannot merge the main branch"));

    Ok(())
}

#[test]
fn test_merge_nonexistent_branch() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    app.init(vec![setup.var("postgres", 0)])?;

    // Try to merge non-existent branch - should fail
    let result = app.merge_worktree(
        "nonexistent".to_string(),
        None,
        false,
        false,
        false,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("does not exist"));

    Ok(())
}

#[test]
fn test_merge_detects_uncommitted_changes() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize and create worktree
    app.init(vec![setup.var("postgres", 0)])?;

    // Commit vibetree config first (so main is clean)
    setup.run_git_cmd(&["add", "vibetree.toml", ".gitignore"])?;
    setup.run_git_cmd(&["commit", "-m", "Add vibetree config"])?;

    app.add_worktree("dirty-feature".to_string(), None, None, false, false)?;

    // Make a committed change in the feature branch first (so there's something to merge)
    let worktree_path = setup
        .repo_path
        .join(".vibetree")
        .join("branches")
        .join("dirty-feature");
    fs::write(worktree_path.join("committed.txt"), "committed change")?;
    Command::new("git")
        .args(["add", "."])
        .current_dir(&worktree_path)
        .output()?;
    Command::new("git")
        .args(["commit", "-m", "Add committed file"])
        .current_dir(&worktree_path)
        .output()?;

    // Now add uncommitted changes
    fs::write(worktree_path.join("uncommitted.txt"), "uncommitted")?;

    // Try to merge - should fail due to uncommitted changes
    let result = app.merge_worktree(
        "dirty-feature".to_string(),
        None,
        false,
        false,
        false,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Uncommitted changes"));

    Ok(())
}

#[test]
fn test_merge_detects_conflicts() -> Result<()> {
    let setup = IntegrationTestSetup::new()?;
    let mut app = setup.create_app()?;

    // Initialize and create worktree
    app.init(vec![setup.var("postgres", 0)])?;
    app.add_worktree("conflict-feature".to_string(), None, None, false, false)?;

    // Create conflicting changes in main
    fs::write(setup.repo_path.join("conflict.txt"), "main version")?;
    Command::new("git")
        .args(["add", "."])
        .current_dir(&setup.repo_path)
        .output()?;
    Command::new("git")
        .args(["commit", "-m", "Main conflict"])
        .current_dir(&setup.repo_path)
        .output()?;

    // Create conflicting changes in feature branch
    let worktree_path = setup
        .repo_path
        .join(".vibetree")
        .join("branches")
        .join("conflict-feature");
    fs::write(worktree_path.join("conflict.txt"), "feature version")?;
    Command::new("git")
        .args(["add", "."])
        .current_dir(&worktree_path)
        .output()?;
    Command::new("git")
        .args(["commit", "-m", "Feature conflict"])
        .current_dir(&worktree_path)
        .output()?;

    // Try to merge - should fail due to conflicts
    let result = app.merge_worktree(
        "conflict-feature".to_string(),
        None,
        false,
        false,
        false,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("conflicts"));

    Ok(())
}
