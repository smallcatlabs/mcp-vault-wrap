use std::fmt;
use std::path::{Path, PathBuf};

use crate::config::{self, ConfigError, VaultUri};
use crate::secret::{SecretBackend, SecretError};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum DoctorError {
    IoError { detail: String },
}

impl fmt::Display for DoctorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DoctorError::IoError { detail } => write!(f, "Error: {detail}"),
        }
    }
}

impl std::error::Error for DoctorError {}

// ---------------------------------------------------------------------------
// Default config path (reuse from run)
// ---------------------------------------------------------------------------

fn default_config_path() -> PathBuf {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"));
    home.join(".config")
        .join("mcp-vault-wrap")
        .join("relay.toml")
}

// ---------------------------------------------------------------------------
// Diagnostic result
// ---------------------------------------------------------------------------

/// Result of running doctor diagnostics. Separates logic from output so tests
/// can inspect structured results.
#[derive(Debug)]
pub struct DiagnosticReport {
    pub config_path: String,
    pub config_status: ConfigStatus,
    pub keychain_status: Option<KeychainStatus>,
    pub server_checks: Vec<ServerCheck>,
    pub issue_count: usize,
}

#[derive(Debug)]
pub enum ConfigStatus {
    NotFound,
    ParseError { detail: String },
    VersionError { found: u32 },
    InvalidStructure { detail: String },
    PermissionsError { mode: u32 },
    PermissionsUnknown { detail: String },
    Valid,
}

#[derive(Debug)]
pub enum KeychainStatus {
    Accessible,
    Inaccessible { detail: String },
}

#[derive(Debug)]
pub struct ServerCheck {
    pub name: String,
    pub entries: Vec<EntryCheck>,
}

#[derive(Debug)]
pub struct EntryCheck {
    pub name: String,
    pub kind: EntryKind,
    pub status: EntryStatus,
}

#[derive(Debug)]
pub enum EntryKind {
    Secret,
    Config,
}

#[derive(Debug)]
pub enum EntryStatus {
    Present,
    Missing { fix_hint: String },
    BackendError { detail: String },
}

// ---------------------------------------------------------------------------
// Core diagnostic logic
// ---------------------------------------------------------------------------

