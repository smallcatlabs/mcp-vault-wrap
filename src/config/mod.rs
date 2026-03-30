pub mod validate;

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::validate::is_valid_name;

/// Top-level relay configuration, serialized as TOML.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RelayConfig {
    pub config_version: u32,
    pub profile: HashMap<String, ProfileConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProfileConfig {
    pub servers: HashMap<String, ServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerConfig {
    pub command: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub secret_env: HashMap<String, String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// A parsed `vault://<profile>/<secret-name>` reference.
#[derive(Debug, Clone, PartialEq)]
pub struct VaultUri {
    pub profile: String,
    pub secret_name: String,
}

#[derive(Debug)]
pub enum ConfigError {
    UnsupportedVersion { found: u32 },
    PermissionsTooOpen { path: String, mode: u32 },
    InvalidVaultUri { raw: String, reason: String },
    VaultUriInEnv { server: String, key: String },
    ParseError { detail: String },
    IoError { detail: String },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::UnsupportedVersion { found } => {
                write!(
                    f,
                    "Relay config version {found} is not supported by this version of mcp-vault-wrap\n\
                     Update mcp-vault-wrap or check your config file."
                )
            }
            ConfigError::PermissionsTooOpen { path, mode } => {
                write!(
                    f,
                    "Relay config permissions are too open ({mode:o}) at {path}\n\
                     Expected 600 (owner read/write only). Fix with: chmod 600 {path}"
                )
            }
            ConfigError::InvalidVaultUri { raw, reason } => {
                write!(f, "Invalid vault URI \"{raw}\": {reason}")
            }
            ConfigError::VaultUriInEnv { server, key } => {
                write!(
                    f,
                    "Server \"{server}\" has vault:// reference in env.{key}\n\
                     Secret references belong in secret_env, not env."
                )
            }
            ConfigError::ParseError { detail } => {
                write!(f, "Failed to parse relay config: {detail}")
            }
            ConfigError::IoError { detail } => {
                write!(f, "{detail}")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

const VAULT_SCHEME: &str = "vault://";

impl VaultUri {
    /// Parse a `vault://<profile>/<secret-name>` string.
    pub fn parse(raw: &str) -> Result<Self, ConfigError> {
        let rest = raw
            .strip_prefix(VAULT_SCHEME)
            .ok_or_else(|| ConfigError::InvalidVaultUri {
                raw: raw.to_string(),
                reason: "must start with vault://".to_string(),
            })?;

        let (profile, secret_name) =
            rest.split_once('/')
                .ok_or_else(|| ConfigError::InvalidVaultUri {
                    raw: raw.to_string(),
                    reason: "expected vault://<profile>/<secret-name>".to_string(),
                })?;

        if profile.is_empty() {
            return Err(ConfigError::InvalidVaultUri {
                raw: raw.to_string(),
                reason: "profile name is empty".to_string(),
            });
        }
        if secret_name.is_empty() {
            return Err(ConfigError::InvalidVaultUri {
                raw: raw.to_string(),
                reason: "secret name is empty".to_string(),
            });
        }

        if !is_valid_name(profile) {
            return Err(ConfigError::InvalidVaultUri {
                raw: raw.to_string(),
                reason: format!("invalid profile name: \"{profile}\""),
            });
        }
        if !is_valid_name(secret_name) {
            return Err(ConfigError::InvalidVaultUri {
                raw: raw.to_string(),
                reason: format!("invalid secret name: \"{secret_name}\""),
            });
        }

        // Reject extra path segments
        if secret_name.contains('/') {
            return Err(ConfigError::InvalidVaultUri {
                raw: raw.to_string(),
                reason: "too many path segments".to_string(),
            });
        }

        Ok(VaultUri {
            profile: profile.to_string(),
            secret_name: secret_name.to_string(),
        })
    }
}

/// Serialize a `RelayConfig` to a TOML string.
pub fn serialize(config: &RelayConfig) -> Result<String, ConfigError> {
    toml::to_string(config).map_err(|e| ConfigError::ParseError {
        detail: e.to_string(),
    })
}

/// Deserialize a TOML string into a `RelayConfig`.
pub fn deserialize(s: &str) -> Result<RelayConfig, ConfigError> {
    toml::from_str(s).map_err(|e| ConfigError::ParseError {
        detail: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- VaultUri tests ---

    #[test]
    fn vault_uri_parses_valid() {
        let uri = VaultUri::parse("vault://default/GITHUB_TOKEN").unwrap();
        assert_eq!(uri.profile, "default");
        assert_eq!(uri.secret_name, "GITHUB_TOKEN");
    }

    #[test]
    fn vault_uri_parses_dotted_names() {
        let uri = VaultUri::parse("vault://my-profile/secret.name").unwrap();
        assert_eq!(uri.profile, "my-profile");
        assert_eq!(uri.secret_name, "secret.name");
    }

    #[test]
    fn vault_uri_rejects_missing_scheme() {
        assert!(VaultUri::parse("default/GITHUB_TOKEN").is_err());
        assert!(VaultUri::parse("http://default/GITHUB_TOKEN").is_err());
    }

    #[test]
    fn vault_uri_rejects_missing_slash() {
        assert!(VaultUri::parse("vault://defaultGITHUB_TOKEN").is_err());
    }

    #[test]
    fn vault_uri_rejects_empty_profile() {
        assert!(VaultUri::parse("vault:///GITHUB_TOKEN").is_err());
    }

    #[test]
    fn vault_uri_rejects_empty_secret() {
        assert!(VaultUri::parse("vault://default/").is_err());
    }

    #[test]
    fn vault_uri_rejects_invalid_profile_name() {
        assert!(VaultUri::parse("vault://../etc/SECRET").is_err());
        assert!(VaultUri::parse("vault://pro file/SECRET").is_err());
    }

    #[test]
    fn vault_uri_rejects_invalid_secret_name() {
        assert!(VaultUri::parse("vault://default/MY KEY").is_err());
        assert!(VaultUri::parse("vault://default/key=value").is_err());
    }

    // --- TOML round-trip tests ---

    fn sample_config() -> RelayConfig {
        let mut secret_env_github = HashMap::new();
        secret_env_github.insert(
            "GITHUB_TOKEN".to_string(),
            "vault://default/GITHUB_TOKEN".to_string(),
        );

        let mut secret_env_slack = HashMap::new();
        secret_env_slack.insert(
            "SLACK_BOT_TOKEN".to_string(),
            "vault://default/SLACK_BOT_TOKEN".to_string(),
        );
        let mut env_slack = HashMap::new();
        env_slack.insert("SLACK_TEAM_ID".to_string(), "T01234567".to_string());

        let mut servers = HashMap::new();
        servers.insert(
            "github".to_string(),
            ServerConfig {
                command: "npx".to_string(),
                args: vec![
                    "-y".to_string(),
                    "@modelcontextprotocol/server-github".to_string(),
                ],
                secret_env: secret_env_github,
                env: HashMap::new(),
            },
        );
        servers.insert(
            "slack".to_string(),
            ServerConfig {
                command: "npx".to_string(),
                args: vec![
                    "-y".to_string(),
                    "@modelcontextprotocol/server-slack".to_string(),
                ],
                secret_env: secret_env_slack,
                env: env_slack,
            },
        );

        let mut profile = HashMap::new();
        profile.insert("default".to_string(), ProfileConfig { servers });

        RelayConfig {
            config_version: 1,
            profile,
        }
    }

    #[test]
    fn toml_round_trip() {
        let config = sample_config();
        let toml_str = serialize(&config).unwrap();
        let deserialized = deserialize(&toml_str).unwrap();
        assert_eq!(config, deserialized);
    }

    #[test]
    fn toml_deserializes_canonical_example() {
        let input = r#"
config_version = 1

[profile.default.servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]

[profile.default.servers.github.secret_env]
GITHUB_TOKEN = "vault://default/GITHUB_TOKEN"

[profile.default.servers.slack]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-slack"]

[profile.default.servers.slack.secret_env]
SLACK_BOT_TOKEN = "vault://default/SLACK_BOT_TOKEN"

[profile.default.servers.slack.env]
SLACK_TEAM_ID = "T01234567"
"#;
        let config = deserialize(input).unwrap();
        assert_eq!(config.config_version, 1);

        let default = &config.profile["default"];
        let github = &default.servers["github"];
        assert_eq!(github.command, "npx");
        assert_eq!(
            github.secret_env["GITHUB_TOKEN"],
            "vault://default/GITHUB_TOKEN"
        );

        let slack = &default.servers["slack"];
        assert_eq!(slack.env["SLACK_TEAM_ID"], "T01234567");
        assert_eq!(
            slack.secret_env["SLACK_BOT_TOKEN"],
            "vault://default/SLACK_BOT_TOKEN"
        );
    }

    #[test]
    fn toml_missing_optional_maps_default_to_empty() {
        let input = r#"
config_version = 1

[profile.default.servers.simple]
command = "echo"
args = ["hello"]
"#;
        let config = deserialize(input).unwrap();
        let simple = &config.profile["default"].servers["simple"];
        assert!(simple.secret_env.is_empty());
        assert!(simple.env.is_empty());
    }

    #[test]
    fn toml_parse_error_on_invalid_input() {
        assert!(deserialize("this is not toml {{{").is_err());
    }
}
