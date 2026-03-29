// TODO: Remove once modules are wired into command implementations.
#![allow(dead_code)]

mod cli;
mod config;
mod host;
mod inject;
mod migrate;
mod registry;
mod relay;
mod secret;
mod transport;
mod validate;

use clap::Parser;
use cli::{Cli, Commands};

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
            profile: _,
            secret_name: _,
            force: _,
        } => {
            todo!("add command")
        }
        Commands::Remove {
            profile: _,
            secret_name: _,
        } => {
            todo!("remove command")
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