pub fn diagnose(
    backend: &dyn SecretBackend,
    config_override: Option<&Path>,
) -> Result<DiagnosticReport, DoctorError> {
    let config_path = config_override
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    let config_path_str = config_path.display().to_string();

    // Step 1: Check config exists
    if !config_path.exists() {
        return Ok(DiagnosticReport {
            config_path: config_path_str,
            config_status: ConfigStatus::NotFound,
            keychain_status: None,
            server_checks: vec![],
            issue_count: 1,
        });
    }

    // Step 2: Parse config
    let content = std::fs::read_to_string(&config_path).map_err(|e| DoctorError::IoError {
        detail: format!("Cannot read {}: {e}", config_path.display()),
    })?;

    let relay_config = match config::deserialize(&content) {
        Ok(c) => c,
        Err(e) => {
            return Ok(DiagnosticReport {
                config_path: config_path_str,
                config_status: ConfigStatus::ParseError {
                    detail: e.to_string(),
                },
                keychain_status: None,
                server_checks: vec![],
                issue_count: 1,
            });
        }
    };

    // Step 3: Validate version
    if let Err(ConfigError::UnsupportedVersion { found }) =
        config::validate::check_version(&relay_config)
    {
        return Ok(DiagnosticReport {
            config_path: config_path_str,
            config_status: ConfigStatus::VersionError { found },
            keychain_status: None,
            server_checks: vec![],
            issue_count: 1,
        });
    }

    // Step 4: Validate config structure (vault URIs, env values)
    if let Err(e) = config::validate::check_vault_uris(&relay_config) {
        return Ok(DiagnosticReport {
            config_path: config_path_str,
            config_status: ConfigStatus::InvalidStructure {
                detail: e.to_string(),
            },
            keychain_status: None,
            server_checks: vec![],
            issue_count: 1,
        });
    }

    // Step 5: Check file permissions
    match config::validate::check_permissions(&config_path) {
        Ok(()) => {}
        Err(ConfigError::PermissionsTooOpen { mode, .. }) => {
            return Ok(DiagnosticReport {
                config_path: config_path_str,
                config_status: ConfigStatus::PermissionsError { mode },
                keychain_status: None,
                server_checks: vec![],
                issue_count: 1,
            });
        }
        Err(e) => {
            return Ok(DiagnosticReport {
                config_path: config_path_str,
                config_status: ConfigStatus::PermissionsUnknown {
                    detail: e.to_string(),
                },
                keychain_status: None,
                server_checks: vec![],
                issue_count: 1,
            });
        }
    }

    // Config is valid at this point
    let config_status = ConfigStatus::Valid;

    // Step 5: Check Keychain accessibility
    let keychain_status = match backend.authenticate() {
        Ok(()) => KeychainStatus::Accessible,
        Err(e) => {
            let detail = match &e {
                SecretError::AccessDenied { detail } => detail.clone(),
                _ => e.to_string(),
            };
            KeychainStatus::Inaccessible { detail }
        }
    };

    // If Keychain is inaccessible, skip server checks
    if matches!(keychain_status, KeychainStatus::Inaccessible { .. }) {
        return Ok(DiagnosticReport {
            config_path: config_path_str,
            config_status,
            keychain_status: Some(keychain_status),
            server_checks: vec![],
            issue_count: 1,
        });
    }

    // Step 6: Check each server in profile.default
    let mut server_checks = Vec::new();
    let mut issue_count = 0;

    if let Some(default_profile) = relay_config.profile.get("default") {
        let mut server_names: Vec<&String> = default_profile.servers.keys().collect();
        server_names.sort();

        for server_name in server_names {
            let server = &default_profile.servers[server_name];
            let mut entries = Vec::new();

            // Check secret_env entries — use exists(), never get()
            let mut secret_keys: Vec<&String> = server.secret_env.keys().collect();
            secret_keys.sort();

            for key in secret_keys {
                let uri_str = &server.secret_env[key];
                // Parse the vault URI to get profile and secret name
                let status = match VaultUri::parse(uri_str) {
                    Ok(uri) => match backend.exists(&uri.profile, &uri.secret_name) {
                        Ok(true) => EntryStatus::Present,
                        Ok(false) => {
                            issue_count += 1;
                            EntryStatus::Missing {
                                fix_hint: format!(
                                    "Run: mcp-vault-wrap add {} {}",
                                    uri.profile, uri.secret_name
                                ),
                            }
                        }
                        Err(e) => {
                            issue_count += 1;
                            EntryStatus::BackendError {
                                detail: e.to_string(),
                            }
                        }
                    },
                    Err(_) => {
                        issue_count += 1;
                        EntryStatus::Missing {
                            fix_hint: format!("Invalid vault URI: {uri_str}"),
                        }
                    }
                };

                entries.push(EntryCheck {
                    name: key.clone(),
                    kind: EntryKind::Secret,
                    status,
                });
            }

            // Report non-secret env entries (informational)
            let mut env_keys: Vec<&String> = server.env.keys().collect();
            env_keys.sort();

            for key in env_keys {
                entries.push(EntryCheck {
                    name: key.clone(),
                    kind: EntryKind::Config,
                    status: EntryStatus::Present,
                });
            }

            server_checks.push(ServerCheck {
                name: server_name.clone(),
                entries,
            });
        }
    }

    Ok(DiagnosticReport {
        config_path: config_path_str,
        config_status,
        keychain_status: Some(keychain_status),
        server_checks,
        issue_count,
    })
}

