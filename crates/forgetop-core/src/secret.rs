//! Secret storage for per-connection PATs. Config only stores a key reference; the
//! actual token lives in the OS keychain (or a `FORGETOP_PAT_*` env var fallback).
//! Secrets are never written to the config file or committed to source.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::error::{Error, Result};

pub const SERVICE: &str = "forgetop";
pub const ENV_PREFIX: &str = "FORGETOP_PAT_";

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, secret: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
    /// False for read-only stores (e.g. the env-var fallback).
    fn is_writable(&self) -> bool;
}

/// OS keychain via the `keyring` crate (macOS Keychain / Windows DPAPI / libsecret).
pub struct KeyringSecretStore;

impl SecretStore for KeyringSecretStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        let entry = keyring::Entry::new(SERVICE, key).map_err(kerr)?;
        match entry.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(kerr(e)),
        }
    }

    fn set(&self, key: &str, secret: &str) -> Result<()> {
        keyring::Entry::new(SERVICE, key).map_err(kerr)?.set_password(secret).map_err(kerr)
    }

    fn delete(&self, key: &str) -> Result<()> {
        let entry = keyring::Entry::new(SERVICE, key).map_err(kerr)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(kerr(e)),
        }
    }

    fn is_writable(&self) -> bool {
        true
    }
}

fn kerr(e: keyring::Error) -> Error {
    Error::Provider(format!("keychain: {e}"))
}

/// Read-only fallback resolving secrets from `FORGETOP_PAT_{KEY}` env vars.
pub struct EnvSecretStore;

impl EnvSecretStore {
    pub fn var_name(key: &str) -> String {
        let sanitized: String = key
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
            .collect();
        format!("{ENV_PREFIX}{sanitized}")
    }
}

impl SecretStore for EnvSecretStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(std::env::var(Self::var_name(key)).ok().filter(|s| !s.is_empty()))
    }
    fn set(&self, _key: &str, _secret: &str) -> Result<()> {
        Err(Error::Config("the environment secret store is read-only".into()))
    }
    fn delete(&self, _key: &str) -> Result<()> {
        Err(Error::Config("the environment secret store is read-only".into()))
    }
    fn is_writable(&self) -> bool {
        false
    }
}

/// In-memory store for tests and the Demo provider.
#[derive(Default)]
pub struct InMemorySecretStore {
    secrets: Mutex<HashMap<String, String>>,
}

impl SecretStore for InMemorySecretStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.secrets.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, secret: &str) -> Result<()> {
        self.secrets.lock().unwrap().insert(key.to_string(), secret.to_string());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.secrets.lock().unwrap().remove(key);
        Ok(())
    }
    fn is_writable(&self) -> bool {
        true
    }
}

/// Tries a writable primary (keychain), falling back to a read-only secondary (env).
pub struct FallbackSecretStore {
    primary: Box<dyn SecretStore>,
    fallback: Box<dyn SecretStore>,
}

impl FallbackSecretStore {
    pub fn new(primary: Box<dyn SecretStore>, fallback: Box<dyn SecretStore>) -> Self {
        Self { primary, fallback }
    }
}

impl SecretStore for FallbackSecretStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        match self.primary.get(key)? {
            Some(v) => Ok(Some(v)),
            None => self.fallback.get(key),
        }
    }
    fn set(&self, key: &str, secret: &str) -> Result<()> {
        self.primary.set(key, secret)
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.primary.delete(key)
    }
    fn is_writable(&self) -> bool {
        self.primary.is_writable()
    }
}

/// The default store: OS keychain with an env-var fallback.
pub fn default_secret_store() -> Box<dyn SecretStore> {
    Box::new(FallbackSecretStore::new(Box::new(KeyringSecretStore), Box::new(EnvSecretStore)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_roundtrip() {
        let store = InMemorySecretStore::default();
        store.set("gh-1", "token").unwrap();
        assert_eq!(store.get("gh-1").unwrap().as_deref(), Some("token"));
        store.delete("gh-1").unwrap();
        assert_eq!(store.get("gh-1").unwrap(), None);
    }

    #[test]
    fn env_var_name_is_sanitized() {
        assert_eq!(EnvSecretStore::var_name("test-conn"), "FORGETOP_PAT_TEST_CONN");
    }

    #[test]
    fn fallback_reads_primary_then_env() {
        let primary = InMemorySecretStore::default();
        primary.set("in-primary", "p").unwrap();
        let store = FallbackSecretStore::new(Box::new(primary), Box::new(InMemorySecretStore::default()));
        assert_eq!(store.get("in-primary").unwrap().as_deref(), Some("p"));
        assert_eq!(store.get("missing").unwrap(), None);
    }
}
