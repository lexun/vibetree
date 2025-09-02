# List all available commands
default:
    just --list --unsorted

# Run the vibetree binary in dev mode
vibetree *args:
    cargo run -- {{args}}
