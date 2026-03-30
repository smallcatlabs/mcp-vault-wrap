use std::process;

use clap::Parser;
use mcp_vault_wrap::cli::{Cli, Commands};
use mcp_vault_wrap::commands;
use mcp_vault_wrap::secret;

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

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
            server_name,
            config,
            verbose,
        } => {
            let backend = create_backend();
            match commands::run::run(&*backend, &server_name, config.as_deref(), verbose).await {
                Ok(exit_code) => {
                    if exit_code != 0 {
                        eprintln!(
                            "Error: Server \"{}\" exited with code {}",
                            server_name, exit_code
                        );
                        process::exit(exit_code);
                    }
                }
                Err(e) => {
                    eprintln!("{e}");
                    process::exit(1);
                }
            }
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
            host,
            servers,
            dry_run,
            config,
        } => {
            let backend = create_backend();
            if let Err(e) =
                mcp_vault_wrap::migrate::run(&host, &servers, config.as_deref(), &*backend, dry_run)
            {
                eprintln!("{e}");
                process::exit(1);
            }
        }
        Commands::Doctor { config } => {
            let backend = create_backend();
            match commands::doctor::run(&*backend, config.as_deref()) {
                Ok(all_passed) => {
                    if !all_passed {
                        process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("{e}");
                    process::exit(1);
                }
            }
        }
    }
}
