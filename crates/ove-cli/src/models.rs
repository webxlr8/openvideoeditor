//! `ove models list|install|remove`: the local machine-learning models in the shared model store (the same list as
//! Settings → Storage in the app). Models over 50 MB download only with `--yes`, and never from MCP.

use std::sync::atomic::AtomicBool;

use clap::Subcommand;
use ove_core::revision::Source;
use ove_models::{Ensure, Origin, Store};
use ove_tools::tools::models::open_store;
use serde_json::{Value, json};

use crate::common::{Fail, Out};

#[derive(Subcommand)]
pub enum ModelsCmd {
    /// Every model this version knows, with what is installed, its size and where it is.
    List,
    /// Download and verify a model. Models over 50 MB and the inference runtime need --yes (the person's consent).
    Install {
        id: String,
        /// Confirm the download of a model over 50 MB or of the inference runtime.
        #[arg(long)]
        yes: bool,
        /// Install offline from this folder, which holds the files the pinned URLs name (no network).
        #[arg(long, value_name = "FOLDER")]
        from: Option<std::path::PathBuf>,
    },
    /// Delete an installed model's files.
    Remove { id: String },
}

pub fn run(cmd: &ModelsCmd, source: Source) -> Out {
    run_in(&open_store()?, cmd, source)
}

fn run_in(store: &Store, cmd: &ModelsCmd, source: Source) -> Out {
    match cmd {
        ModelsCmd::List => list(store),
        ModelsCmd::Install { id, yes, from } => {
            // `--yes` typed at a terminal is the person's consent; an MCP call never is.
            let origin = if *yes && source == Source::Cli { Origin::Person } else { Origin::Agent };
            match from {
                // Copying files the person placed in a folder downloads nothing; it is still the person's action.
                Some(folder) if source == Source::Cli => install_from(store, id, folder),
                Some(_) => Err(Fail::new("model.consent", "an offline install runs only from the terminal")),
                None => install(store, id, origin),
            }
        }
        ModelsCmd::Remove { id } => remove(store, id),
    }
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1e6)
}

fn list(store: &Store) -> Out {
    let r = ove_tools::tools::models::list(store);
    Ok((r.json, r.text))
}

fn install(store: &Store, id: &str, origin: Origin) -> Out {
    let m = store.manifest(id)?;
    let already = store.is_installed(id);
    let mut told_host = false;
    let host = m.host().unwrap_or("the pinned location").to_string();
    let mut last_pct = u64::MAX;
    let mut on_progress = |p: &ove_models::Progress| {
        let pct = p.done * 100 / p.total.max(1);
        // Progress goes to stderr; stdout keeps the one result (JSON with --json).
        if !already && !told_host {
            told_host = true;
            eprintln!("downloading {} ({}) from {host}", p.model, mb(p.total));
        }
        if !already && pct != last_pct && (pct.is_multiple_of(5) || pct == 100) {
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

fn install_from(store: &Store, id: &str, folder: &std::path::Path) -> Out {
    let out = store.install_from_folder(id, folder, &mut |_| {})?;
    let status = store.status(id)?;
    let text = if out.already {
        format!("{id} is already installed ({})", mb(status.bytes))
    } else {
        format!("installed {id} ({}) from {}", mb(status.bytes), folder.display())
    };
    Ok((json!({"id": id, "installed": true, "already": out.already, "bytes": status.bytes, "paths": out.paths}), text))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_mcp_call_never_consents_to_a_large_download() {
        let dir = tempfile::tempdir().unwrap();
        // A mirror nobody listens on: a download attempt would fail with a network error, not a consent refusal.
        let store = Store::open(dir.path())
            .with_url_override(Some(ove_models::UrlOverride::Mirror("http://127.0.0.1:9".into())));
        let cmd = ModelsCmd::Install { id: "whisper-base".into(), yes: true, from: None };
        let e = run_in(&store, &cmd, Source::Mcp).unwrap_err();
        assert_eq!(e.code, "model.consent");
        assert_eq!(e.details["bytes"], 147_951_465);
        assert!(!dir.path().join("whisper-base").exists(), "nothing was downloaded");
    }

    #[test]
    fn the_runtime_needs_consent_and_names_its_host() {
        if ove_models::manifest::RuntimePlatform::current().is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path())
            .with_url_override(Some(ove_models::UrlOverride::Mirror("http://127.0.0.1:9".into())));
        for (yes, source) in [(false, Source::Cli), (true, Source::Mcp)] {
            let cmd = ModelsCmd::Install { id: "onnxruntime".into(), yes, from: None };
            let e = run_in(&store, &cmd, source).unwrap_err();
            assert_eq!(e.code, "model.consent");
            assert_eq!(e.details["host"], "files.pythonhosted.org");
            assert!(e.message.starts_with("runtime not installed"), "{}", e.message);
        }
        let cmd = ModelsCmd::Install { id: "onnxruntime".into(), yes: true, from: Some(dir.path().into()) };
        assert_eq!(run_in(&store, &cmd, Source::Mcp).unwrap_err().code, "model.consent");
        assert!(!dir.path().join("onnxruntime").exists(), "nothing was downloaded");
    }
}
