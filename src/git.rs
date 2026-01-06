use anyhow::{Context, Result};
use git2::Repository;
use std::path::{Path, PathBuf};

pub struct GitManager;

impl GitManager {
    /// Check if the given path is the root of a git repository
    pub fn is_git_repo_root(path: &Path) -> bool {
        path.join(".git").exists()
    }

    /// Check if there are uncommitted changes in the repository
    pub fn has_uncommitted_changes(repo_path: &Path) -> Result<bool> {
        use std::process::Command;

        let output = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(repo_path)
            .output()
            .context("Failed to execute git status")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git status failed: {}", stderr);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(!stdout.trim().is_empty())
    }

    /// Check if a branch is an ancestor of another (i.e., already merged)
    pub fn is_ancestor(repo_path: &Path, branch: &str, target: &str) -> Result<bool> {
        use std::process::Command;

        let output = Command::new("git")
            .args(["merge-base", "--is-ancestor", branch, target])
            .current_dir(repo_path)
            .output()
            .context("Failed to execute git merge-base")?;

        // Exit code 0 means branch IS an ancestor of target
        // Exit code 1 means branch is NOT an ancestor
        // Other exit codes are errors
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("git merge-base failed: {}", stderr);
            }
        }
    }

    /// Test if a merge would succeed without conflicts
    pub fn can_merge_cleanly(repo_path: &Path, branch: &str, target: &str) -> Result<bool> {
        use std::process::Command;

        // First, checkout the target branch
        let checkout = Command::new("git")
            .args(["checkout", target])
            .current_dir(repo_path)
            .output()
            .context("Failed to checkout target branch")?;

        if !checkout.status.success() {
            let stderr = String::from_utf8_lossy(&checkout.stderr);
            anyhow::bail!("Failed to checkout {}: {}", target, stderr);
        }

        // Try a merge with --no-commit --no-ff to test
        let merge = Command::new("git")
            .args(["merge", "--no-commit", "--no-ff", branch])
            .current_dir(repo_path)
            .output()
            .context("Failed to execute test merge")?;

        let can_merge = merge.status.success();

        // Always abort/reset to clean up
        let _ = Command::new("git")
            .args(["merge", "--abort"])
            .current_dir(repo_path)
            .output();

        Ok(can_merge)
    }

    /// Test if a squash merge would succeed without conflicts
    pub fn can_squash_cleanly(repo_path: &Path, branch: &str, target: &str) -> Result<bool> {
        use std::process::Command;

        // First, checkout the target branch
        let checkout = Command::new("git")
            .args(["checkout", target])
            .current_dir(repo_path)
            .output()
            .context("Failed to checkout target branch")?;

        if !checkout.status.success() {
            let stderr = String::from_utf8_lossy(&checkout.stderr);
            anyhow::bail!("Failed to checkout {}: {}", target, stderr);
        }

        // Try a squash merge with --no-commit to test
        let merge = Command::new("git")
            .args(["merge", "--squash", "--no-commit", branch])
            .current_dir(repo_path)
            .output()
            .context("Failed to execute test squash merge")?;

        let can_merge = merge.status.success();

        // Reset to clean up (squash doesn't leave merge state, so use reset)
        let _ = Command::new("git")
            .args(["reset", "--hard", "HEAD"])
            .current_dir(repo_path)
            .output();

        Ok(can_merge)
    }

    /// Test if a rebase would succeed without conflicts
    ///
    /// For worktrees, the rebase must be run from the worktree directory since
    /// git doesn't allow checking out a branch that's already checked out elsewhere.
    pub fn can_rebase_cleanly(
        repo_path: &Path,
        branch: &str,
        target: &str,
        worktree_path: Option<&Path>,
    ) -> Result<bool> {
        use std::process::Command;

        // Determine where to run the rebase
        let rebase_dir = worktree_path.unwrap_or(repo_path);

        // For worktree case, the branch is already checked out there
        // For non-worktree case, we need to checkout first (but this shouldn't happen in practice)
        if worktree_path.is_none() {
            // Remember current branch to restore later
            let current_branch = Self::get_current_branch(repo_path)?;

            // Checkout the branch to rebase
            let checkout = Command::new("git")
                .args(["checkout", branch])
                .current_dir(repo_path)
                .output()
                .context("Failed to checkout branch for rebase test")?;

            if !checkout.status.success() {
                let stderr = String::from_utf8_lossy(&checkout.stderr);
                anyhow::bail!("Failed to checkout {}: {}", branch, stderr);
            }

            // Try the rebase
            let rebase = Command::new("git")
                .args(["rebase", target])
                .current_dir(repo_path)
                .output()
                .context("Failed to execute test rebase")?;

            let can_rebase = rebase.status.success();

            // Abort rebase if it failed
            if !can_rebase {
                let _ = Command::new("git")
                    .args(["rebase", "--abort"])
                    .current_dir(repo_path)
                    .output();
            }

            // If rebase succeeded, undo it
            if can_rebase {
                let _ = Command::new("git")
                    .args(["reset", "--hard", &format!("{}@{{1}}", branch)])
                    .current_dir(repo_path)
                    .output();
            }

            // Return to original branch
            let _ = Command::new("git")
                .args(["checkout", &current_branch])
                .current_dir(repo_path)
                .output();

            Ok(can_rebase)
        } else {
            // Worktree case - run rebase directly in the worktree
            let rebase = Command::new("git")
                .args(["rebase", target])
                .current_dir(rebase_dir)
                .output()
                .context("Failed to execute test rebase")?;

            let can_rebase = rebase.status.success();

            // Abort rebase if it failed
            if !can_rebase {
                let _ = Command::new("git")
                    .args(["rebase", "--abort"])
                    .current_dir(rebase_dir)
                    .output();
            }

            // If rebase succeeded, undo it using reflog
            if can_rebase {
                let _ = Command::new("git")
                    .args(["reset", "--hard", &format!("{}@{{1}}", branch)])
                    .current_dir(rebase_dir)
                    .output();
            }

            Ok(can_rebase)
        }
    }

    /// Execute a merge
    pub fn merge_branch(repo_path: &Path, branch: &str, target: &str) -> Result<()> {
        use std::process::Command;

        // Checkout target branch
        let checkout = Command::new("git")
            .args(["checkout", target])
            .current_dir(repo_path)
            .output()
            .context("Failed to checkout target branch")?;

        if !checkout.status.success() {
            let stderr = String::from_utf8_lossy(&checkout.stderr);
            anyhow::bail!("Failed to checkout {}: {}", target, stderr);
        }

        // Execute merge
        let merge = Command::new("git")
            .args(["merge", branch])
            .current_dir(repo_path)
            .output()
            .context("Failed to execute merge")?;

        if !merge.status.success() {
            let stderr = String::from_utf8_lossy(&merge.stderr);
            anyhow::bail!("Merge failed: {}", stderr);
        }

        Ok(())
    }

    /// Execute a squash merge with a custom commit message
    pub fn squash_merge_branch(
        repo_path: &Path,
        branch: &str,
        target: &str,
        message: &str,
    ) -> Result<()> {
        use std::process::Command;

        // Checkout target branch
        let checkout = Command::new("git")
            .args(["checkout", target])
            .current_dir(repo_path)
            .output()
            .context("Failed to checkout target branch")?;

        if !checkout.status.success() {
            let stderr = String::from_utf8_lossy(&checkout.stderr);
            anyhow::bail!("Failed to checkout {}: {}", target, stderr);
        }

        // Execute squash merge
        let merge = Command::new("git")
            .args(["merge", "--squash", branch])
            .current_dir(repo_path)
            .output()
            .context("Failed to execute squash merge")?;

        if !merge.status.success() {
            let stderr = String::from_utf8_lossy(&merge.stderr);
            anyhow::bail!("Squash merge failed: {}", stderr);
        }

        // Commit with the provided message
        let commit = Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(repo_path)
            .output()
            .context("Failed to commit squash merge")?;

        if !commit.status.success() {
            let stderr = String::from_utf8_lossy(&commit.stderr);
            anyhow::bail!("Failed to commit squash merge: {}", stderr);
        }

        Ok(())
    }

    /// Execute a rebase and fast-forward merge
    ///
    /// For worktrees, the rebase must be run from the worktree directory since
    /// git doesn't allow checking out a branch that's already checked out elsewhere.
    pub fn rebase_and_merge(
        repo_path: &Path,
        branch: &str,
        target: &str,
        worktree_path: Option<&Path>,
    ) -> Result<()> {
        use std::process::Command;

        // Determine where to run the rebase
        let rebase_dir = worktree_path.unwrap_or(repo_path);

        if worktree_path.is_some() {
            // Worktree case - run rebase directly in the worktree
            let rebase = Command::new("git")
                .args(["rebase", target])
                .current_dir(rebase_dir)
                .output()
                .context("Failed to execute rebase")?;

            if !rebase.status.success() {
                // Abort the failed rebase
                let _ = Command::new("git")
                    .args(["rebase", "--abort"])
                    .current_dir(rebase_dir)
                    .output();
                let stderr = String::from_utf8_lossy(&rebase.stderr);
                anyhow::bail!("Rebase failed: {}", stderr);
            }
        } else {
            // Non-worktree case - checkout first
            let checkout = Command::new("git")
                .args(["checkout", branch])
                .current_dir(repo_path)
                .output()
                .context("Failed to checkout branch for rebase")?;

            if !checkout.status.success() {
                let stderr = String::from_utf8_lossy(&checkout.stderr);
                anyhow::bail!("Failed to checkout {}: {}", branch, stderr);
            }

            // Execute rebase
            let rebase = Command::new("git")
                .args(["rebase", target])
                .current_dir(repo_path)
                .output()
                .context("Failed to execute rebase")?;

            if !rebase.status.success() {
                // Abort the failed rebase
                let _ = Command::new("git")
                    .args(["rebase", "--abort"])
                    .current_dir(repo_path)
                    .output();
                let stderr = String::from_utf8_lossy(&rebase.stderr);
                anyhow::bail!("Rebase failed: {}", stderr);
            }
        }

        // Checkout target (in main repo) and fast-forward merge
        let checkout_target = Command::new("git")
            .args(["checkout", target])
            .current_dir(repo_path)
            .output()
            .context("Failed to checkout target after rebase")?;

        if !checkout_target.status.success() {
            let stderr = String::from_utf8_lossy(&checkout_target.stderr);
            anyhow::bail!("Failed to checkout {}: {}", target, stderr);
        }

        // Fast-forward merge
        let merge = Command::new("git")
            .args(["merge", "--ff-only", branch])
            .current_dir(repo_path)
            .output()
            .context("Failed to execute fast-forward merge")?;

        if !merge.status.success() {
            let stderr = String::from_utf8_lossy(&merge.stderr);
            anyhow::bail!("Fast-forward merge failed: {}", stderr);
        }

        Ok(())
    }

    /// Check if a branch exists
    pub fn branch_exists(repo_path: &Path, branch: &str) -> Result<bool> {
        use std::process::Command;

        let output = Command::new("git")
            .args(["rev-parse", "--verify", &format!("refs/heads/{}", branch)])
            .current_dir(repo_path)
            .output()
            .context("Failed to check branch existence")?;

        Ok(output.status.success())
    }

    /// Check if vibetree is already configured (has vibetree.toml)
    pub fn is_vibetree_configured(repo_root: &Path) -> bool {
        repo_root.join("vibetree.toml").exists()
    }

    pub fn find_repo_root(start_path: &Path) -> Result<PathBuf> {
        // Use git2's discover to properly handle both regular repos and worktrees
        let repo = Repository::discover(start_path)
            .context("Not inside a git repository")?;

        // For worktrees, we need to find the main repository's working directory
        // Check if this is a worktree by looking at the commondir
        let git_dir = repo.path();

        // Read the commondir file which exists in worktrees and points to main repo's .git
        let commondir_path = git_dir.join("commondir");
        if commondir_path.exists() {
            // This is a worktree - read commondir to find main repo's .git directory
            let commondir_content = std::fs::read_to_string(&commondir_path)
                .context("Failed to read commondir file")?;
            let common_git_dir = git_dir.join(commondir_content.trim());

            // Canonicalize to resolve relative paths like ../..
            let canonical_common_git_dir = common_git_dir.canonicalize()
                .context("Failed to canonicalize common git directory")?;

            // The main repo's working directory is the parent of its .git directory
            canonical_common_git_dir.parent()
                .ok_or_else(|| anyhow::anyhow!("Cannot determine main repository root"))
                .map(|p| p.to_path_buf())
        } else {
            // Not a worktree - use the regular working directory
            repo.workdir()
                .ok_or_else(|| anyhow::anyhow!("Repository has no working directory (bare repo?)"))
                .map(|p| p.to_path_buf())
        }
    }

    pub fn get_current_branch(repo_path: &Path) -> Result<String> {
        let repo = Repository::open(repo_path)
            .with_context(|| format!("Failed to open git repository at {}", repo_path.display()))?;

        let head = repo.head().context("Failed to get HEAD reference")?;

        if let Some(branch_name) = head.shorthand() {
            Ok(branch_name.to_string())
        } else {
            anyhow::bail!("Unable to determine current branch name")
        }
    }

    pub fn create_worktree(
        repo_path: &Path,
        worktree_path: &Path,
        branch_name: &str,
        base_branch: Option<&str>,
    ) -> Result<()> {
        use std::process::Command;

        // Use git command line for worktree creation to avoid git2 reference conflicts
        let mut cmd = Command::new("git");
        cmd.args(["worktree", "add"]);

        // Add the worktree path
        cmd.arg(worktree_path);

        // Add branch creation arguments
        if let Some(base) = base_branch {
            // Create new branch from base
            cmd.args(["-b", branch_name, base]);
        } else {
            // Create new branch from HEAD
            cmd.args(["-b", branch_name]);
        }

        // Set working directory to the repo
        cmd.current_dir(repo_path);

        let output = cmd
            .output()
            .context("Failed to execute git worktree command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Git worktree creation failed: {}", stderr);
        }

        Ok(())
    }

    pub fn remove_worktree(repo_path: &Path, worktree_name: &str, keep_branch: bool) -> Result<()> {
        let repo = Repository::open(repo_path)
            .with_context(|| format!("Failed to open git repository at {}", repo_path.display()))?;

        let worktree = repo
            .find_worktree(worktree_name)
            .with_context(|| format!("Worktree '{}' not found", worktree_name))?;

        // Remove worktree directory if it exists
        let path = worktree.path();
        if path.exists() {
            std::fs::remove_dir_all(path).with_context(|| {
                format!("Failed to remove worktree directory: {}", path.display())
            })?;
        }

        // Prune the worktree from git
        worktree.prune(None).context("Failed to prune worktree")?;

        // Remove branch if requested
        if !keep_branch {
            if let Ok(mut branch) = repo.find_branch(worktree_name, git2::BranchType::Local) {
                branch
                    .delete()
                    .with_context(|| format!("Failed to delete branch: {}", worktree_name))?;
            }
        }

        Ok(())
    }

    /// Prune invalid worktrees from git configuration
    pub fn prune_worktrees(repo_path: &Path) -> Result<()> {
        use std::process::Command;

        let output = Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(repo_path)
            .output()
            .context("Failed to run git worktree prune")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git worktree prune failed: {}", stderr);
        }

        Ok(())
    }

    pub fn validate_worktree_state(worktree_path: &Path) -> Result<WorktreeValidation> {
        let mut validation = WorktreeValidation {
            exists: false,
            is_git_worktree: false,
            has_vibetree_dir: false,
            has_env_file: false,
            branch_name: None,
        };

        validation.exists = worktree_path.exists();
        if !validation.exists {
            return Ok(validation);
        }

        // Check if it's a git worktree
        let git_dir = worktree_path.join(".git");
        validation.is_git_worktree = git_dir.exists();

        if validation.is_git_worktree {
            validation.branch_name = Self::get_current_branch(worktree_path).ok();
        }

        // Check vibetree directory and env file
        let vibetree_dir = worktree_path.join(".vibetree");
        validation.has_vibetree_dir = vibetree_dir.exists();
        validation.has_env_file = vibetree_dir.join("env").exists();

        Ok(validation)
    }

    /// Discover all git worktrees in the repository
    pub fn discover_worktrees(repo_path: &Path) -> Result<Vec<DiscoveredWorktree>> {
        use std::process::Command;

        let output = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(repo_path)
            .output()
            .context("Failed to execute git worktree list")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Git worktree list failed: {}", stderr);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut worktrees = Vec::new();
        let mut current_worktree: Option<DiscoveredWorktree> = None;

        for line in stdout.lines() {
            if line.starts_with("worktree ") {
                // Save previous worktree if exists
                if let Some(wt) = current_worktree.take() {
                    worktrees.push(wt);
                }

                let path = line.strip_prefix("worktree ").unwrap_or("");
                current_worktree = Some(DiscoveredWorktree {
                    path: PathBuf::from(path),
                    branch: None,
                    is_bare: false,
                    is_detached: false,
                });
            } else if line.starts_with("branch ") {
                if let Some(ref mut wt) = current_worktree {
                    let branch = line.strip_prefix("branch ").unwrap_or("");
                    // Strip refs/heads/ prefix if present to get just the branch name
                    let branch_name = branch.strip_prefix("refs/heads/").unwrap_or(branch);
                    wt.branch = Some(branch_name.to_string());
                }
            } else if line == "bare" {
                if let Some(ref mut wt) = current_worktree {
                    wt.is_bare = true;
                }
            } else if line == "detached" {
                if let Some(ref mut wt) = current_worktree {
                    wt.is_detached = true;
                }
            }
        }

        // Don't forget the last worktree
        if let Some(wt) = current_worktree {
            worktrees.push(wt);
        }

        Ok(worktrees)
    }
}

