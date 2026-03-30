pub mod claude_desktop;

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

/// Parsed host configuration data, preserving the raw structure for round-trip writes.
#[derive(Debug, Clone)]
pub struct HostConfigData {
    pub servers: HashMap<String, HostServerEntry>,
    /// Preserves the full original JSON for round-trip fidelity.
    pub raw: serde_json::Value,
}

/// A single server entry from a host config file.
#[derive(Debug, Clone, PartialEq)]
pub struct HostServerEntry {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

#[derive(Debug)]
pub enum HostConfigError {
    IoError {
        detail: String,
    },
    ParseError {
        detail: String,
    },
    ServerNotFound {
        server: String,
        available: Vec<String>,
    },
}

impl fmt::Display for HostConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostConfigError::IoError { detail } => write!(f, "{detail}"),
            HostConfigError::ParseError { detail } => {
                write!(f, "Failed to parse host config: {detail}")
            }
            HostConfigError::ServerNotFound { server, available } => {
                write!(
                    f,
                    "Server \"{server}\" not found in host config\nAvailable servers: {}",
                    available.join(", ")
                )
            }
        }
    }
}

impl std::error::Error for HostConfigError {}

/// Trait for reading and writing MCP host configurations.
pub trait HostConfig {
    fn parse(path: &Path) -> Result<HostConfigData, HostConfigError>;
    fn write(path: &Path, data: &HostConfigData) -> Result<(), HostConfigError>;
}
