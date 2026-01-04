use carapace_spec_clap::Spec;
use clap::CommandFactory;
use clap_complete::{generate, Shell};

use crate::{Cli, CompletionShell};

pub fn generate_completions(shell: CompletionShell) {
    let mut cmd = Cli::command();
    match shell {
        CompletionShell::Bash => generate(Shell::Bash, &mut cmd, "vibetree", &mut std::io::stdout()),
        CompletionShell::Elvish => {
            generate(Shell::Elvish, &mut cmd, "vibetree", &mut std::io::stdout())
        }
        CompletionShell::Fish => generate(Shell::Fish, &mut cmd, "vibetree", &mut std::io::stdout()),
        CompletionShell::Powershell => {
            generate(Shell::PowerShell, &mut cmd, "vibetree", &mut std::io::stdout())
        }
        CompletionShell::Zsh => generate(Shell::Zsh, &mut cmd, "vibetree", &mut std::io::stdout()),
        CompletionShell::Carapace => generate_carapace_spec(&mut cmd),
        CompletionShell::Install => install_completions(&mut cmd),
    }
}

fn install_completions(cmd: &mut clap::Command) {
    if which_carapace().is_some() {
        install_carapace_spec(cmd);
        return;
    }

    // Fall back to native shell completions
    let shell = detect_shell();
    match shell {
        Some(name) => {
            println!(
                "Carapace not found. For {} completions, add this to your shell rc:",
                name
            );
            println!();
            match name {
                "zsh" => println!("  source <(COMPLETE=zsh vibetree)"),
                "bash" => println!("  source <(COMPLETE=bash vibetree)"),
                "fish" => println!("  COMPLETE=fish vibetree | source"),
                _ => println!("  # See: vibetree completions --help"),
            }
            println!();
        }
        None => {
            println!("Could not detect shell. Generate completions manually:");
            println!("  vibetree completions <shell>");
            println!();
            println!("Available shells: bash, zsh, fish, powershell, elvish, carapace");
        }
    }
}

fn which_carapace() -> Option<std::path::PathBuf> {
    std::process::Command::new("which")
        .arg("carapace")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| std::path::PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
}

fn detect_shell() -> Option<&'static str> {
    std::env::var("SHELL").ok().and_then(|s| {
        if s.contains("zsh") {
            Some("zsh")
        } else if s.contains("bash") {
            Some("bash")
        } else if s.contains("fish") {
            Some("fish")
        } else {
            None
        }
    })
}

fn install_carapace_spec(cmd: &mut clap::Command) {
    // Determine carapace specs directory based on OS
    let specs_dir = if cfg!(target_os = "macos") {
        dirs::home_dir().map(|h| h.join("Library/Application Support/carapace/specs"))
    } else {
        dirs::config_dir().map(|c| c.join("carapace/specs"))
    };

    let Some(specs_dir) = specs_dir else {
        eprintln!("Error: Could not determine carapace specs directory");
        std::process::exit(1);
    };

    // Create directory if it doesn't exist
    if let Err(e) = std::fs::create_dir_all(&specs_dir) {
        eprintln!("Error: Could not create directory {:?}: {}", specs_dir, e);
        std::process::exit(1);
    }

    let spec_path = specs_dir.join("vibetree.yaml");

    // Generate spec to buffer
    let mut buffer = Vec::new();
    generate(Spec, cmd, "vibetree", &mut buffer);
    let spec_str = String::from_utf8_lossy(&buffer);

    // Parse and add dynamic completions
    let mut spec: serde_yaml::Value =
        serde_yaml::from_str(&spec_str).expect("Failed to parse generated carapace spec");

    add_dynamic_completions(&mut spec);

    let spec_yaml = serde_yaml::to_string(&spec).expect("Failed to serialize carapace spec");

    // Write to file
    if let Err(e) = std::fs::write(&spec_path, spec_yaml) {
        eprintln!("Error: Could not write to {:?}: {}", spec_path, e);
        std::process::exit(1);
    }

    println!("Installed carapace completions to {:?}", spec_path);
}

fn generate_carapace_spec(cmd: &mut clap::Command) {
    // Generate base spec to a buffer
    let mut buffer = Vec::new();
    generate(Spec, cmd, "vibetree", &mut buffer);
    let spec_str = String::from_utf8_lossy(&buffer);

    // Parse as YAML, add dynamic completions, and re-serialize
    let mut spec: serde_yaml::Value =
        serde_yaml::from_str(&spec_str).expect("Failed to parse generated carapace spec");

    add_dynamic_completions(&mut spec);

    // Output the modified spec
    print!(
        "{}",
        serde_yaml::to_string(&spec).expect("Failed to serialize carapace spec")
    );
}

fn add_dynamic_completions(spec: &mut serde_yaml::Value) {
    if let Some(commands) = spec.get_mut("commands").and_then(|c| c.as_sequence_mut()) {
        for command in commands.iter_mut() {
            let name = command.get("name").and_then(|n| n.as_str());
            if matches!(name, Some("switch") | Some("remove")) {
                let completion = serde_yaml::Value::Mapping({
                    let mut map = serde_yaml::Mapping::new();
                    map.insert(
                        serde_yaml::Value::String("positional".to_string()),
                        serde_yaml::Value::Sequence(vec![serde_yaml::Value::Sequence(vec![
                            serde_yaml::Value::String("$(vibetree list --format names)".to_string()),
                        ])]),
                    );
                    map
                });
                command.as_mapping_mut().unwrap().insert(
                    serde_yaml::Value::String("completion".to_string()),
                    completion,
                );
            }
        }
    }
}