// ---------------------------------------------------------------------------
// Output formatting (P-§7)
// ---------------------------------------------------------------------------

impl DiagnosticReport {
    /// Format the report to match product spec §7 output examples.
    pub fn format(&self) -> String {
        let mut out = String::new();

        match &self.config_status {
            ConfigStatus::NotFound => {
                out.push_str(&format!(
                    "Config: {} not found\n\
                     Run \"mcp-vault-wrap migrate --host claude-desktop --servers ...\" to set up.\n",
                    self.config_path
                ));
                return out;
            }
            ConfigStatus::ParseError { detail } => {
                out.push_str(&format!(
                    "Config: {} (parse error: {})\n",
                    self.config_path, detail
                ));
                return out;
            }
            ConfigStatus::VersionError { found } => {
                out.push_str(&format!(
                    "Config: {} (unsupported version {})\n\
                     Update mcp-vault-wrap or check your config file.\n",
                    self.config_path, found
                ));
                return out;
            }
            ConfigStatus::InvalidStructure { detail } => {
                out.push_str(&format!(
                    "Config: {} (invalid structure)\n\
                     {detail}\n",
                    self.config_path
                ));
                return out;
            }
            ConfigStatus::PermissionsError { mode } => {
                out.push_str(&format!(
                    "Config: {} (valid, version 1)\n\
                     \u{2717} File permissions are too open ({:o}). Expected 600.\n\
                     Fix with: chmod 600 {}\n",
                    self.config_path, mode, self.config_path
                ));
                return out;
            }
            ConfigStatus::PermissionsUnknown { detail } => {
                out.push_str(&format!(
                    "Config: {} (valid, version 1)\n\
                     \u{2717} Could not check file permissions: {detail}\n",
                    self.config_path
                ));
                return out;
            }
            ConfigStatus::Valid => {
                out.push_str(&format!(
                    "Config: {} (valid, version 1, permissions ok)\n",
                    self.config_path
                ));
            }
        }

        // Keychain status
        match &self.keychain_status {
            Some(KeychainStatus::Accessible) => {
                out.push_str("Keychain: accessible\n");
            }
            Some(KeychainStatus::Inaccessible { detail }) => {
                out.push_str(&format!(
                    "Keychain: inaccessible ({detail})\n\
                     Ensure your desktop session is unlocked and mcp-vault-wrap has Keychain access.\n\
                     Cannot check secrets \u{2014} skipping server checks.\n"
                ));
                return out;
            }
            None => {}
        }

        // Server checks
        for server in &self.server_checks {
            out.push_str(&format!("Server \"{}\":\n", server.name));
            for entry in &server.entries {
                match (&entry.kind, &entry.status) {
                    (EntryKind::Secret, EntryStatus::Present) => {
                        out.push_str(&format!("  \u{2713} {} present in Keychain\n", entry.name));
                    }
                    (EntryKind::Secret, EntryStatus::Missing { fix_hint }) => {
                        out.push_str(&format!(
                            "  \u{2717} {} not found in Keychain\n\
                             \x20   {}\n",
                            entry.name, fix_hint
                        ));
                    }
                    (EntryKind::Secret, EntryStatus::BackendError { detail }) => {
                        out.push_str(&format!(
                            "  \u{2717} {} could not be checked: {}\n",
                            entry.name, detail
                        ));
                    }
                    (EntryKind::Config, EntryStatus::Present) => {
                        out.push_str(&format!(
                            "  \u{2713} {} in relay config (non-secret)\n",
                            entry.name
                        ));
                    }
                    (EntryKind::Config, EntryStatus::Missing { fix_hint }) => {
                        out.push_str(&format!(
                            "  \u{2717} {} missing from relay config\n\
                             \x20   {}\n",
                            entry.name, fix_hint
                        ));
                    }
                    (EntryKind::Config, EntryStatus::BackendError { .. }) => {
                        // Config entries are never checked via backend — unreachable
                    }
                }
            }
        }

        // Summary
        if self.issue_count == 0 {
            out.push_str("All checks passed.\n");
        } else if self.issue_count == 1 {
            out.push_str("1 issue found.\n");
        } else {
            out.push_str(&format!("{} issues found.\n", self.issue_count));
        }

        out
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run the doctor command. Returns Ok(true) if all checks pass, Ok(false) if
/// issues were found.
pub fn run(
    backend: &dyn SecretBackend,
    config_override: Option<&Path>,
) -> Result<bool, DoctorError> {
    let report = diagnose(backend, config_override)?;
    let all_passed = report.issue_count == 0;
    print!("{}", report.format());
    Ok(all_passed)
}

// ---------------------------------------------------------------------------
// Tests (I-39)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::memory::InMemoryBackend;
    use std::os::unix::fs::PermissionsExt;

    /// Helper: write a valid relay TOML with github and slack servers.
    fn write_valid_config(path: &Path) {
        let toml = r#"
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
        std::fs::write(path, toml).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    /// Collect output lines, trimming the trailing empty line from the final \n.
    fn output_lines(report: &DiagnosticReport) -> Vec<String> {
        let text = report.format();
        let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
        // Remove trailing empty element if text ends with \n
        if lines.last().map_or(false, |l| l.is_empty()) {
            lines.pop();
        }
        lines
    }

    // --- Locked backend helper ---

    struct LockedBackend;
    impl SecretBackend for LockedBackend {
        fn authenticate(&self) -> Result<(), crate::secret::SecretError> {
            Err(crate::secret::SecretError::AccessDenied {
                detail: "session locked".to_string(),
            })
        }
        fn get(&self, _: &str, _: &str) -> Result<String, crate::secret::SecretError> {
            unreachable!()
        }
        fn set(&self, _: &str, _: &str, _: &str) -> Result<(), crate::secret::SecretError> {
            unreachable!()
        }
        fn delete(&self, _: &str, _: &str) -> Result<(), crate::secret::SecretError> {
            unreachable!()
        }
        fn exists(&self, _: &str, _: &str) -> Result<bool, crate::secret::SecretError> {
            unreachable!()
        }
    }

    // -----------------------------------------------------------------------
    // All-pass (P-§7 "All Checks Pass")
    // -----------------------------------------------------------------------

    #[test]
    fn all_checks_pass() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("relay.toml");
        write_valid_config(&config_path);

        let backend = InMemoryBackend::new();
        backend.set("default", "GITHUB_TOKEN", "ghp_xxx").unwrap();
        backend
            .set("default", "SLACK_BOT_TOKEN", "xoxb-xxx")
            .unwrap();

        let report = diagnose(&backend, Some(&config_path)).unwrap();

        assert_eq!(report.issue_count, 0);
        assert!(matches!(report.config_status, ConfigStatus::Valid));
        assert!(matches!(
            report.keychain_status,
            Some(KeychainStatus::Accessible)
        ));
        assert_eq!(report.server_checks.len(), 2);

        let lines = output_lines(&report);
        let path = &config_path.display().to_string();
        assert_eq!(
            lines,
            vec![
                format!("Config: {path} (valid, version 1, permissions ok)"),
                "Keychain: accessible".to_string(),
                "Server \"github\":".to_string(),
                "  \u{2713} GITHUB_TOKEN present in Keychain".to_string(),
                "Server \"slack\":".to_string(),
                "  \u{2713} SLACK_BOT_TOKEN present in Keychain".to_string(),
                "  \u{2713} SLACK_TEAM_ID in relay config (non-secret)".to_string(),
                "All checks passed.".to_string(),
            ]
        );
    }

    // -----------------------------------------------------------------------
    // Missing secret (P-§7 "Issues Found")
    // -----------------------------------------------------------------------

    #[test]
    fn missing_secret_reported() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("relay.toml");
        write_valid_config(&config_path);

        let backend = InMemoryBackend::new();
        // Only add SLACK_BOT_TOKEN, leave GITHUB_TOKEN missing
        backend
            .set("default", "SLACK_BOT_TOKEN", "xoxb-xxx")
            .unwrap();

        let report = diagnose(&backend, Some(&config_path)).unwrap();
        assert_eq!(report.issue_count, 1);

        let lines = output_lines(&report);
        let path = &config_path.display().to_string();
        assert_eq!(
            lines,
            vec![
                format!("Config: {path} (valid, version 1, permissions ok)"),
                "Keychain: accessible".to_string(),
                "Server \"github\":".to_string(),
                "  \u{2717} GITHUB_TOKEN not found in Keychain".to_string(),
                "    Run: mcp-vault-wrap add default GITHUB_TOKEN".to_string(),
                "Server \"slack\":".to_string(),
                "  \u{2713} SLACK_BOT_TOKEN present in Keychain".to_string(),
                "  \u{2713} SLACK_TEAM_ID in relay config (non-secret)".to_string(),
                "1 issue found.".to_string(),
            ]
        );
    }

