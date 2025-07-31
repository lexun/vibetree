use anyhow::{Context, Result};
use git2::Repository;
use std::path::{Path, PathBuf};

pub struct GitManager;

impl GitManager {
    /// Check if the given path is the root of a git repository
    pub fn is_git_repo_root(path: &Path) -> bool {
        path.join(".git").exists()
    }

    /// Check if vibetree is already configured (has vibetree.toml)
    pub fn is_vibetree_configured(repo_root: &Path) -> bool {
        repo_root.join("vibetree.toml").exists()
    }

    pub fn find_repo_root(start_path: &Path) -> Result<PathBuf> {
        // Search up the directory tree to find the git repository root
        let mut current = start_path.to_path_buf();
        loop {
            if current.join(".git").exists() {
                return Ok(current);
            }

            match current.parent() {
                Some(parent) => current = parent.to_path_buf(),
                None => anyhow::bail!("Not inside a git repository"),
            }
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
}

#[derive(Debug)]
pub struct WorktreeValidation {
    pub exists: bool,
    pub is_git_worktree: bool,
    pub has_vibetree_dir: bool,
    pub has_env_file: bool,
    pub branch_name: Option<String>,
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
        assert_eq!(found_root, repo_path);

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
