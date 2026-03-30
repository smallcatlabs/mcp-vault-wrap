// TODO: Remove once all modules are wired into command implementations.
#![allow(dead_code)]

mod cli;
mod commands;
mod config;
mod host;
mod inject;
mod migrate;
mod registry;
mod relay;
mod secret;
mod transport;
mod validate;

use std::process;

use clap::Parser;
use cli::{Cli, Commands};

fn create_backend() -> Box<dyn secret::SecretBackend> {
    #[cfg(target_os = "macos")]
    {
        Box::new(secret::keychain::KeychainBackend)
    }
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("Error: mcp-vault-wrap requires macOS Keychain. This platform is not supported.");
        process::exit(1);
    }
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
            server_name: _,
            config: _,
            verbose: _,
        } => {
            todo!("run command")
        }
        Commands::Add {
            profile,
            secret_name,
            force,
        } => {
            let backend = create_backend();
            if let Err(e) = commands::add::run(&*backend, &profile, &secret_name, force) {
                eprintln!("{e}");
                process::exit(1);
            }
        }
        Commands::Remove {
            profile,
            secret_name,
        } => {
            let backend = create_backend();
            if let Err(e) = commands::remove::run(&*backend, &profile, &secret_name) {
                eprintln!("{e}");
                process::exit(1);
            }
        }
        Commands::Migrate {
            host: _,
            servers: _,
            dry_run: _,
            config: _,
        } => {
            todo!("migrate command")
        }
        Commands::Doctor { config: _ } => {
            todo!("doctor command")
        }
    }
}