    // -----------------------------------------------------------------------
    // Missing config (P-§7 "No Config")
    // -----------------------------------------------------------------------

    #[test]
    fn missing_config_reported() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("nonexistent.toml");

        let backend = InMemoryBackend::new();
        let report = diagnose(&backend, Some(&config_path)).unwrap();

        assert_eq!(report.issue_count, 1);
        assert!(matches!(report.config_status, ConfigStatus::NotFound));

        let lines = output_lines(&report);
        let path = &config_path.display().to_string();
        assert_eq!(
            lines,
            vec![
                format!("Config: {path} not found"),
                "Run \"mcp-vault-wrap migrate --host claude-desktop --servers ...\" to set up."
                    .to_string(),
            ]
        );
    }

    // -----------------------------------------------------------------------
    // Bad permissions (P-§7 "Bad Permissions")
    // -----------------------------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn bad_permissions_reported() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("relay.toml");
        write_valid_config(&config_path);
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let backend = InMemoryBackend::new();
        let report = diagnose(&backend, Some(&config_path)).unwrap();

        assert_eq!(report.issue_count, 1);
        assert!(matches!(
            report.config_status,
            ConfigStatus::PermissionsError { mode: 0o644 }
        ));

        let lines = output_lines(&report);
        let path = &config_path.display().to_string();
        assert_eq!(
            lines,
            vec![
                format!("Config: {path} (valid, version 1)"),
                "\u{2717} File permissions are too open (644). Expected 600.".to_string(),
                format!("Fix with: chmod 600 {path}"),
            ]
        );
    }

    // -----------------------------------------------------------------------
    // Inaccessible Keychain (P-§7 "Keychain Inaccessible")
    // -----------------------------------------------------------------------

    #[test]
    fn inaccessible_keychain_skips_server_checks() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("relay.toml");
        write_valid_config(&config_path);

        let backend = LockedBackend;
        let report = diagnose(&backend, Some(&config_path)).unwrap();

        assert_eq!(report.issue_count, 1);
        assert!(matches!(
            report.keychain_status,
            Some(KeychainStatus::Inaccessible { .. })
        ));
        assert!(report.server_checks.is_empty());

        let lines = output_lines(&report);
        let path = &config_path.display().to_string();
        assert_eq!(
            lines,
            vec![
                format!("Config: {path} (valid, version 1, permissions ok)"),
                "Keychain: inaccessible (session locked)".to_string(),
                "Ensure your desktop session is unlocked and mcp-vault-wrap has Keychain access."
                    .to_string(),
                "Cannot check secrets \u{2014} skipping server checks.".to_string(),
            ]
        );
    }

    // -----------------------------------------------------------------------
    // Invalid config version
    // -----------------------------------------------------------------------

    #[test]
    fn invalid_config_version_reported() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("relay.toml");
        let toml = "config_version = 99\n\n[profile]\n";
        std::fs::write(&config_path, toml).unwrap();
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600)).unwrap();

        let backend = InMemoryBackend::new();
        let report = diagnose(&backend, Some(&config_path)).unwrap();

        assert_eq!(report.issue_count, 1);
        assert!(matches!(
            report.config_status,
            ConfigStatus::VersionError { found: 99 }
        ));

        let lines = output_lines(&report);
        let path = &config_path.display().to_string();
        assert_eq!(
            lines,
            vec![
                format!("Config: {path} (unsupported version 99)"),
                "Update mcp-vault-wrap or check your config file.".to_string(),
            ]
        );
    }

    // -----------------------------------------------------------------------
    // Parse error
    // -----------------------------------------------------------------------

    #[test]
    fn parse_error_reported() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("relay.toml");
        std::fs::write(&config_path, "this is not valid toml {{{").unwrap();
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600)).unwrap();

        let backend = InMemoryBackend::new();
        let report = diagnose(&backend, Some(&config_path)).unwrap();

        assert_eq!(report.issue_count, 1);
        assert!(matches!(
            report.config_status,
            ConfigStatus::ParseError { .. }
        ));

        let output = report.format();
        let path = &config_path.display().to_string();
        // Output starts with the config path and parse error
        assert!(output.starts_with(&format!("Config: {path} (parse error:")));
    }

    // -----------------------------------------------------------------------
    // Multiple missing secrets
    // -----------------------------------------------------------------------

    #[test]
    fn multiple_missing_secrets_counted() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("relay.toml");
        write_valid_config(&config_path);

        let backend = InMemoryBackend::new();
        let report = diagnose(&backend, Some(&config_path)).unwrap();

        assert_eq!(report.issue_count, 2);

        let lines = output_lines(&report);
        assert_eq!(lines.last().unwrap(), "2 issues found.");
    }

    // -----------------------------------------------------------------------
    // Invalid vault URI in secret_env (Finding 1: structural validation)
    // -----------------------------------------------------------------------

    #[test]
    fn invalid_vault_uri_caught_as_config_error() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("relay.toml");
        let toml = r#"