#[derive(Debug)]
pub struct WorktreeValidation {
    pub exists: bool,
    pub is_git_worktree: bool,
    pub has_vibetree_dir: bool,
    pub has_env_file: bool,
    pub branch_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DiscoveredWorktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub is_bare: bool,
    pub is_detached: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Repository;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_repo() -> Result<(TempDir, PathBuf)> {
        let temp_dir = TempDir::new()?;
        let repo_path = temp_dir.path().to_path_buf();

        let repo = Repository::init(&repo_path)?;

        // Create initial commit
        let signature = git2::Signature::now("Test User", "test@example.com")?;
        let tree_id = {
            let mut index = repo.index()?;
            // Create a simple file
            fs::write(repo_path.join("README.md"), "# Test Repo")?;
            index.add_path(Path::new("README.md"))?;
            index.write()?;
            index.write_tree()?
        };

        let tree = repo.find_tree(tree_id)?;
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "Initial commit",
            &tree,
            &[],
        )?;

        Ok((temp_dir, repo_path))
    }

    #[test]
    fn test_find_repo_root() -> Result<()> {
        let (_temp_dir, repo_path) = create_test_repo()?;

        // Create a subdirectory
        let sub_dir = repo_path.join("subdir");
        fs::create_dir(&sub_dir)?;

        let found_root = GitManager::find_repo_root(&sub_dir)?;
        // Canonicalize both paths for comparison to handle symlinks (e.g., /var vs /private/var on macOS)
        assert_eq!(found_root.canonicalize()?, repo_path.canonicalize()?);

        Ok(())
    }

    #[test]
    fn test_get_current_branch() -> Result<()> {
        let (_temp_dir, repo_path) = create_test_repo()?;

        let branch = GitManager::get_current_branch(&repo_path)?;
        // git init can default to either 'main' or 'master' depending on git version and config
        assert!(branch == "main" || branch == "master");

        Ok(())
    }

    #[test]
    fn test_validate_worktree_state() -> Result<()> {
        let (_temp_dir, repo_path) = create_test_repo()?;

        let validation = GitManager::validate_worktree_state(&repo_path)?;
        assert!(validation.exists);
        assert!(validation.is_git_worktree);
        assert!(!validation.has_vibetree_dir);
        assert!(!validation.has_env_file);

        // Create .vibetree directory and env file
        let vibetree_dir = repo_path.join(".vibetree");
        fs::create_dir(&vibetree_dir)?;
        fs::write(vibetree_dir.join("env"), "PGPORT=5432")?;

        let validation = GitManager::validate_worktree_state(&repo_path)?;
        assert!(validation.has_vibetree_dir);
        assert!(validation.has_env_file);

        Ok(())
    }

    #[test]
    fn test_find_repo_root_not_in_git() {
        let temp_dir = TempDir::new().unwrap();
        let result = GitManager::find_repo_root(temp_dir.path());
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Not inside a git repository")
        );
    }
}
