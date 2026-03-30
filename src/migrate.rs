use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::config::{ProfileConfig, RelayConfig, ServerConfig};
use crate::host::{HostConfigData, HostConfigError, HostServerEntry};
use crate::registry::{self, EnvClassification};
use crate::secret::{SecretBackend, SecretError};
use crate::validate::is_valid_name;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum MigrateError {
    UnsupportedHost {
        host: String,
    },
    HostConfigError(HostConfigError),
    ServerNotInHostConfig {
        server: String,
        available: Vec<String>,
    },
    ServerNotInRegistry {
        server: String,
        supported: Vec<&'static str>,
    },
    UnrecognizedEnvVars {
        server: String,
        vars: Vec<String>,
    },
    InvalidEnvVarName {
        server: String,
        name: String,
    },
    RelayTomlExists {
        path: String,
    },
    BackupExists {
        path: String,
    },
    KeychainAccessDenied {
        detail: String,
    },
    KeychainWriteFailure {
        secret_name: String,
        detail: String,
    },
    IoError {
        detail: String,
    },
}

impl fmt::Display for MigrateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MigrateError::UnsupportedHost { host } => {
                write!(
                    f,
                    "Error: Unsupported host \"{host}\"\nSupported hosts: claude-desktop"
                )
            }
            MigrateError::HostConfigError(e) => write!(f, "Error: {e}"),
            MigrateError::ServerNotInHostConfig { server, available } => {
                let list = available.join(", ");
                write!(
                    f,
                    "Error: Server \"{server}\" not found in claude_desktop_config.json\n\
                     Available servers: {list}"
                )
            }
            MigrateError::ServerNotInRegistry { server, supported } => {
                let list = supported.join(", ");
                write!(
                    f,
                    "Error: No registry definition for server \"{server}\"\n\
                     Supported servers: {list}"
                )
            }
            MigrateError::UnrecognizedEnvVars { server, vars } => {
                let list = vars.join(", ");
                write!(
                    f,
                    "Error: Server \"{server}\" has unrecognized environment variables: {list}\n\
                     All env vars must be defined in the registry for classification."
                )
            }
            MigrateError::InvalidEnvVarName { server, name } => {
                write!(
                    f,
                    "Error: Server \"{server}\" has invalid env var name: \"{name}\"\n\
                     Names must match [a-zA-Z0-9_.-]"
                )
            }
            MigrateError::RelayTomlExists { path } => {
                write!(
                    f,
                    "Error: Relay config already exists at {path}\n\
                     Remove it and rerun to migrate from scratch."
                )
            }
            MigrateError::BackupExists { path } => {
                write!(
                    f,
                    "Error: Backup already exists at {path}\n\
                     Remove it before running migrate again."
                )
            }
            MigrateError::KeychainAccessDenied { detail } => {
                write!(
                    f,
                    "Error: Cannot access macOS Keychain: {detail}\n\
                     Ensure mcp-vault-wrap has Keychain access in System Settings → Privacy & Security."
                )
            }
            MigrateError::KeychainWriteFailure {
                secret_name,
                detail,
            } => {
                write!(
                    f,
                    "Error: Failed to write secret \"{secret_name}\" to Keychain: {detail}\n\
                     Ensure mcp-vault-wrap has Keychain access in System Settings → Privacy & Security."
                )
            }
            MigrateError::IoError { detail } => {
                write!(f, "Error: {detail}")
            }
        }
    }
}

impl std::error::Error for MigrateError {}

// ---------------------------------------------------------------------------
// Classification result for one server's env vars
// ---------------------------------------------------------------------------

struct ClassifiedServer {
    server_name: String,
    host_entry: HostServerEntry,
    secrets: Vec<(String, String)>,     // (env_var_name, value)
    config_vars: Vec<(String, String)>, // (env_var_name, value)
}

// ---------------------------------------------------------------------------
// Phase 1: Validate
// ---------------------------------------------------------------------------