config_version = 1

[profile.default.servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]

[profile.default.servers.github.secret_env]
GITHUB_TOKEN = "not-a-vault-uri"
"#;
        std::fs::write(&config_path, toml).unwrap();
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600)).unwrap();

        let backend = InMemoryBackend::new();
        let report = diagnose(&backend, Some(&config_path)).unwrap();

        assert_eq!(report.issue_count, 1);
        assert!(matches!(
            report.config_status,
            ConfigStatus::InvalidStructure { .. }
        ));
        // Should not reach server checks — reported as config-level error
        assert!(report.server_checks.is_empty());

        let lines = output_lines(&report);
        let path = &config_path.display().to_string();
        assert_eq!(lines[0], format!("Config: {path} (invalid structure)"));
        assert_eq!(lines.len(), 2);
    }

    // -----------------------------------------------------------------------
    // vault:// in env (non-secret) section caught as config error
    // -----------------------------------------------------------------------

    #[test]
    fn vault_uri_in_env_caught_as_config_error() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("relay.toml");
        let toml = r#"
config_version = 1

[profile.default.servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]

[profile.default.servers.github.env]
GITHUB_TOKEN = "vault://default/GITHUB_TOKEN"
"#;
        std::fs::write(&config_path, toml).unwrap();
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600)).unwrap();

        let backend = InMemoryBackend::new();
        let report = diagnose(&backend, Some(&config_path)).unwrap();

        assert_eq!(report.issue_count, 1);
        assert!(matches!(
            report.config_status,
            ConfigStatus::InvalidStructure { .. }
        ));
        assert!(report.server_checks.is_empty());
    }

    // -----------------------------------------------------------------------
    // Bad permissions on malformed config: reports parse error, not "valid"
    // (Finding 2: permissions after parse+validate)
    // -----------------------------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn bad_permissions_on_malformed_config_reports_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("relay.toml");
        std::fs::write(&config_path, "this is not valid toml {{{").unwrap();
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let backend = InMemoryBackend::new();
        let report = diagnose(&backend, Some(&config_path)).unwrap();

        // Should report parse error, not permissions — parse runs first
        assert!(matches!(
            report.config_status,
            ConfigStatus::ParseError { .. }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn bad_permissions_on_wrong_version_reports_version_error() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("relay.toml");
        let toml = "config_version = 99\n\n[profile]\n";
        std::fs::write(&config_path, toml).unwrap();
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let backend = InMemoryBackend::new();
        let report = diagnose(&backend, Some(&config_path)).unwrap();

        // Should report version error, not permissions — version runs first
        assert!(matches!(
            report.config_status,
            ConfigStatus::VersionError { found: 99 }
        ));
    }

    // -----------------------------------------------------------------------
    // Backend error on exists() reported distinctly from "missing"
    // -----------------------------------------------------------------------

    #[test]
    fn backend_error_on_exists_not_reported_as_missing() {
        /// A backend where authenticate() succeeds but exists() returns a
        /// BackendError for any secret.
        struct FlakyBackend;
        impl SecretBackend for FlakyBackend {
            fn authenticate(&self) -> Result<(), crate::secret::SecretError> {
                Ok(())
            }
            fn get(&self, _: &str, _: &str) -> Result<String, crate::secret::SecretError> {
                unreachable!()
            }
            fn set(&self, _: &str, _: &str, _: &str) -> Result<(), crate::secret::SecretError> {
                unreachable!()
            }
            fn delete(&self, _: &str, _: &str) -> Result<(), crate::secret::SecretError> {
                unreachable!()
            }
            fn exists(&self, _: &str, _: &str) -> Result<bool, crate::secret::SecretError> {
                Err(crate::secret::SecretError::BackendError {
                    detail: "unexpected Keychain error".to_string(),
                })
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("relay.toml");
        write_valid_config(&config_path);

        let backend = FlakyBackend;
        let report = diagnose(&backend, Some(&config_path)).unwrap();

        assert_eq!(report.issue_count, 2); // both secrets fail

        // Verify the output says "could not be checked", not "not found"
        let output = report.format();
        assert!(
            !output.contains("not found in Keychain"),
            "should not say 'not found' for backend errors"
        );
        assert!(output.contains("could not be checked"));
        assert!(output.contains("unexpected Keychain error"));
        // Should NOT suggest `add` — the secret may already exist
        assert!(
            !output.contains("mcp-vault-wrap add"),
            "should not suggest 'add' when the real problem is a backend error"
        );
    }

    // -----------------------------------------------------------------------
    // Permission check I/O failure reported, not silently ignored
    // -----------------------------------------------------------------------

    #[test]
    fn permissions_io_error_not_silently_ignored() {
        // We simulate a permission-check I/O failure by deleting the file
        // after it has been read but before permissions are checked.
        // Since diagnose reads the file content first (step 2) and checks
        // permissions later (step 5), we can't easily do this in a single
        // call. Instead, test the formatting path directly.

        let report = DiagnosticReport {
            config_path: "/some/path/relay.toml".to_string(),
            config_status: ConfigStatus::PermissionsUnknown {
                detail: "No such file or directory".to_string(),
            },
            keychain_status: None,
            server_checks: vec![],
            issue_count: 1,
        };

        let lines = output_lines(&report);
        assert_eq!(
            lines,
            vec![
                "Config: /some/path/relay.toml (valid, version 1)".to_string(),
                "\u{2717} Could not check file permissions: No such file or directory".to_string(),
            ]
        );
    }
}
