# List all available commands
default:
    just --list --unsorted

# Run the vibetree binary in dev mode
vibetree *args:
    cargo run -- {{args}}

# Bump version, commit, and tag for release
tag:
    #!/usr/bin/env bash
    set -euo pipefail
    current=$(cargo pkgid | cut -d'#' -f2)
    echo "Current version: $current"
    read -p "New version: " version
    cargo set-version "$version"
    cargo check --quiet
    git add Cargo.toml Cargo.lock
    git commit -m "Bump version to $version"
    git tag -m "v$version" "v$version"
    echo "Tagged v$version. Push with: git push && git push --tags"
