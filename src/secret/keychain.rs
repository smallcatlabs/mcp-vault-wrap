use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

use super::{SecretBackend, SecretError};

/// macOS Keychain implementation of `SecretBackend`.
///
/// Maps `(profile, name)` to Keychain service name `mcp-vault-wrap.<profile>.<name>`.
/// Account field is the literal string `mcp-vault-wrap`.
pub struct KeychainBackend;

const ACCOUNT: &str = "mcp-vault-wrap";

/// errSecItemNotFound — the item does not exist.
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
/// errSecAuthFailed — Keychain authentication failed (e.g. wrong password).
const ERR_SEC_AUTH_FAILED: i32 = -25291;
/// errSecInteractionNotAllowed — Keychain is locked and no UI is available.
const ERR_SEC_INTERACTION_NOT_ALLOWED: i32 = -25293;

impl KeychainBackend {
    pub(crate) fn service_name(profile: &str, name: &str) -> String {
        format!("mcp-vault-wrap.{profile}.{name}")
    }

    /// Map a security-framework error to the appropriate SecretError variant.
    fn map_error(e: security_framework::base::Error, service: &str) -> SecretError {
        match e.code() {
            ERR_SEC_ITEM_NOT_FOUND => SecretError::NotFound {
                service: service.to_string(),
            },
            ERR_SEC_AUTH_FAILED | ERR_SEC_INTERACTION_NOT_ALLOWED => SecretError::AccessDenied {
                detail: format!("{e}"),
            },
            _ => SecretError::BackendError {
                detail: format!("Keychain error for \"{service}\": {e}"),
            },
        }
    }
}

impl SecretBackend for KeychainBackend {
    /// Probe Keychain accessibility by attempting a lookup for a known-missing entry.
    ///
    /// Returns `Ok(())` if the Keychain is reachable (even though the probe entry
    /// won't exist). Returns `AccessDenied` if the session is locked or access
    /// is denied.
    fn authenticate(&self) -> Result<(), SecretError> {
        let probe_service = "mcp-vault-wrap.__probe__";
        match get_generic_password(probe_service, ACCOUNT) {
            Ok(_) => Ok(()),                                        // unexpected but harmless
            Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()), // expected: accessible
            Err(e)
                if e.code() == ERR_SEC_AUTH_FAILED
                    || e.code() == ERR_SEC_INTERACTION_NOT_ALLOWED =>
            {
                Err(SecretError::AccessDenied {
                    detail: format!("{e}"),
                })
            }
            Err(e) => Err(SecretError::BackendError {
                detail: format!("Keychain accessibility probe failed: {e}"),
            }),
        }
    }

    fn get(&self, profile: &str, name: &str) -> Result<String, SecretError> {
        let service = Self::service_name(profile, name);
        let bytes =
            get_generic_password(&service, ACCOUNT).map_err(|e| Self::map_error(e, &service))?;
        String::from_utf8(bytes).map_err(|e| SecretError::BackendError {
            detail: format!("Secret \"{service}\" is not valid UTF-8: {e}"),
        })
    }

    /// Idempotent — overwrites if the entry already exists.
    fn set(&self, profile: &str, name: &str, value: &str) -> Result<(), SecretError> {
        let service = Self::service_name(profile, name);
        // Delete first if it exists, to make set idempotent (update).
        let _ = delete_generic_password(&service, ACCOUNT);
        set_generic_password(&service, ACCOUNT, value.as_bytes())
            .map_err(|e| Self::map_error(e, &service))
    }

    fn delete(&self, profile: &str, name: &str) -> Result<(), SecretError> {
        let service = Self::service_name(profile, name);
        delete_generic_password(&service, ACCOUNT).map_err(|e| Self::map_error(e, &service))
    }

    fn exists(&self, profile: &str, name: &str) -> Result<bool, SecretError> {
        let service = Self::service_name(profile, name);
        match get_generic_password(&service, ACCOUNT) {
            Ok(_) => Ok(true),
            Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(false),
            Err(e) => Err(Self::map_error(e, &service)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_name_format() {
        assert_eq!(
            KeychainBackend::service_name("default", "GITHUB_TOKEN"),
            "mcp-vault-wrap.default.GITHUB_TOKEN"
        );
        assert_eq!(
            KeychainBackend::service_name("work", "SLACK_BOT_TOKEN"),
            "mcp-vault-wrap.work.SLACK_BOT_TOKEN"
        );
    }
}
