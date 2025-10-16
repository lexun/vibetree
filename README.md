# Vibetree

A work tree management tool that assigns unique ports to each work tree's dependent services.

## Overview

Vibetree manages Git work trees while automatically allocating distinct ports for each work tree's services. This enables:

- Isolated development environments per work tree
- Parallel test execution without port conflicts
- Multiple AI agents working on different branches simultaneously

## Status

⚠️ **Disclaimer**: This tool has been primarily developed by AI, with some human oversight.

⚠️ **Work in Progress** - Not ready for production use.

## Usage

### Commands

#### Initialize
```bash
# Initialize vibetree in a repository
vibetree init --variables web,api,db

# Convert existing git repo to vibetree-managed structure
vibetree init --variables web,api,db --convert-repo
```

#### Add Worktrees
```bash
# Add a new worktree
vibetree add feature-branch

# Add worktree from a specific branch
vibetree add feature-branch --from main

# Add worktree with custom port assignments
vibetree add feature-branch --ports 3000,8080,5432

# Add worktree and switch to it immediately
vibetree add feature-branch --switch

# Preview what would be added without making changes
vibetree add feature-branch --dry-run
```

#### List Worktrees
```bash
# List all worktrees with their port allocations
vibetree list

# List in different formats
vibetree list --format table
vibetree list --format json
vibetree list --format yaml
```

#### Remove Worktrees
```bash
# Remove a worktree
vibetree remove feature-branch

# Force removal even if processes are running on allocated ports
vibetree remove feature-branch --force

# Remove worktree but keep the git branch
vibetree remove feature-branch --keep-branch
```

#### Switch Between Worktrees
```bash
# Switch to an existing worktree directory
vibetree switch feature-branch
```

#### Repair Configuration
```bash
# Repair configuration and discover orphaned worktrees
vibetree repair

# Preview what would be repaired without making changes
vibetree repair --dry-run
```

#### Global Options
```bash
# Enable verbose output for any command
vibetree --verbose <command>

# Show help for any command
vibetree <command> --help

# Show version
vibetree --version
```

## License

Licensed under either of <a href="LICENSE-APACHE">Apache License, Version
2.0</a> or <a href="LICENSE-MIT">MIT license</a> at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
