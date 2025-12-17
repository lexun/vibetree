# Vibetree

A worktree management tool that assigns unique environment values to each worktree's dependent services.

## Overview

Vibetree manages Git worktrees while automatically allocating distinct values (ports, IPs, instance IDs, etc.) for each worktree's services. This enables:

- Isolated development environments per worktree
- Parallel test execution without port conflicts
- Multiple AI agents working on different branches simultaneously

## Status

🌱 **Early Development** - Usable but APIs may change. Feedback welcome!

## Installation

### With Nix

Requires [Nix](https://github.com/DeterminateSystems/nix-installer).

```bash
nix profile add github:lexun/vibetree
```

### With Cargo

Requires [Cargo](https://doc.rust-lang.org/cargo/getting-started/installation.html).

```bash
cargo install --git https://github.com/lexun/vibetree
```

## Configuration

Vibetree generates a `.vibetree/env` file for each worktree with unique values based on your `vibetree.toml` config. You can source it directly, symlink it to `.env` for docker-compose, or use direnv to load it automatically.

Variables can be:

- **Static** - same value for all worktrees
- **Auto-allocated ports** - finds next available port from a base value
- **Auto-incrementing integers** - simple counter-based allocation
- **String templates** - with embedded `{port:N}` or `{int:N}` components

### Example: Unique Ports per Worktree

```toml
[[variables]]
name = "POSTGRES_PORT"
value = 5432
type = "port"

[[variables]]
name = "REDIS_PORT"
value = 6379
type = "port"
```

Each worktree gets the next available port starting from the base value. On `main`, `POSTGRES_PORT` might be 5432; `feature-1` gets 5433, etc.

### Example: Unique Container Names and Volumes

Use string templates to generate unique Docker resource names per worktree:

```toml
[[variables]]
name = "COMPOSE_PROJECT_NAME"
value = "myapp_{int:1}"  # myapp_1, myapp_2, ...

[[variables]]
name = "POSTGRES_VOLUME"
value = "pgdata_{int:1}"

[[variables]]
name = "POSTGRES_PORT"
value = 5432
type = "port"
```

Each worktree gets isolated Docker resources:

- main: `COMPOSE_PROJECT_NAME=myapp_1`, `POSTGRES_VOLUME=pgdata_1`
- feature-1: `COMPOSE_PROJECT_NAME=myapp_2`, `POSTGRES_VOLUME=pgdata_2`

Then in your `docker-compose.yml`:

```yaml
volumes:
  ${POSTGRES_VOLUME}:

services:
  db:
    image: postgres
    ports:
      - "${POSTGRES_PORT}:5432"
    volumes:
      - ${POSTGRES_VOLUME}:/var/lib/postgresql/data
```

## Usage

### Commands

#### Initialize

```bash
# Initialize vibetree in a repository
vibetree init

# Initialize with predefined service variables
vibetree init --variables POSTGRES_PORT,REDIS_PORT,API_PORT
```

This creates a `vibetree.toml` config file. Edit it to define your variables.

#### Add Worktrees

```bash
# Add a new worktree (auto-allocates values)
vibetree add feature-branch

# Add worktree from a specific branch
vibetree add feature-branch --from main

# Add worktree with custom value assignments (positional, matches variable order)
vibetree add feature-branch --ports 5440,6380

# Add worktree and switch to it immediately
vibetree add feature-branch --switch

# Preview what would be added without making changes
vibetree add feature-branch --dry-run
```

#### List Worktrees

```bash
# List all worktrees with their allocated values
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
