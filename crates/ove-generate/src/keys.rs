//! Provider API keys: in the OS credential store (macOS Keychain, Windows Credential Manager, Linux kernel keyring),
//! never in project files or logs. Environment variables (`FAL_KEY`, `OPENAI_API_KEY`, `ELEVENLABS_API_KEY`) are used
//! when nothing is stored.

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::catalog::Provider;

const SERVICE: &str = "OpenVideoEditor";

fn account(p: Provider) -> String {
    format!("provider-key:{}", p.id())
}

pub fn env_var(p: Provider) -> &'static str {
    match p {
        Provider::Fal => "FAL_KEY",
        Provider::OpenAi => "OPENAI_API_KEY",
        Provider::ElevenLabs => "ELEVENLABS_API_KEY",
    }
}

/// Where a key came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KeySource {
    Keychain,
    Environment,
}

pub trait KeyStore: Send + Sync {
    fn get(&self, p: Provider) -> Option<(String, KeySource)>;
    fn set(&self, p: Provider, key: &str) -> Result<(), String>;
    fn clear(&self, p: Provider) -> Result<(), String>;
}

/// Checks the shape of a pasted key (no network call).
pub fn validate(key: &str) -> Result<String, String> {
    let k = key.trim();
    if k.is_empty() {
        return Err("Paste an API key.".into());
    }
    if k.chars().any(char::is_whitespace) || k.len() < 16 {
        return Err("That doesn't look like an API key.".into());
    }
    Ok(k.to_string())
}

fn env_key(p: Provider) -> Option<(String, KeySource)> {
    std::env::var(env_var(p))
        .ok()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .map(|k| (k, KeySource::Environment))
}

#[derive(Debug, Default)]
pub struct OsKeyStore;

impl KeyStore for OsKeyStore {
    fn get(&self, p: Provider) -> Option<(String, KeySource)> {
        match keyring::Entry::new(SERVICE, &account(p)).ok().and_then(|e| e.get_password().ok()) {
            Some(k) if !k.is_empty() => Some((k, KeySource::Keychain)),
            _ => env_key(p),
        }
    }

    fn set(&self, p: Provider, key: &str) -> Result<(), String> {
        let k = validate(key)?;
        keyring::Entry::new(SERVICE, &account(p))
            .and_then(|e| e.set_password(&k))
            .map_err(|e| format!("Couldn't save the key in the system credential store: {e}"))
    }

    fn clear(&self, p: Provider) -> Result<(), String> {
        match keyring::Entry::new(SERVICE, &account(p)).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("Couldn't remove the key: {e}")),
        }
    }
}

/// In-memory keys (tests, previews), with the environment as fallback.
#[derive(Debug, Default)]
pub struct MemoryKeyStore(Mutex<BTreeMap<&'static str, String>>);

impl KeyStore for MemoryKeyStore {
    fn get(&self, p: Provider) -> Option<(String, KeySource)> {
        self.0.lock().unwrap().get(p.id()).cloned().map(|k| (k, KeySource::Keychain)).or_else(|| env_key(p))
    }
    fn set(&self, p: Provider, key: &str) -> Result<(), String> {
        self.0.lock().unwrap().insert(p.id(), validate(key)?);
        Ok(())
    }
    fn clear(&self, p: Provider) -> Result<(), String> {
        self.0.lock().unwrap().remove(p.id());
        Ok(())
    }
}

/// Key status per provider, for settings screens (never the key itself).
pub fn status(store: &dyn KeyStore) -> Vec<serde_json::Value> {
    Provider::ALL
        .iter()
        .map(|p| {
            let got = store.get(*p);
            serde_json::json!({
                "provider": p.id(), "label": p.label(), "keyUrl": p.key_url(), "envVar": env_var(*p),
                "hasKey": got.is_some(), "source": got.map(|(_, s)| s),
            })
        })
        .collect()
}
