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

```bash
# Initialize vibetree in a repository
vibetree init --variables web,api,db

# Or specify custom starting ports
vibetree init --variables web:3000,api:8080,db:5432

# Create a new work tree with allocated ports
vibetree create feature-branch

# Work trees are created in the branches/ directory by default
cd branches/feature-branch

# List work trees and their port assignments
vibetree list

# Remove a work tree
vibetree remove feature-branch
```

## License

Licensed under either of <a href="LICENSE-APACHE">Apache License, Version
2.0</a> or <a href="LICENSE-MIT">MIT license</a> at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
