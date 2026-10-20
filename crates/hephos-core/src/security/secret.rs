use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{Error, Result};

/// A protected wrapper for sensitive values (API keys, tokens, credentials)
/// that redacts them from debug and display outputs and avoids accidental log leakage.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret<T> {
    inner: T,
}

impl<T> Secret<T> {
    /// Wrap a sensitive value.
    pub fn new(inner: T) -> Self {
        Secret { inner }
    }

    /// Explicitly expose the underlying secret for authorized operations (e.g. HTTP headers).
    pub fn expose_secret(&self) -> &T {
        &self.inner
    }

    /// Consume the wrapper and return the underlying secret value.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

/// An alias for string secrets.
pub type SecretString = Secret<String>;

impl From<String> for SecretString {
    fn from(s: String) -> Self {
        Secret::new(s)
    }
}

impl From<&str> for SecretString {
    fn from(s: &str) -> Self {
        Secret::new(s.to_string())
    }
}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[REDACTED]")
    }
}

impl<T> fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[REDACTED]")
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Secret<T> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        T::deserialize(deserializer).map(Secret::new)
    }
}

impl<T> Serialize for Secret<T> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str("[REDACTED]")
    }
}

/// Pluggable provider for retrieving secrets securely without hardcoding.
pub trait SecretProvider: Send + Sync + 'static {
    fn get_secret(&self, key: &str) -> Result<Option<SecretString>>;
}

/// Secret provider reading from environment variables.
#[derive(Debug, Clone, Default)]
pub struct EnvSecretProvider;

impl SecretProvider for EnvSecretProvider {
    fn get_secret(&self, key: &str) -> Result<Option<SecretString>> {
        match std::env::var(key) {
            Ok(val) => Ok(Some(SecretString::new(val))),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(e) => Err(Error::internal(format!(
                "failed to read secret env '{key}': {e}"
            ))),
        }
    }
}

/// Secret provider reading from files (e.g. Docker or Kubernetes secrets).
#[derive(Debug, Clone)]
pub struct FileSecretProvider {
    base_dir: PathBuf,
}

impl FileSecretProvider {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        FileSecretProvider {
            base_dir: base_dir.into(),
        }
    }
}

impl SecretProvider for FileSecretProvider {
    fn get_secret(&self, key: &str) -> Result<Option<SecretString>> {
        if key.contains("..") || key.contains('/') || key.contains('\\') {
            return Err(Error::invalid(format!("invalid secret key path: {key}")));
        }
        let path = self.base_dir.join(key);
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| Error::internal(format!("failed to read secret file '{key}': {e}")))?;
        Ok(Some(SecretString::new(content.trim().to_string())))
    }
}

/// Chains multiple providers, querying them sequentially until a secret is found.
#[derive(Clone)]
pub struct ChainedSecretProvider {
    providers: Vec<Arc<dyn SecretProvider>>,
}

impl ChainedSecretProvider {
    pub fn new(providers: Vec<Arc<dyn SecretProvider>>) -> Self {
        ChainedSecretProvider { providers }
    }
}

impl SecretProvider for ChainedSecretProvider {
    fn get_secret(&self, key: &str) -> Result<Option<SecretString>> {
        for provider in &self.providers {
            if let Some(secret) = provider.get_secret(key)? {
                return Ok(Some(secret));
            }
        }
        Ok(None)
    }
}

/// Unified secrets handle for accessing application credentials.
#[derive(Clone)]
pub struct Secrets {
    provider: Arc<dyn SecretProvider>,
}

impl Secrets {
    pub fn new(provider: impl SecretProvider) -> Self {
        Secrets {
            provider: Arc::new(provider),
        }
    }

    /// Read secrets from environment variables.
    pub fn from_env() -> Self {
        Self::new(EnvSecretProvider)
    }

    /// Read secrets from files in a given base directory.
    pub fn from_file(base_dir: impl Into<PathBuf>) -> Self {
        Self::new(FileSecretProvider::new(base_dir))
    }

    /// Standard production provider: checks `/run/secrets` (Docker/K8s) first,
    /// then falls back to environment variables.
    pub fn standard() -> Self {
        let file = Arc::new(FileSecretProvider::new("/run/secrets"));
        let env = Arc::new(EnvSecretProvider);
        Self::new(ChainedSecretProvider::new(vec![file, env]))
    }

    /// Retrieve an optional secret.
    pub fn get(&self, key: &str) -> Result<Option<SecretString>> {
        self.provider.get_secret(key)
    }

    /// Retrieve a secret or return `Error::invalid` if missing.
    pub fn require(&self, key: &str) -> Result<SecretString> {
        self.get(key)?
            .ok_or_else(|| Error::invalid(format!("missing required secret: {key}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_and_display_are_redacted() {
        let secret = SecretString::from("super-secret-password");
        assert_eq!(format!("{secret:?}"), "[REDACTED]");
        assert_eq!(format!("{secret}"), "[REDACTED]");
        assert_eq!(secret.expose_secret(), "super-secret-password");
    }

    #[test]
    fn secret_serde_deserializes_and_redacts_serialization() {
        let json = r#""plain-secret""#;
        let secret: SecretString = serde_json::from_str(json).unwrap();
        assert_eq!(secret.expose_secret(), "plain-secret");

        let serialized = serde_json::to_string(&secret).unwrap();
        assert_eq!(serialized, r#""[REDACTED]""#);
    }

    #[test]
    fn env_and_file_secret_providers() {
        let temp_dir =
            std::env::temp_dir().join(format!("hephos-test-secrets-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let secret_file = temp_dir.join("api_key");
        std::fs::write(&secret_file, "file-secret-value\n").unwrap();

        let file_provider = FileSecretProvider::new(&temp_dir);
        let val = file_provider.get_secret("api_key").unwrap().unwrap();
        assert_eq!(val.expose_secret(), "file-secret-value");

        assert!(file_provider.get_secret("nonexistent").unwrap().is_none());
        assert!(file_provider.get_secret("../etc/passwd").is_err());

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }
}
