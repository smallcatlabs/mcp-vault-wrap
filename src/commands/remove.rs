use std::io::{self, Write};

use crate::secret::{SecretBackend, SecretError};
use crate::validate::is_valid_name;

/// Execute the `remove` command: validate names, check existence, delete from backend.
///
/// `output` receives the success confirmation (stdout in production).
pub fn execute(
    backend: &dyn SecretBackend,
    profile: &str,
    secret_name: &str,
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

    // Check existence first to provide a clear error
    match backend.exists(profile, secret_name) {
        Ok(true) => {}
        Ok(false) => {
            return Err(format!(
                "Error: Secret \"{service_name}\" not found in Keychain"
            ));
        }
        Err(SecretError::AccessDenied { .. }) => {
            return Err(
                "Error: Failed to access Keychain: access denied\n\
                 Ensure mcp-vault-wrap has Keychain access in System Settings \u{2192} Privacy & Security."
                    .to_string(),
            );
        }
        Err(e) => {
            return Err(format!("Error: Failed to access Keychain: {e}"));
        }
    }

    // Delete from backend
    backend
        .delete(profile, secret_name)
        .map_err(|e| match e {
            SecretError::AccessDenied { .. } => {
                "Error: Failed to access Keychain: access denied\n\
                 Ensure mcp-vault-wrap has Keychain access in System Settings \u{2192} Privacy & Security."
                    .to_string()
            }
            _ => format!("Error: Failed to access Keychain: {e}"),
        })?;

    writeln!(output, "Removed: {service_name}").map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::memory::InMemoryBackend;

    fn run_remove(
        backend: &dyn SecretBackend,
        profile: &str,
        secret_name: &str,
    ) -> (Result<(), String>, String) {
        let mut output = Vec::new();
        let result = execute(backend, profile, secret_name, &mut output);
        (result, String::from_utf8(output).unwrap())
    }

    #[test]
    fn remove_existing_secret() {
        let backend = InMemoryBackend::new();
        backend.set("default", "GITHUB_TOKEN", "ghp_abc").unwrap();
        let (result, output) = run_remove(&backend, "default", "GITHUB_TOKEN");
        assert!(result.is_ok());
        assert_eq!(output, "Removed: mcp-vault-wrap.default.GITHUB_TOKEN\n");
        assert!(!backend.exists("default", "GITHUB_TOKEN").unwrap());
    }

    #[test]
    fn remove_not_found() {
        let backend = InMemoryBackend::new();
        let (result, _) = run_remove(&backend, "default", "GITHUB_TOKEN");
        let err = result.unwrap_err();
        assert_eq!(
            err,
            "Error: Secret \"mcp-vault-wrap.default.GITHUB_TOKEN\" not found in Keychain"
        );
    }

    #[test]
    fn remove_invalid_profile_name() {
        let backend = InMemoryBackend::new();
        let (result, _) = run_remove(&backend, "../etc", "TOKEN");
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
    fn remove_invalid_secret_name() {
        let backend = InMemoryBackend::new();
        let (result, _) = run_remove(&backend, "default", "MY KEY");
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
    fn remove_different_profile_isolation() {
        let backend = InMemoryBackend::new();
        backend.set("work", "TOKEN", "val").unwrap();
        let (result, _) = run_remove(&backend, "default", "TOKEN");
        assert!(
            result.is_err(),
            "should not find token in different profile"
        );
        // Original still exists
        assert!(backend.exists("work", "TOKEN").unwrap());
    }
}

/// Entry point called from main with real stdout.
pub fn run(backend: &dyn SecretBackend, profile: &str, secret_name: &str) -> Result<(), String> {
    let mut output = io::stdout();
    execute(backend, profile, secret_name, &mut output)
}
