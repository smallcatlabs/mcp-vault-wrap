use std::io::{self, BufRead, IsTerminal, Write};

use crate::secret::{SecretBackend, SecretError};
use crate::validate::is_valid_name;

/// Execute the `add` command: validate names, store secret in backend.
///
/// The secret `value` must already be read from input before calling this.
/// `output` receives the success confirmation (stdout in production).
pub fn execute(
    backend: &dyn SecretBackend,
    profile: &str,
    secret_name: &str,
    force: bool,
    value: &str,
    output: &mut dyn Write,
) -> Result<(), String> {
    // Validate profile name
    if !is_valid_name(profile) {
        return Err(format!(
            "Error: Invalid profile name: \"{profile}\"\nNames must match [a-zA-Z0-9_.-]"
        ));
    }

    // Validate secret name
    if !is_valid_name(secret_name) {
        return Err(format!(
            "Error: Invalid secret name: \"{secret_name}\"\nNames must match [a-zA-Z0-9_.-]"
        ));
    }

    let service_name = format!("mcp-vault-wrap.{profile}.{secret_name}");

    // Check if secret already exists (unless --force)
    let already_exists = if !force {
        match backend.exists(profile, secret_name) {
            Ok(true) => {
                return Err(format!(
                    "Error: Secret \"{service_name}\" already exists in Keychain\n\
                     Use --force to overwrite."
                ));
            }
            Ok(false) => false,
            Err(SecretError::AccessDenied { .. }) => {
                return Err(
                    "Error: Failed to write to Keychain: access denied\n\
                     Ensure mcp-vault-wrap has Keychain access in System Settings \u{2192} Privacy & Security."
                        .to_string(),
                );
            }
            Err(e) => {
                return Err(format!("Error: Failed to write to Keychain: {e}"));
            }
        }
    } else {
        // With --force, check existence to decide Stored vs Updated messaging
        backend.exists(profile, secret_name).unwrap_or(false)
    };

    if value.is_empty() {
        return Err("Error: Secret value cannot be empty".to_string());
    }

    // Write to backend
    backend
        .set(profile, secret_name, value)
        .map_err(|e| match e {
            SecretError::AccessDenied { .. } => {
                "Error: Failed to write to Keychain: access denied\n\
                 Ensure mcp-vault-wrap has Keychain access in System Settings \u{2192} Privacy & Security."
                    .to_string()
            }
            _ => format!("Error: Failed to write to Keychain: {e}"),
        })?;

    // Print confirmation
    if force && already_exists {
        writeln!(output, "Updated: {service_name}").map_err(|e| e.to_string())?;
    } else {
        writeln!(output, "Stored: {service_name}").map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Read the secret value from the user. Uses rpassword for no-echo terminal
/// input; falls back to BufRead::read_line for piped stdin.
fn read_secret(secret_name: &str) -> Result<String, String> {
    if io::stdin().is_terminal() {
        eprint!("Enter secret value for {secret_name}: ");
        rpassword::read_password().map_err(|e| format!("Error: Failed to read secret value: {e}"))
    } else {
        let mut value = String::new();
        io::stdin()
            .lock()
            .read_line(&mut value)
            .map_err(|e| format!("Error: Failed to read secret value: {e}"))?;
        let value = value.trim_end_matches('\n').trim_end_matches('\r');
        Ok(value.to_string())
    }
}

/// Entry point called from main with real stdin/stdout/stderr.
pub fn run(
    backend: &dyn SecretBackend,
    profile: &str,
    secret_name: &str,
    force: bool,
) -> Result<(), String> {
    let value = read_secret(secret_name)?;
    let mut output = io::stdout();
    execute(backend, profile, secret_name, force, &value, &mut output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::memory::InMemoryBackend;

    fn run_add(
        backend: &dyn SecretBackend,
        profile: &str,
        secret_name: &str,
        force: bool,
        value: &str,
    ) -> (Result<(), String>, String) {
        let mut output = Vec::new();
        let result = execute(backend, profile, secret_name, force, value, &mut output);
        (result, String::from_utf8(output).unwrap())
    }

    #[test]
    fn add_new_secret() {
        let backend = InMemoryBackend::new();
        let (result, output) = run_add(&backend, "default", "GITHUB_TOKEN", false, "ghp_abc123");
        assert!(result.is_ok());
        assert_eq!(output, "Stored: mcp-vault-wrap.default.GITHUB_TOKEN\n");
        assert_eq!(
            backend.get("default", "GITHUB_TOKEN").unwrap(),
            "ghp_abc123"
        );
    }

    #[test]
    fn add_duplicate_without_force() {
        let backend = InMemoryBackend::new();
        backend.set("default", "GITHUB_TOKEN", "old").unwrap();
        let (result, _) = run_add(&backend, "default", "GITHUB_TOKEN", false, "new");
        let err = result.unwrap_err();
        assert!(err.contains("already exists in Keychain"), "got: {err}");
        assert!(err.contains("Use --force to overwrite"), "got: {err}");
        // Value should not have changed
        assert_eq!(backend.get("default", "GITHUB_TOKEN").unwrap(), "old");
    }

    #[test]
    fn add_duplicate_with_force() {
        let backend = InMemoryBackend::new();
        backend.set("default", "GITHUB_TOKEN", "old").unwrap();
        let (result, output) = run_add(&backend, "default", "GITHUB_TOKEN", true, "new_value");
        assert!(result.is_ok());
        assert_eq!(output, "Updated: mcp-vault-wrap.default.GITHUB_TOKEN\n");
        assert_eq!(backend.get("default", "GITHUB_TOKEN").unwrap(), "new_value");
    }

    #[test]
    fn add_force_new_secret_says_stored() {
        let backend = InMemoryBackend::new();
        let (result, output) = run_add(&backend, "default", "NEW_TOKEN", true, "value");
        assert!(result.is_ok());
        assert_eq!(output, "Stored: mcp-vault-wrap.default.NEW_TOKEN\n");
    }

    #[test]
    fn add_invalid_profile_name() {
        let backend = InMemoryBackend::new();
        let (result, _) = run_add(&backend, "../etc", "TOKEN", false, "val");
        let err = result.unwrap_err();
        assert!(
            err.contains("Invalid profile name: \"../etc\""),
            "got: {err}"
        );
        assert!(
            err.contains("Names must match [a-zA-Z0-9_.-]"),
            "got: {err}"
        );
    }

    #[test]
    fn add_invalid_secret_name() {
        let backend = InMemoryBackend::new();
        let (result, _) = run_add(&backend, "default", "MY KEY", false, "val");
        let err = result.unwrap_err();
        assert!(
            err.contains("Invalid secret name: \"MY KEY\""),
            "got: {err}"
        );
        assert!(
            err.contains("Names must match [a-zA-Z0-9_.-]"),
            "got: {err}"
        );
    }

    #[test]
    fn add_empty_value_rejected() {
        let backend = InMemoryBackend::new();
        let (result, _) = run_add(&backend, "default", "TOKEN", false, "");
        let err = result.unwrap_err();
        assert!(err.contains("Secret value cannot be empty"), "got: {err}");
    }
}
