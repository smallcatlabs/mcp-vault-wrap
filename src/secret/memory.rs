use std::collections::HashMap;
use std::sync::Mutex;

use super::{SecretBackend, SecretError};

/// In-memory secret backend for automated tests.
pub struct InMemoryBackend {
    store: Mutex<HashMap<(String, String), String>>,
}

impl InMemoryBackend {
    pub fn new() -> Self {
        Self {
            store: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretBackend for InMemoryBackend {
    fn authenticate(&self) -> Result<(), SecretError> {
        Ok(())
    }

    fn get(&self, profile: &str, name: &str) -> Result<String, SecretError> {
        let store = self.store.lock().unwrap();
        let key = (profile.to_string(), name.to_string());
        store
            .get(&key)
            .cloned()
            .ok_or_else(|| SecretError::NotFound {
                service: format!("mcp-vault-wrap.{profile}.{name}"),
            })
    }

    fn set(&self, profile: &str, name: &str, value: &str) -> Result<(), SecretError> {
        let mut store = self.store.lock().unwrap();
        let key = (profile.to_string(), name.to_string());
        store.insert(key, value.to_string());
        Ok(())
    }

    fn delete(&self, profile: &str, name: &str) -> Result<(), SecretError> {
        let mut store = self.store.lock().unwrap();
        let key = (profile.to_string(), name.to_string());
        if store.remove(&key).is_none() {
            return Err(SecretError::NotFound {
                service: format!("mcp-vault-wrap.{profile}.{name}"),
            });
        }
        Ok(())
    }

    fn exists(&self, profile: &str, name: &str) -> Result<bool, SecretError> {
        let store = self.store.lock().unwrap();
        let key = (profile.to_string(), name.to_string());
        Ok(store.contains_key(&key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_get() {
        let backend = InMemoryBackend::new();
        backend.set("default", "TOKEN", "secret123").unwrap();
        assert_eq!(backend.get("default", "TOKEN").unwrap(), "secret123");
    }

    #[test]
    fn get_not_found() {
        let backend = InMemoryBackend::new();
        let err = backend.get("default", "MISSING").unwrap_err();
        match err {
            SecretError::NotFound { service } => {
                assert_eq!(service, "mcp-vault-wrap.default.MISSING");
            }
            _ => panic!("expected NotFound, got {err:?}"),
        }
    }

    #[test]
    fn set_overwrites() {
        let backend = InMemoryBackend::new();
        backend.set("default", "TOKEN", "v1").unwrap();
        backend.set("default", "TOKEN", "v2").unwrap();
        assert_eq!(backend.get("default", "TOKEN").unwrap(), "v2");
    }

    #[test]
    fn delete_existing() {
        let backend = InMemoryBackend::new();
        backend.set("default", "TOKEN", "val").unwrap();
        backend.delete("default", "TOKEN").unwrap();
        assert!(!backend.exists("default", "TOKEN").unwrap());
    }

    #[test]
    fn delete_not_found() {
        let backend = InMemoryBackend::new();
        let err = backend.delete("default", "MISSING").unwrap_err();
        match err {
            SecretError::NotFound { service } => {
                assert_eq!(service, "mcp-vault-wrap.default.MISSING");
            }
            _ => panic!("expected NotFound, got {err:?}"),
        }
    }

    #[test]
    fn exists_true_and_false() {
        let backend = InMemoryBackend::new();
        assert!(!backend.exists("default", "TOKEN").unwrap());
        backend.set("default", "TOKEN", "val").unwrap();
        assert!(backend.exists("default", "TOKEN").unwrap());
    }

    #[test]
    fn authenticate_succeeds() {
        let backend = InMemoryBackend::new();
        backend.authenticate().unwrap();
    }

    #[test]
    fn profiles_are_isolated() {
        let backend = InMemoryBackend::new();
        backend.set("work", "TOKEN", "work_val").unwrap();
        assert_eq!(backend.get("work", "TOKEN").unwrap(), "work_val");
        let err = backend.get("default", "TOKEN").unwrap_err();
        assert!(matches!(err, SecretError::NotFound { .. }));
    }
}
