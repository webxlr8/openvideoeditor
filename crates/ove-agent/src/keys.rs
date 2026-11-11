//! The user's Anthropic API key: kept in the OS credential store (macOS Keychain, Windows Credential Manager, Linux
//! kernel keyring), never in files or logs. `ANTHROPIC_API_KEY` in the environment is used when nothing is stored.

use std::sync::Mutex;

/// Where a key came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KeySource {
    Keychain,
    Environment,
}

pub trait KeyStore: Send + Sync {
    fn get(&self) -> Option<(String, KeySource)>;
    fn set(&self, key: &str) -> Result<(), String>;
    fn clear(&self) -> Result<(), String>;
}

/// Checks the shape of a pasted key (no network call).
pub fn validate(key: &str) -> Result<String, String> {
    let k = key.trim();
    if k.is_empty() {
        return Err("Paste an API key.".into());
    }
    if k.chars().any(char::is_whitespace) || k.len() < 20 {
        return Err("That doesn't look like an API key.".into());
    }
    Ok(k.to_string())
}

const SERVICE: &str = "OpenVideoEditor";
const ACCOUNT: &str = "anthropic-api-key";

/// The OS credential store, falling back to the environment.
#[derive(Debug, Default)]
pub struct OsKeyStore;

fn env_key() -> Option<(String, KeySource)> {
    std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .map(|k| (k.trim().to_string(), KeySource::Environment))
}

impl KeyStore for OsKeyStore {
    fn get(&self) -> Option<(String, KeySource)> {
        let stored = keyring::Entry::new(SERVICE, ACCOUNT).ok().and_then(|e| e.get_password().ok());
        match stored {
            Some(k) if !k.is_empty() => Some((k, KeySource::Keychain)),
            _ => env_key(),
        }
    }

    fn set(&self, key: &str) -> Result<(), String> {
        let k = validate(key)?;
        keyring::Entry::new(SERVICE, ACCOUNT)
            .and_then(|e| e.set_password(&k))
            .map_err(|e| format!("Couldn't save the key in the system credential store: {e}"))
    }

    fn clear(&self) -> Result<(), String> {
        match keyring::Entry::new(SERVICE, ACCOUNT).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("Couldn't remove the key: {e}")),
        }
    }
}

/// An in-memory store (tests, browser previews).
#[derive(Debug, Default)]
pub struct MemoryKeyStore(Mutex<Option<String>>);

impl KeyStore for MemoryKeyStore {
    fn get(&self) -> Option<(String, KeySource)> {
        self.0.lock().unwrap().clone().map(|k| (k, KeySource::Keychain))
    }
    fn set(&self, key: &str) -> Result<(), String> {
        *self.0.lock().unwrap() = Some(validate(key)?);
        Ok(())
    }
    fn clear(&self) -> Result<(), String> {
        *self.0.lock().unwrap() = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_and_memory_store() {
        assert!(validate("  ").is_err());
        assert!(validate("short").is_err());
        assert!(validate("sk-ant with space aaaaaaaaaaaaa").is_err());
        assert_eq!(validate(" sk-ant-api03-abcdefghijklmnop \n").unwrap(), "sk-ant-api03-abcdefghijklmnop");
        let m = MemoryKeyStore::default();
        assert!(m.get().is_none());
        m.set("sk-ant-api03-abcdefghijklmnop").unwrap();
        assert_eq!(m.get().unwrap().0, "sk-ant-api03-abcdefghijklmnop");
        m.clear().unwrap();
        assert!(m.get().is_none());
    }
}
