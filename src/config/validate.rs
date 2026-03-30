use std::path::Path;

use super::{ConfigError, RelayConfig, VaultUri};

const SUPPORTED_CONFIG_VERSION: u32 = 1;

/// Validate the config version is supported.
pub fn check_version(config: &RelayConfig) -> Result<(), ConfigError> {
    if config.config_version != SUPPORTED_CONFIG_VERSION {
        return Err(ConfigError::UnsupportedVersion {
            found: config.config_version,
        });
    }
    Ok(())
}

/// Check file permissions are 600 (owner read/write only).
///
/// On non-Unix platforms this is a no-op (always passes).
#[cfg(unix)]
pub fn check_permissions(path: &Path) -> Result<(), ConfigError> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::metadata(path).map_err(|e| ConfigError::IoError {
        detail: format!("Cannot read file metadata for {}: {e}", path.display()),
    })?;

    let mode = metadata.permissions().mode() & 0o777;
    if mode != 0o600 {
        return Err(ConfigError::PermissionsTooOpen {
            path: path.display().to_string(),
            mode,
        });
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn check_permissions(_path: &Path) -> Result<(), ConfigError> {
    Ok(())
}

/// Validate all vault:// URIs in secret_env and reject vault:// in env.
pub fn check_vault_uris(config: &RelayConfig) -> Result<(), ConfigError> {
    for profile in config.profile.values() {
        for (server_name, server) in &profile.servers {
            // Every secret_env value must be a valid vault:// URI
            for (key, value) in &server.secret_env {
                VaultUri::parse(value).map_err(|_| ConfigError::InvalidVaultUri {
                    raw: value.clone(),
                    reason: format!("in server \"{server_name}\" secret_env.{key}"),
                })?;
            }

            // No env value may start with vault://
            for (key, value) in &server.env {
                if value.starts_with("vault://") {
                    return Err(ConfigError::VaultUriInEnv {
                        server: server_name.clone(),
                        key: key.clone(),
                    });
                }
            }
        }
    }
    Ok(())
}

/// Run all config validations (version, vault URIs). Permission checks are
/// separate because they require a file path.
pub fn validate(config: &RelayConfig) -> Result<(), ConfigError> {
    check_version(config)?;
    check_vault_uris(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::config::{ProfileConfig, RelayConfig, ServerConfig};

    fn minimal_config(version: u32) -> RelayConfig {
        RelayConfig {
            config_version: version,
            profile: HashMap::new(),
        }
    }

    fn config_with_server(
        secret_env: HashMap<String, String>,
        env: HashMap<String, String>,
    ) -> RelayConfig {
        let mut servers = HashMap::new();
        servers.insert(
            "test-server".to_string(),
            ServerConfig {
                command: "echo".to_string(),
                args: vec![],
                secret_env,
                env,
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
    fn version_1_accepted() {
        assert!(check_version(&minimal_config(1)).is_ok());
    }

    #[test]
    fn version_0_rejected() {
        let err = check_version(&minimal_config(0)).unwrap_err();
        assert!(matches!(err, ConfigError::UnsupportedVersion { found: 0 }));
    }

    #[test]
    fn version_2_rejected() {
        let err = check_version(&minimal_config(2)).unwrap_err();
        assert!(matches!(err, ConfigError::UnsupportedVersion { found: 2 }));
    }

    #[test]
    fn valid_vault_uris_accepted() {
        let mut secret_env = HashMap::new();
        secret_env.insert("TOKEN".to_string(), "vault://default/TOKEN".to_string());
        let config = config_with_server(secret_env, HashMap::new());
        assert!(check_vault_uris(&config).is_ok());
    }

    #[test]
    fn invalid_vault_uri_rejected() {
        let mut secret_env = HashMap::new();
        secret_env.insert("TOKEN".to_string(), "not-a-vault-uri".to_string());
        let config = config_with_server(secret_env, HashMap::new());
        assert!(check_vault_uris(&config).is_err());
    }

    #[test]
    fn vault_uri_in_env_rejected() {
        let mut env = HashMap::new();
        env.insert("TOKEN".to_string(), "vault://default/TOKEN".to_string());
        let config = config_with_server(HashMap::new(), env);
        let err = check_vault_uris(&config).unwrap_err();
        assert!(matches!(err, ConfigError::VaultUriInEnv { .. }));
    }

    #[test]
    fn validate_passes_good_config() {
        let mut secret_env = HashMap::new();
        secret_env.insert("TOKEN".to_string(), "vault://default/TOKEN".to_string());
        let config = config_with_server(secret_env, HashMap::new());
        assert!(validate(&config).is_ok());
    }

    #[test]
    fn validate_rejects_bad_version() {
        let config = minimal_config(99);
        assert!(validate(&config).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn permissions_600_accepted() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.toml");
        std::fs::write(&path, "test").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(check_permissions(&path).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn permissions_644_rejected() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.toml");
        std::fs::write(&path, "test").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = check_permissions(&path).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::PermissionsTooOpen { mode: 0o644, .. }
        ));
    }
}
