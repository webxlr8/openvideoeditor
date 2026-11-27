//! `list_models_local`: the local machine-learning models in the shared model store (`ove-models`), the same list as
//! `ove models list` and Settings → Storage in the app. Read-only: installing a model over 50 MB needs the person's
//! consent, which no agent call can give.

use ove_models::{ModelError, ModelStatus, Store};
use serde_json::{Value, json};

use crate::ctx::ToolCtx;
use crate::fail::Fail;
use crate::receipt::Receipt;
use crate::spec::Args;

impl From<ModelError> for Fail {
    fn from(e: ModelError) -> Self {
        Fail::new(e.code(), e.to_string())
    }
}

/// The model store at its usual place (`OVE_MODELS_DIR`, else the app's data folder). Speech models saved there by
/// earlier versions are recorded where they are, the first time they are seen.
pub fn open_store() -> Result<Store, Fail> {
    let root = ove_models::default_root()
        .ok_or_else(|| Fail::new("model.store", "could not find the app data folder (set OVE_MODELS_DIR)"))?;
    let store = Store::open(root);
    store.adopt_root_files();
    Ok(store)
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1e6)
}

fn status_json(s: &ModelStatus) -> Value {
    serde_json::to_value(s).unwrap_or(Value::Null)
}

/// Every known model with its size, purpose, licence, state and folder.
pub fn list(store: &Store) -> Receipt {
    let models = store.list();
    let mut text = String::new();
    for m in &models {
        let state = if m.installed { "installed" } else { "not installed" };
        text.push_str(&format!("{:<16} {:>9}  {:<13}  {}\n", m.id, mb(m.bytes), state, m.purpose));
        if let Some(loc) = &m.location {
            text.push_str(&format!("{:<16} {}\n", "", loc.display()));
        }
    }
    text.push_str(&format!("store: {}", store.root().display()));
    let installed_bytes: u64 = models.iter().filter(|m| m.installed).map(|m| m.bytes).sum();
    Receipt::new(
        json!({
            "root": store.root(),
            "installedBytes": installed_bytes,
            "models": models.iter().map(status_json).collect::<Vec<_>>(),
        }),
        text,
    )
}

pub fn list_models_local(_: &mut dyn ToolCtx, _: &Args) -> Result<Receipt, Fail> {
    Ok(list(&open_store()?))
}
