use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "mcp-vault-wrap",
    version,
    about = "Move MCP credentials from plaintext config into macOS Keychain"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Relay MCP traffic for a server, resolving secrets from Keychain
    Run {
        /// Server name as defined in relay config
        server_name: String,

        /// Override the default relay TOML path
        #[arg(long)]
        config: Option<PathBuf>,

        /// Emit relay startup and operational messages to stderr
        #[arg(long)]
        verbose: bool,
    },

    /// Store a secret in Keychain
    Add {
        /// Profile name
        profile: String,

        /// Secret name
        secret_name: String,

        /// Overwrite an existing secret
        #[arg(long)]
        force: bool,
    },

    /// Remove a secret from Keychain
    Remove {
        /// Profile name
        profile: String,

        /// Secret name
        secret_name: String,
    },

    /// Migrate MCP server credentials from host config to Keychain
    Migrate {
        /// MCP host to migrate from
        #[arg(long)]
        host: String,

        /// Comma-separated list of server names to migrate
        #[arg(long, value_delimiter = ',', required = true)]
        servers: Vec<String>,

        /// Preview changes without writing anything
        #[arg(long)]
        dry_run: bool,

        /// Override the default relay TOML output path
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// Diagnose configuration and Keychain issues
    Doctor {
        /// Override the default relay TOML path
        #[arg(long)]
        config: Option<PathBuf>,
    },
}