/// Validate all inputs before any writes. Returns classified server data for
/// Phases 2-4.
fn validate(
    host_config_path: &Path,
    server_names: &[String],
    relay_toml_path: &Path,
    backend: &dyn SecretBackend,
    output: &mut dyn std::io::Write,
    dry_run: bool,
) -> Result<(HostConfigData, Vec<ClassifiedServer>), MigrateError> {
    // Parse host config
    let host_data = parse_host_config(host_config_path)?;

    let mut classified = Vec::new();

    for server_name in server_names {
        // Verify server exists in host config
        let host_entry = host_data.servers.get(server_name).ok_or_else(|| {
            MigrateError::ServerNotInHostConfig {
                server: server_name.clone(),
                available: {
                    let mut names: Vec<String> = host_data.servers.keys().cloned().collect();
                    names.sort();
                    names
                },
            }
        })?;

        // Verify server exists in registry
        let reg_entry =
            registry::lookup(server_name).ok_or_else(|| MigrateError::ServerNotInRegistry {
                server: server_name.clone(),
                supported: registry::supported_servers(),
            })?;

        // Validate all env var names
        for env_name in host_entry.env.keys() {
            if !is_valid_name(env_name) {
                return Err(MigrateError::InvalidEnvVarName {
                    server: server_name.clone(),
                    name: env_name.clone(),
                });
            }
        }

        // Build classification map from registry
        let reg_map: HashMap<&str, EnvClassification> = reg_entry
            .env_vars
            .iter()
            .map(|(name, class)| (*name, *class))
            .collect();

        // Check every env var in host config has a registry classification
        let mut unrecognized: Vec<String> = Vec::new();
        for env_name in host_entry.env.keys() {
            if !reg_map.contains_key(env_name.as_str()) {
                unrecognized.push(env_name.clone());
            }
        }
        if !unrecognized.is_empty() {
            unrecognized.sort();
            return Err(MigrateError::UnrecognizedEnvVars {
                server: server_name.clone(),
                vars: unrecognized,
            });
        }

        // Classify env vars
        let mut secrets = Vec::new();
        let mut config_vars = Vec::new();
        for (env_name, env_value) in &host_entry.env {
            match reg_map.get(env_name.as_str()) {
                Some(EnvClassification::Secret) => {
                    secrets.push((env_name.clone(), env_value.clone()));
                }
                Some(EnvClassification::Config) => {
                    config_vars.push((env_name.clone(), env_value.clone()));
                }
                None => unreachable!(), // already checked above
            }
        }

        classified.push(ClassifiedServer {
            server_name: server_name.clone(),
            host_entry: host_entry.clone(),
            secrets,
            config_vars,
        });
    }

    // Verify relay TOML does not already exist
    if relay_toml_path.exists() {
        return Err(MigrateError::RelayTomlExists {
            path: relay_toml_path.display().to_string(),
        });
    }

    // Verify Keychain is accessible
    backend.authenticate().map_err(|e| match e {
        SecretError::AccessDenied { detail } => MigrateError::KeychainAccessDenied { detail },
        other => MigrateError::KeychainAccessDenied {
            detail: other.to_string(),
        },
    })?;

    // Create backup
    let backup_path = backup_path_for(host_config_path);
    if backup_path.exists() {
        return Err(MigrateError::BackupExists {
            path: backup_path.display().to_string(),
        });
    }

    let prefix = if dry_run { "[dry-run] " } else { "" };
    writeln!(
        output,
        "{prefix}Backing up {} → {}",
        host_config_path.display(),
        backup_path.file_name().unwrap().to_string_lossy()
    )
    .ok();

    if !dry_run {
        std::fs::copy(host_config_path, &backup_path).map_err(|e| MigrateError::IoError {
            detail: format!("Failed to create backup at {}: {e}", backup_path.display()),
        })?;
    }

    Ok((host_data, classified))
}

fn parse_host_config(path: &Path) -> Result<HostConfigData, MigrateError> {
    use crate::host::HostConfig;
    use crate::host::claude_desktop::ClaudeDesktopConfig;

    ClaudeDesktopConfig::parse(path).map_err(MigrateError::HostConfigError)
}

fn backup_path_for(host_config_path: &Path) -> PathBuf {
    let mut backup = host_config_path.to_path_buf();
    let file_name = backup
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    backup.set_file_name(format!("{file_name}.bak"));
    backup
}

// ---------------------------------------------------------------------------
// Phase 2: Write secrets
// ---------------------------------------------------------------------------

