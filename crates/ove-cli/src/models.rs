//! `ove models list|install|remove`: the local machine-learning models in the shared model store (the same list as
//! Settings → Storage in the app). Models over 50 MB download only with `--yes`, and never from MCP.

use std::sync::atomic::AtomicBool;

use clap::Subcommand;
use ove_core::revision::Source;
use ove_models::{Ensure, ModelError, ModelStatus, Origin, Store};
use serde_json::{Value, json};

use crate::common::{Fail, Out, progress_enabled};

#[derive(Subcommand)]
pub enum ModelsCmd {
    /// Every model this version knows, with what is installed, its size and where it is.
    List,
    /// Download and verify a model. Models over 50 MB need --yes (the person's consent).
    Install {
        id: String,
        /// Confirm the download of a model over 50 MB.
        #[arg(long)]
        yes: bool,
    },
    /// Delete an installed model's files.
    Remove { id: String },
}

impl From<ModelError> for Fail {
    fn from(e: ModelError) -> Self {
        Fail::new(e.code(), e.to_string())
    }
}

fn open_store() -> Result<Store, Fail> {
    let root = ove_models::default_root()
        .ok_or_else(|| Fail::new("model.store", "could not find the app data folder (set OVE_MODELS_DIR)"))?;
    let store = Store::open(root);
    // Speech models saved by earlier versions are recorded where they are, the first time they are seen.
    store.adopt_root_files();
    Ok(store)
}

pub fn run(cmd: &ModelsCmd, source: Source) -> Out {
    let store = open_store()?;
    match cmd {
        ModelsCmd::List => list(&store),
        ModelsCmd::Install { id, yes } => {
            // `--yes` typed at a terminal is the person's consent; an MCP call never is.
            let origin = if *yes && source == Source::Cli { Origin::Person } else { Origin::Agent };
            install(&store, id, origin)
        }
        ModelsCmd::Remove { id } => remove(&store, id),
    }
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1e6)
}

fn status_json(s: &ModelStatus) -> Value {
    serde_json::to_value(s).unwrap_or(Value::Null)
}

fn list(store: &Store) -> Out {
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
    Ok((
        json!({
            "root": store.root(),
            "installedBytes": installed_bytes,
            "models": models.iter().map(status_json).collect::<Vec<_>>(),
        }),
        text,
    ))
}

fn install(store: &Store, id: &str, origin: Origin) -> Out {
    let already = store.manifest(id).map(|_| store.is_installed(id))?;
    let show = progress_enabled();
    let mut last_pct = u64::MAX;
    let mut on_progress = |p: &ove_models::Progress| {
        let pct = p.done * 100 / p.total.max(1);
        if show && !already && pct != last_pct && (pct.is_multiple_of(5) || pct == 100) {
            last_pct = pct;
            eprintln!("downloading {}: {pct}% ({} of {})", p.model, mb(p.done), mb(p.total));
        }
    };
    match store.ensure_installed(id, origin, &mut on_progress, &AtomicBool::new(false))? {
        Ensure::Ready(paths) => {
            let status = store.status(id)?;
            let text = if already {
                format!("{id} is already installed ({})", mb(status.bytes))
            } else {
                format!("installed {id} ({})", mb(status.bytes))
            };
            Ok((json!({"id": id, "installed": true, "already": already, "bytes": status.bytes, "paths": paths}), text))
        }
        Ensure::NeedsConsent(n) => {
            Err(Fail::new("model.consent", n.message.clone()).details(serde_json::to_value(&n).unwrap_or(Value::Null)))
        }
    }
}

fn remove(store: &Store, id: &str) -> Out {
    match store.remove(id)? {
        Some(freed) => {
            let free = ove_models::space::free_bytes(store.root()).ok();
            let free_text = free.map(|f| format!("; {} free", mb(f))).unwrap_or_default();
            Ok((
                json!({"id": id, "removed": true, "freedBytes": freed, "freeBytes": free}),
                format!("removed {id} (freed {}{free_text})", mb(freed)),
            ))
        }
        None => Err(Fail::new("model.notInstalled", format!("{id} is not installed"))),
    }
}
