#[cfg(target_os = "macos")]
pub mod keychain;
pub mod memory;

use std::fmt;

/// Trait mediating all secret access. Concrete implementations are macOS Keychain
/// (production) and InMemoryBackend (tests).
pub trait SecretBackend {
    fn authenticate(&self) -> Result<(), SecretError>;
    fn get(&self, profile: &str, name: &str) -> Result<String, SecretError>;
    fn set(&self, profile: &str, name: &str, value: &str) -> Result<(), SecretError>;
    fn delete(&self, profile: &str, name: &str) -> Result<(), SecretError>;
    fn exists(&self, profile: &str, name: &str) -> Result<bool, SecretError>;
}

#[derive(Debug)]
pub enum SecretError {
    NotFound { service: String },
    AccessDenied { detail: String },
    BackendError { detail: String },
}

impl fmt::Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecretError::NotFound { service } => {
                write!(f, "Secret \"{service}\" not found in Keychain")
            }
            SecretError::AccessDenied { detail } => {
                write!(f, "Keychain access denied: {detail}")
            }
            SecretError::BackendError { detail } => {
                write!(f, "Secret backend error: {detail}")
            }
        }
    }
}

impl std::error::Error for SecretError {}