fn write_secrets(
    classified: &[ClassifiedServer],
    backend: &dyn SecretBackend,
    output: &mut dyn std::io::Write,
    dry_run: bool,
) -> Result<(), MigrateError> {
    let prefix = if dry_run { "[dry-run] " } else { "" };

    for server in classified {
        writeln!(output, "{prefix}Migrating server: {}", server.server_name).ok();

        for (env_name, env_value) in &server.secrets {
            let service = format!("mcp-vault-wrap.default.{env_name}");
            writeln!(output, "  {env_name} → Keychain ({service})").ok();
            if !dry_run {
                backend.set("default", env_name, env_value).map_err(|e| {
                    MigrateError::KeychainWriteFailure {
                        secret_name: env_name.clone(),
                        detail: e.to_string(),
                    }
                })?;
            }
        }

        for (env_name, _) in &server.config_vars {
            writeln!(output, "  {env_name} → relay.toml env (non-secret)").ok();
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Phase 3: Write relay TOML
// ---------------------------------------------------------------------------

fn write_relay_toml(
    classified: &[ClassifiedServer],
    relay_toml_path: &Path,
    output: &mut dyn std::io::Write,
    dry_run: bool,
) -> Result<(), MigrateError> {
    let prefix = if dry_run { "[dry-run] " } else { "" };

    // Build RelayConfig
    let mut servers = HashMap::new();
    for server in classified {
        let mut secret_env = HashMap::new();
        for (env_name, _) in &server.secrets {
            secret_env.insert(env_name.clone(), format!("vault://default/{env_name}"));
        }

        let mut env = HashMap::new();
        for (env_name, env_value) in &server.config_vars {
            env.insert(env_name.clone(), env_value.clone());
        }

        servers.insert(
            server.server_name.clone(),
            ServerConfig {
                command: server.host_entry.command.clone(),
                args: server.host_entry.args.clone(),
                secret_env,
                env,
            },
        );
    }

    let mut profile = HashMap::new();
    profile.insert("default".to_string(), ProfileConfig { servers });

    let relay_config = RelayConfig {
        config_version: 1,
        profile,
    };

    if dry_run {
        writeln!(
            output,
            "{prefix}Would write relay config → {}",
            relay_toml_path.display()
        )
        .ok();
    } else {
        // Create config directory with 700 permissions if needed
        if let Some(parent) = relay_toml_path.parent()
            && !parent.exists()
        {
            std::fs::create_dir_all(parent).map_err(|e| MigrateError::IoError {
                detail: format!(
                    "Failed to create config directory {}: {e}",
                    parent.display()
                ),
            })?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).map_err(
                    |e| MigrateError::IoError {
                        detail: format!("Failed to set permissions on {}: {e}", parent.display()),
                    },
                )?;
            }
        }

        let toml_str =
            crate::config::serialize(&relay_config).map_err(|e| MigrateError::IoError {
                detail: format!("Failed to serialize relay config: {e}"),
            })?;

        std::fs::write(relay_toml_path, &toml_str).map_err(|e| MigrateError::IoError {
            detail: format!("Failed to write {}: {e}", relay_toml_path.display()),
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(relay_toml_path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| MigrateError::IoError {
                    detail: format!(
                        "Failed to set permissions on {}: {e}",
                        relay_toml_path.display()
                    ),
                })?;
        }

        writeln!(output, "Wrote relay config → {}", relay_toml_path.display()).ok();
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Phase 4: Rewrite host config
// ---------------------------------------------------------------------------

fn rewrite_host_config(
    host_config_path: &Path,
    host_data: &HostConfigData,
    classified: &[ClassifiedServer],
    output: &mut dyn std::io::Write,
    dry_run: bool,
) -> Result<(), MigrateError> {
    let prefix = if dry_run { "[dry-run] " } else { "" };
    let count = classified.len();

    // Build HostConfigData containing ONLY the migrated server entries.
    // write() replaces only entries present in data.servers, leaving all
    // other raw mcpServers entries (and their extra fields) untouched.
    let mut migrated_servers = HashMap::new();
    for server in classified {
        migrated_servers.insert(
            server.server_name.clone(),
            HostServerEntry {
                command: "mcp-vault-wrap".to_string(),
                args: vec!["run".to_string(), server.server_name.clone()],
                env: HashMap::new(),
            },
        );
    }
    let modified_data = HostConfigData {
        servers: migrated_servers,
        raw: host_data.raw.clone(),
    };

    if dry_run {
        writeln!(
            output,
            "{prefix}Would update {} ({count} server{} migrated)",
            host_config_path.display(),
            if count == 1 { "" } else { "s" }
        )
        .ok();
        writeln!(output, "No changes made.").ok();
    } else {
        use crate::host::HostConfig;
        use crate::host::claude_desktop::ClaudeDesktopConfig;

        ClaudeDesktopConfig::write(host_config_path, &modified_data)
            .map_err(MigrateError::HostConfigError)?;

        writeln!(
            output,
            "Updated {} ({count} server{} migrated)",
            host_config_path.display(),
            if count == 1 { "" } else { "s" }
        )
        .ok();
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Execute the migrate command. All output goes to `output` (stdout in production,
/// a buffer in tests). Returns Ok(()) on success.
pub fn execute(
    host_config_path: &Path,
    server_names: &[String],
    relay_toml_path: &Path,
    backend: &dyn SecretBackend,
    dry_run: bool,
    output: &mut dyn std::io::Write,
) -> Result<(), MigrateError> {
    // Phase 1: Validate
    let (host_data, classified) = validate(
        host_config_path,
        server_names,
        relay_toml_path,
        backend,
        output,
        dry_run,
    )?;

    // Phase 2: Write secrets
    write_secrets(&classified, backend, output, dry_run)?;

    // Phase 3: Write relay TOML
    write_relay_toml(&classified, relay_toml_path, output, dry_run)?;

    // Phase 4: Rewrite host config
    rewrite_host_config(host_config_path, &host_data, &classified, output, dry_run)?;

    Ok(())
}

/// Entry point from main — resolves host config path and relay TOML path.
pub fn run(
    host: &str,
    server_names: &[String],
    relay_toml_override: Option<&Path>,
    backend: &dyn SecretBackend,
    dry_run: bool,
) -> Result<(), MigrateError> {
    // Resolve host config path
    let host_config_path =
        crate::host::claude_desktop::resolve_host_config_path(host).map_err(|_| {
            MigrateError::UnsupportedHost {
                host: host.to_string(),
            }
        })?;

    let relay_toml_path = relay_toml_override
        .map(PathBuf::from)
        .unwrap_or_else(crate::commands::run::default_config_path);

    let mut stdout = std::io::stdout();
    execute(
        &host_config_path,
        server_names,
        &relay_toml_path,
        backend,
        dry_run,
        &mut stdout,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::memory::InMemoryBackend;

    /// Write a sample Claude Desktop config JSON file.
    fn write_host_config(dir: &Path) -> PathBuf {
        let path = dir.join("claude_desktop_config.json");
        let config = serde_json::json!({
            "mcpServers": {
                "github": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-github"],
                    "env": {
                        "GITHUB_TOKEN": "ghp_secret123"
                    }
                },
                "slack": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-slack"],
                    "env": {
                        "SLACK_BOT_TOKEN": "xoxb-secret",
                        "SLACK_TEAM_ID": "T01234567"
                    }
                }
            },
            "otherSetting": true
        });
        std::fs::write(&path, config.to_string()).unwrap();
        path
    }

    // -----------------------------------------------------------------------
    // Success path
    // -----------------------------------------------------------------------

    #[test]
    fn migrate_success_github_and_slack() {
        let dir = tempfile::tempdir().unwrap();
        let host_config = write_host_config(dir.path());
        let relay_toml = dir.path().join("relay.toml");
        let backend = InMemoryBackend::new();
        let mut output = Vec::new();

        let result = execute(
            &host_config,
            &["github".to_string(), "slack".to_string()],
            &relay_toml,
            &backend,
            false,
            &mut output,
        );
        assert!(result.is_ok(), "migrate failed: {:?}", result.err());

        // Secrets written to backend
        assert_eq!(
            backend.get("default", "GITHUB_TOKEN").unwrap(),
            "ghp_secret123"
        );
        assert_eq!(
            backend.get("default", "SLACK_BOT_TOKEN").unwrap(),
            "xoxb-secret"
        );
        // Non-secret not in backend
        assert!(backend.get("default", "SLACK_TEAM_ID").is_err());

        // Relay TOML was written
        assert!(relay_toml.exists());
        let toml_content = std::fs::read_to_string(&relay_toml).unwrap();
        let relay_config: RelayConfig = crate::config::deserialize(&toml_content).unwrap();
        assert_eq!(relay_config.config_version, 1);

        let default = &relay_config.profile["default"];
        let github = &default.servers["github"];
        assert_eq!(github.command, "npx");
        assert_eq!(
            github.args,
            vec!["-y", "@modelcontextprotocol/server-github"]
        );
        assert_eq!(
            github.secret_env["GITHUB_TOKEN"],
            "vault://default/GITHUB_TOKEN"
        );
        assert!(github.env.is_empty());

        let slack = &default.servers["slack"];
        assert_eq!(
            slack.secret_env["SLACK_BOT_TOKEN"],
            "vault://default/SLACK_BOT_TOKEN"
        );
        assert_eq!(slack.env["SLACK_TEAM_ID"], "T01234567");

        // Host config was rewritten
        let host_json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&host_config).unwrap()).unwrap();
        assert_eq!(
            host_json["mcpServers"]["github"]["command"],
            "mcp-vault-wrap"
        );
        assert_eq!(
            host_json["mcpServers"]["github"]["args"],
            serde_json::json!(["run", "github"])
        );
        assert!(host_json["mcpServers"]["github"].get("env").is_none());
        assert_eq!(
            host_json["mcpServers"]["slack"]["command"],
            "mcp-vault-wrap"
        );
        // Non-server fields preserved
        assert_eq!(host_json["otherSetting"], serde_json::json!(true));

        // Backup exists
        let backup = dir.path().join("claude_desktop_config.json.bak");
        assert!(backup.exists());

        // Output includes expected messages
        let output_str = String::from_utf8(output).unwrap();
        assert!(output_str.contains("Backing up"));
        assert!(output_str.contains("Migrating server: github"));
        assert!(output_str.contains("GITHUB_TOKEN → Keychain"));
        assert!(output_str.contains("Migrating server: slack"));
        assert!(output_str.contains("SLACK_BOT_TOKEN → Keychain"));
        assert!(output_str.contains("SLACK_TEAM_ID → relay.toml env (non-secret)"));
        assert!(output_str.contains("Wrote relay config"));
        assert!(output_str.contains("2 servers migrated"));
    }

    // -----------------------------------------------------------------------
    // Dry-run
    // -----------------------------------------------------------------------

    #[test]
    fn migrate_dry_run_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let host_config = write_host_config(dir.path());
        let relay_toml = dir.path().join("relay.toml");
        let backend = InMemoryBackend::new();
        let mut output = Vec::new();

        let result = execute(
            &host_config,
            &["github".to_string()],
            &relay_toml,
            &backend,
            true,
            &mut output,
        );
        assert!(result.is_ok());

        // No secrets written
        assert!(backend.get("default", "GITHUB_TOKEN").is_err());

        // No relay TOML written
        assert!(!relay_toml.exists());

        // Host config unchanged (original still has env)
        let host_json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&host_config).unwrap()).unwrap();
        assert_eq!(host_json["mcpServers"]["github"]["command"], "npx");

        // No backup created
        let backup = dir.path().join("claude_desktop_config.json.bak");
        assert!(!backup.exists());

        // Output contains dry-run prefix
        let output_str = String::from_utf8(output).unwrap();
        assert!(output_str.contains("[dry-run]"));
        assert!(output_str.contains("GITHUB_TOKEN → Keychain"));
        assert!(output_str.contains("Would write relay config"));
        assert!(output_str.contains("Would update"));
        assert!(output_str.contains("No changes made."));
    }

    // -----------------------------------------------------------------------
    // Failure: server not found in host config
    // -----------------------------------------------------------------------

    #[test]
    fn migrate_server_not_in_host_config() {
        let dir = tempfile::tempdir().unwrap();
        let host_config = write_host_config(dir.path());
        let relay_toml = dir.path().join("relay.toml");
        let backend = InMemoryBackend::new();
        let mut output = Vec::new();

        let err = execute(
            &host_config,
            &["nonexistent".to_string()],
            &relay_toml,
            &backend,
            false,
            &mut output,
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("Server \"nonexistent\" not found"));
        assert!(msg.contains("Available servers:"));
    }

    // -----------------------------------------------------------------------
    // Failure: server not in registry
    // -----------------------------------------------------------------------

    #[test]
    fn migrate_server_not_in_registry() {
        let dir = tempfile::tempdir().unwrap();
        // Create host config with an unregistered server
        let path = dir.path().join("claude_desktop_config.json");
        let config = serde_json::json!({
            "mcpServers": {
                "custom-server": {
                    "command": "my-server",
                    "args": [],
                    "env": { "TOKEN": "secret" }
                }
            }
        });
        std::fs::write(&path, config.to_string()).unwrap();

        let relay_toml = dir.path().join("relay.toml");
        let backend = InMemoryBackend::new();
        let mut output = Vec::new();

        let err = execute(
            &path,
            &["custom-server".to_string()],
            &relay_toml,
            &backend,
            false,
            &mut output,
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("No registry definition for server \"custom-server\""));
        assert!(msg.contains("Supported servers:"));
    }

    // -----------------------------------------------------------------------
    // Failure: unrecognized env vars
    // -----------------------------------------------------------------------

    #[test]
    fn migrate_unrecognized_env_vars() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude_desktop_config.json");
        let config = serde_json::json!({
            "mcpServers": {
                "github": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-github"],
                    "env": {
                        "GITHUB_TOKEN": "ghp_test",
                        "GITHUB_ENTERPRISE_URL": "https://example.com"
                    }
                }
            }
        });
        std::fs::write(&path, config.to_string()).unwrap();

        let relay_toml = dir.path().join("relay.toml");
        let backend = InMemoryBackend::new();
        let mut output = Vec::new();

        let err = execute(
            &path,
            &["github".to_string()],
            &relay_toml,
            &backend,
            false,
            &mut output,
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("unrecognized environment variables"));
        assert!(msg.contains("GITHUB_ENTERPRISE_URL"));
    }

    // -----------------------------------------------------------------------
    // Failure: relay TOML already exists
    // -----------------------------------------------------------------------

    #[test]
    fn migrate_relay_toml_exists() {
        let dir = tempfile::tempdir().unwrap();
        let host_config = write_host_config(dir.path());
        let relay_toml = dir.path().join("relay.toml");
        std::fs::write(&relay_toml, "existing content").unwrap();

        let backend = InMemoryBackend::new();
        let mut output = Vec::new();

        let err = execute(
            &host_config,
            &["github".to_string()],
            &relay_toml,
            &backend,
            false,
            &mut output,
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("Relay config already exists"));
        assert!(msg.contains("Remove it and rerun"));
    }

    // -----------------------------------------------------------------------
    // Failure: invalid env var name
    // -----------------------------------------------------------------------

    #[test]
    fn migrate_invalid_env_var_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude_desktop_config.json");
        let config = serde_json::json!({
            "mcpServers": {
                "github": {
                    "command": "npx",
                    "args": [],
                    "env": {
                        "MY KEY": "value"
                    }
                }
            }
        });
        std::fs::write(&path, config.to_string()).unwrap();

        let relay_toml = dir.path().join("relay.toml");
        let backend = InMemoryBackend::new();
        let mut output = Vec::new();

        let err = execute(
            &path,
            &["github".to_string()],
            &relay_toml,
            &backend,
            false,
            &mut output,
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("invalid env var name"));
        assert!(msg.contains("MY KEY"));
    }

    // -----------------------------------------------------------------------
    // Failure: backup already exists
    // -----------------------------------------------------------------------

    #[test]
    fn migrate_backup_collision() {
        let dir = tempfile::tempdir().unwrap();
        let host_config = write_host_config(dir.path());
        let relay_toml = dir.path().join("relay.toml");

        // Create .bak file first
        let backup = dir.path().join("claude_desktop_config.json.bak");
        std::fs::write(&backup, "old backup").unwrap();

        let backend = InMemoryBackend::new();
        let mut output = Vec::new();

        let err = execute(
            &host_config,
            &["github".to_string()],
            &relay_toml,
            &backend,
            false,
            &mut output,
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("Backup already exists"));
    }

    // -----------------------------------------------------------------------
    // Idempotent rerun after Phase 2 failure
    // -----------------------------------------------------------------------

    #[test]
    fn migrate_idempotent_rerun() {
        let dir = tempfile::tempdir().unwrap();
        let host_config = write_host_config(dir.path());
        let relay_toml = dir.path().join("relay.toml");
        let backend = InMemoryBackend::new();

        // Pre-set a secret (simulating partial Phase 2 from a previous run)
        backend
            .set("default", "GITHUB_TOKEN", "ghp_secret123")
            .unwrap();

        let mut output = Vec::new();
        let result = execute(
            &host_config,
            &["github".to_string()],
            &relay_toml,
            &backend,
            false,
            &mut output,
        );
        assert!(result.is_ok(), "rerun failed: {:?}", result.err());

        // Secret is overwritten with same value (idempotent)
        assert_eq!(
            backend.get("default", "GITHUB_TOKEN").unwrap(),
            "ghp_secret123"
        );
    }

    // -----------------------------------------------------------------------
    // Relay TOML file permissions (unix only)
    // -----------------------------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn migrate_sets_relay_toml_permissions_600() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let host_config = write_host_config(dir.path());
        let relay_toml = dir.path().join("relay.toml");
        let backend = InMemoryBackend::new();
        let mut output = Vec::new();

        execute(
            &host_config,
            &["github".to_string()],
            &relay_toml,
            &backend,
            false,
            &mut output,
        )
        .unwrap();

        let mode = std::fs::metadata(&relay_toml).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    // -----------------------------------------------------------------------
    // Single server migration
    // -----------------------------------------------------------------------

    #[test]
    fn migrate_single_server() {
        let dir = tempfile::tempdir().unwrap();
        let host_config = write_host_config(dir.path());
        let relay_toml = dir.path().join("relay.toml");
        let backend = InMemoryBackend::new();
        let mut output = Vec::new();

        execute(
            &host_config,
            &["github".to_string()],
            &relay_toml,
            &backend,
            false,
            &mut output,
        )
        .unwrap();

        let output_str = String::from_utf8(output).unwrap();
        assert!(output_str.contains("1 server migrated"));

        // Only github migrated in host config
        let host_json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&host_config).unwrap()).unwrap();
        assert_eq!(
            host_json["mcpServers"]["github"]["command"],
            "mcp-vault-wrap"
        );
        // slack untouched
        assert_eq!(host_json["mcpServers"]["slack"]["command"], "npx");
    }

    // -----------------------------------------------------------------------
    // Untouched servers preserve extra fields through migration
    // -----------------------------------------------------------------------

    #[test]
    fn migrate_preserves_extra_fields_on_untouched_servers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude_desktop_config.json");
        let config = serde_json::json!({
            "mcpServers": {
                "github": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-github"],
                    "env": { "GITHUB_TOKEN": "ghp_secret" }
                },
                "slack": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-slack"],
                    "env": { "SLACK_BOT_TOKEN": "xoxb-test", "SLACK_TEAM_ID": "T01234567" },
                    "disabled": true,
                    "alwaysAllow": ["read_channel"]
                }
            }
        });
        std::fs::write(&path, config.to_string()).unwrap();

        let relay_toml = dir.path().join("relay.toml");
        let backend = InMemoryBackend::new();
        let mut output = Vec::new();

        execute(
            &path,
            &["github".to_string()],
            &relay_toml,
            &backend,
            false,
            &mut output,
        )
        .unwrap();

        let host_json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();

        // github was migrated
        assert_eq!(
            host_json["mcpServers"]["github"]["command"],
            "mcp-vault-wrap"
        );

        // slack's extra fields survived — not in data.servers, so raw preserved
        assert_eq!(host_json["mcpServers"]["slack"]["disabled"], true);
        assert_eq!(
            host_json["mcpServers"]["slack"]["alwaysAllow"],
            serde_json::json!(["read_channel"])
        );
        assert_eq!(host_json["mcpServers"]["slack"]["command"], "npx");
    }

    // -----------------------------------------------------------------------
    // Server with no env vars (all env vars optional in host config)
    // -----------------------------------------------------------------------

    #[test]
    fn migrate_server_with_no_env_vars() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude_desktop_config.json");
        let config = serde_json::json!({
            "mcpServers": {
                "github": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-github"]
                }
            }
        });
        std::fs::write(&path, config.to_string()).unwrap();

        let relay_toml = dir.path().join("relay.toml");
        let backend = InMemoryBackend::new();
        let mut output = Vec::new();

        let result = execute(
            &path,
            &["github".to_string()],
            &relay_toml,
            &backend,
            false,
            &mut output,
        );
        assert!(result.is_ok());
    }

    // -----------------------------------------------------------------------
    // FailingBackend — configurable test backend for error-path coverage
    // -----------------------------------------------------------------------

    /// A test backend that can be configured to fail on authenticate() or set().
    struct FailingBackend {
        fail_authenticate: bool,
        fail_set: bool,
    }

    impl SecretBackend for FailingBackend {
        fn authenticate(&self) -> Result<(), SecretError> {
            if self.fail_authenticate {
                Err(SecretError::AccessDenied {
                    detail: "session locked".to_string(),
                })
            } else {
                Ok(())
            }
        }

        fn get(&self, profile: &str, name: &str) -> Result<String, SecretError> {
            Err(SecretError::NotFound {
                service: format!("mcp-vault-wrap.{profile}.{name}"),
            })
        }

        fn set(&self, _profile: &str, _name: &str, _value: &str) -> Result<(), SecretError> {
            if self.fail_set {
                Err(SecretError::AccessDenied {
                    detail: "access denied".to_string(),
                })
            } else {
                Ok(())
            }
        }

        fn delete(&self, _profile: &str, _name: &str) -> Result<(), SecretError> {
            Ok(())
        }

        fn exists(&self, _profile: &str, _name: &str) -> Result<bool, SecretError> {
            Ok(false)
        }
    }

    // -----------------------------------------------------------------------
    // Failure: Keychain auth failure
    // -----------------------------------------------------------------------

    #[test]
    fn migrate_keychain_auth_failure() {
        let dir = tempfile::tempdir().unwrap();
        let host_config = write_host_config(dir.path());
        let relay_toml = dir.path().join("relay.toml");
        let backend = FailingBackend {
            fail_authenticate: true,
            fail_set: false,
        };
        let mut output = Vec::new();

        let err = execute(
            &host_config,
            &["github".to_string()],
            &relay_toml,
            &backend,
            false,
            &mut output,
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("Cannot access macOS Keychain"), "got: {msg}");
        assert!(msg.contains("Keychain access"), "got: {msg}");
    }

    // -----------------------------------------------------------------------
    // Failure: Keychain write failure during Phase 2
    // -----------------------------------------------------------------------

    #[test]
    fn migrate_keychain_write_failure() {
        let dir = tempfile::tempdir().unwrap();
        let host_config = write_host_config(dir.path());
        let relay_toml = dir.path().join("relay.toml");
        let backend = FailingBackend {
            fail_authenticate: false,
            fail_set: true,
        };
        let mut output = Vec::new();

        let err = execute(
            &host_config,
            &["github".to_string()],
            &relay_toml,
            &backend,
            false,
            &mut output,
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("Failed to write secret"), "got: {msg}");
        assert!(msg.contains("GITHUB_TOKEN"), "got: {msg}");
    }

    // -----------------------------------------------------------------------
    // Unsupported host through migrate::run() entry point
    // -----------------------------------------------------------------------

    #[test]
    fn migrate_run_unsupported_host() {
        let backend = InMemoryBackend::new();
        let err = run("cursor", &["github".to_string()], None, &backend, false).unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("Unsupported host \"cursor\""), "got: {msg}");
        assert!(msg.contains("claude-desktop"), "got: {msg}");
    }

    // -----------------------------------------------------------------------
    // Config directory created with 700 permissions
    // -----------------------------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn migrate_creates_config_dir_with_700_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let host_config = write_host_config(dir.path());
        // Put relay TOML inside a non-existent subdirectory
        let config_dir = dir.path().join("newdir");
        let relay_toml = config_dir.join("relay.toml");
        let backend = InMemoryBackend::new();
        let mut output = Vec::new();

        execute(
            &host_config,
            &["github".to_string()],
            &relay_toml,
            &backend,
            false,
            &mut output,
        )
        .unwrap();

        assert!(config_dir.exists());
        let mode = std::fs::metadata(&config_dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }
}
