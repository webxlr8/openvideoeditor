//! The model store: `<root>/<id>/<version>/<files>` plus `<root>/installed.json`.
//!
//! A model counts as installed only when `installed.json` records it at the manifest's version and every recorded
//! file is on disk with its size. Files are written as `.part` and renamed after their hash matched, so features
//! never see an incomplete model. Files already on disk from before the store (whisper models in `<root>`) are
//! adopted in place: hashed once and recorded where they are.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Condvar, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::download::{self, FileJob, io_err, part_path, sha256_file};
use crate::error::{ModelError, Result};
use crate::manifest::{self, Manifest, RuntimeKind};

pub const INDEX_FILE: &str = "installed.json";

/// A model or runtime larger than this downloads only after the person confirms.
pub const CONSENT_ABOVE_BYTES: u64 = 50 * 1_000_000;

/// Who asks for a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The person confirmed the download (the app's prompt, or `--yes` typed at the terminal).
    Person,
    /// A feature in the app needs the model and nobody was asked.
    Feature,
    /// An agent, an MCP call or a CLI call without `--yes`.
    Agent,
}

/// Result of [`Store::ensure_installed`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ensure {
    /// Installed (now or before): the model's files in manifest order.
    Ready(Vec<PathBuf>),
    /// Not installed, and too large to download without the person's consent. Nothing was downloaded.
    NeedsConsent(NotInstalled),
}

/// A "model not installed" result for agents, the CLI and MCP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotInstalled {
    pub id: String,
    pub purpose: String,
    pub bytes: u64,
    /// How to install it.
    pub install: String,
    pub message: String,
}

impl NotInstalled {
    pub fn for_manifest(m: &Manifest) -> NotInstalled {
        let bytes = m.total_bytes();
        let install = format!("ove models install {} --yes", m.id);
        NotInstalled {
            id: m.id.clone(),
            purpose: m.purpose.clone(),
            bytes,
            message: format!(
                "the model {} ({}) is not installed; it is a {} MB download that needs your consent: install it in                  Settings > Storage or run `{install}`",
                m.id,
                m.purpose,
                crate::error::mb_up(bytes)
            ),
            install,
        }
    }
}

/// Download progress of a whole model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub model: String,
    /// The file being downloaded.
    pub file: String,
    /// Bytes of the model on disk so far.
    pub done: u64,
    /// Bytes of the whole model.
    pub total: u64,
}

/// Where downloads come from instead of the pinned URLs (tests and offline mirrors).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlOverride {
    /// `<base>/<id>/<version>/<file>`.
    Mirror(String),
    /// `<base>/<file>` (the former whisper-only override).
    Flat(String),
}

impl UrlOverride {
    /// `OVE_MODEL_BASE_URL`, when set.
    pub fn from_env() -> Option<UrlOverride> {
        std::env::var("OVE_MODEL_BASE_URL").ok().filter(|s| !s.is_empty()).map(UrlOverride::Mirror)
    }
}

/// One model as the Storage list and `ove models list` show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub id: String,
    pub version: String,
    pub purpose: String,
    pub licence: String,
    pub runtime: RuntimeKind,
    pub in_process: bool,
    /// Size of the whole model.
    pub bytes: u64,
    pub installed: bool,
    /// The folder holding the model's files, when installed.
    pub location: Option<PathBuf>,
}

/// Result of [`Store::install`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallOutcome {
    /// True when the model was already installed and nothing was downloaded.
    pub already: bool,
    /// The model's files, in manifest order.
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Index {
    version: u32,
    models: Vec<Record>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    id: String,
    version: String,
    files: Vec<RecordFile>,
    installed_at_ms: u64,
    /// True when the files were on disk before the store and were recorded where they are.
    #[serde(default)]
    adopted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecordFile {
    name: String,
    /// Relative to the store root (with `/`), or absolute when the file lives elsewhere.
    path: String,
    sha256: String,
    bytes: u64,
}

type FreeSpace = Box<dyn Fn(&Path) -> io::Result<u64> + Send + Sync>;

/// The models on this computer.
pub struct Store {
    root: PathBuf,
    catalog: Vec<Manifest>,
    url_override: Option<UrlOverride>,
    free_space: FreeSpace,
    agent: ureq::Agent,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").field("root", &self.root).field("url_override", &self.url_override).finish()
    }
}

impl Store {
    /// The store at `root` with the built-in manifests and the `OVE_MODEL_BASE_URL` override.
    pub fn open(root: impl Into<PathBuf>) -> Store {
        Store::with_catalog(root, manifest::catalog()).with_url_override(UrlOverride::from_env())
    }

    /// The store at `root` with these manifests (tests and feature-specific stores).
    pub fn with_catalog(root: impl Into<PathBuf>, catalog: Vec<Manifest>) -> Store {
        Store {
            root: root.into(),
            catalog,
            url_override: None,
            free_space: Box::new(crate::space::free_bytes),
            agent: download::agent(),
        }
    }

    pub fn with_url_override(mut self, o: Option<UrlOverride>) -> Store {
        self.url_override = o;
        self
    }

    /// Replaces the free-space probe (tests).
    pub fn with_free_space(mut self, f: impl Fn(&Path) -> io::Result<u64> + Send + Sync + 'static) -> Store {
        self.free_space = Box::new(f);
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn catalog(&self) -> &[Manifest] {
        &self.catalog
    }

    pub fn manifest(&self, id: &str) -> Result<&Manifest> {
        self.catalog.iter().find(|m| m.id == id).ok_or_else(|| ModelError::UnknownModel(id.into()))
    }

    /// Where a model's downloaded files go.
    pub fn model_dir(&self, m: &Manifest) -> PathBuf {
        self.root.join(&m.id).join(&m.version)
    }

    /// The URL a file is fetched from.
    pub fn url_for(&self, m: &Manifest, file: &manifest::ModelFile) -> String {
        match &self.url_override {
            None => file.url.clone(),
            Some(UrlOverride::Mirror(base)) => {
                format!("{}/{}/{}/{}", base.trim_end_matches('/'), m.id, m.version, file.name)
            }
            Some(UrlOverride::Flat(base)) => format!("{}/{}", base.trim_end_matches('/'), file.name),
        }
    }

    /// Every known model with its install state.
    pub fn list(&self) -> Vec<ModelStatus> {
        let index = self.read_index();
        self.catalog.iter().map(|m| self.status_from(m, &index)).collect()
    }

    pub fn status(&self, id: &str) -> Result<ModelStatus> {
        let m = self.manifest(id)?;
        Ok(self.status_from(m, &self.read_index()))
    }

    fn status_from(&self, m: &Manifest, index: &Index) -> ModelStatus {
        let paths = self.complete_paths(m, index);
        ModelStatus {
            id: m.id.clone(),
            version: m.version.clone(),
            purpose: m.purpose.clone(),
            licence: m.licence.clone(),
            runtime: m.runtime,
            in_process: m.in_process,
            bytes: m.total_bytes(),
            installed: paths.is_some(),
            location: paths.and_then(|p| p.first().and_then(|f| f.parent().map(Path::to_path_buf))),
        }
    }

    /// The model's files when it is installed and complete; `None` otherwise.
    pub fn installed_paths(&self, id: &str) -> Option<Vec<PathBuf>> {
        let m = self.manifest(id).ok()?;
        self.complete_paths(m, &self.read_index())
    }

    pub fn is_installed(&self, id: &str) -> bool {
        self.installed_paths(id).is_some()
    }

    /// One file of an installed model.
    pub fn file_path(&self, id: &str, name: &str) -> Option<PathBuf> {
        let m = self.manifest(id).ok()?;
        let i = m.files.iter().position(|f| f.name == name)?;
        self.installed_paths(id)?.into_iter().nth(i)
    }

    fn complete_paths(&self, m: &Manifest, index: &Index) -> Option<Vec<PathBuf>> {
        let rec = index.models.iter().find(|r| r.id == m.id && r.version == m.version)?;
        m.files
            .iter()
            .map(|f| {
                let rf = rec.files.iter().find(|r| r.name == f.name && r.sha256 == f.sha256 && r.bytes == f.bytes)?;
                let p = self.resolve(&rf.path);
                std::fs::metadata(&p).is_ok_and(|md| md.is_file() && md.len() == f.bytes).then_some(p)
            })
            .collect()
    }

    /// Downloads a model unless it is installed. Checks free space first (remaining bytes plus 10 % of the model);
    /// one download per model at a time in this process; `cancel` is checked at every chunk.
    pub fn install(
        &self,
        id: &str,
        progress: &mut dyn FnMut(&Progress),
        cancel: &AtomicBool,
    ) -> Result<InstallOutcome> {
        let m = self.manifest(id)?.clone();
        m.validate()?;
        if let Some(paths) = self.installed_paths(id) {
            let total = m.total_bytes();
            progress(&Progress { model: m.id.clone(), file: String::new(), done: total, total });
            return Ok(InstallOutcome { already: true, paths });
        }
        let _busy = Busy::take(&self.root, &m.id)?;
        let dir = self.model_dir(&m);
        let total = m.total_bytes();

        // Bytes already on disk: finished files that still verify by size, and partial files.
        let on_disk: Vec<u64> = m
            .files
            .iter()
            .map(|f| {
                let dest = dir.join(&f.name);
                match std::fs::metadata(&dest) {
                    Ok(md) if md.len() == f.bytes => f.bytes,
                    _ => std::fs::metadata(part_path(&dest)).map(|md| md.len().min(f.bytes)).unwrap_or(0),
                }
            })
            .collect();
        let remaining = total - on_disk.iter().sum::<u64>();
        let needed = remaining + total / 10;
        let free = (self.free_space)(&self.root).map_err(|e| io_err("check the free space of", &self.root, e))?;
        if free < needed {
            return Err(ModelError::NoSpace { model: m.id.clone(), needed, free });
        }

        let mut paths = Vec::with_capacity(m.files.len());
        let mut base = 0u64;
        for f in &m.files {
            let dest = dir.join(&f.name);
            let finished = std::fs::metadata(&dest).is_ok_and(|md| md.len() == f.bytes)
                && sha256_file(&dest).is_ok_and(|h| h == f.sha256);
            if !finished {
                let _ = std::fs::remove_file(&dest);
                let url = self.url_for(&m, f);
                let job = FileJob { model: &m.id, file: f, url: &url, dest: &dest };
                let mut on_bytes = |done: u64| {
                    progress(&Progress { model: m.id.clone(), file: f.name.clone(), done: base + done, total })
                };
                download::fetch_file(&self.agent, &job, &mut on_bytes, cancel)?;
            }
            base += f.bytes;
            progress(&Progress { model: m.id.clone(), file: f.name.clone(), done: base, total });
            paths.push(dest);
        }
        self.record(&m, &paths, false)?;
        Ok(InstallOutcome { already: false, paths })
    }

    /// Makes sure a model is installed, honouring consent: a model over [`CONSENT_ABOVE_BYTES`] downloads only for
    /// [`Origin::Person`]; for anyone else it returns [`Ensure::NeedsConsent`] and downloads nothing. Smaller models
    /// download for any origin, with progress.
    pub fn ensure_installed(
        &self,
        id: &str,
        origin: Origin,
        progress: &mut dyn FnMut(&Progress),
        cancel: &AtomicBool,
    ) -> Result<Ensure> {
        let m = self.manifest(id)?;
        if let Some(paths) = self.installed_paths(id) {
            return Ok(Ensure::Ready(paths));
        }
        if m.total_bytes() > CONSENT_ABOVE_BYTES && origin != Origin::Person {
            return Ok(Ensure::NeedsConsent(NotInstalled::for_manifest(m)));
        }
        Ok(Ensure::Ready(self.install(id, progress, cancel)?.paths))
    }

    /// Adopts every known model whose files sit directly in the store root with their pinned sizes and hashes
    /// (speech models saved there by earlier versions). Each file is hashed once; later calls find them recorded.
    /// Returns the ids adopted now.
    pub fn adopt_root_files(&self) -> Vec<String> {
        let index = self.read_index();
        let mut adopted = Vec::new();
        for m in &self.catalog {
            if self.complete_paths(m, &index).is_some() {
                continue;
            }
            let present = m.files.iter().all(|f| {
                std::fs::metadata(self.root.join(&f.name)).is_ok_and(|md| md.is_file() && md.len() == f.bytes)
            });
            if present && self.adopt_in_place(&m.id, &self.root).unwrap_or(false) {
                adopted.push(m.id.clone());
            }
        }
        adopted
    }

    /// Records a model whose files already sit in `dir` (for example whisper files from before the store), after
    /// checking each file's size and hash. Returns false, and records nothing, when a file is missing or differs.
    pub fn adopt_in_place(&self, id: &str, dir: &Path) -> Result<bool> {
        let m = self.manifest(id)?.clone();
        if self.installed_paths(id).is_some() {
            return Ok(true);
        }
        let mut paths = Vec::with_capacity(m.files.len());
        for f in &m.files {
            let p = dir.join(&f.name);
            if !std::fs::metadata(&p).is_ok_and(|md| md.is_file() && md.len() == f.bytes) {
                return Ok(false);
            }
            if sha256_file(&p)? != f.sha256 {
                return Ok(false);
            }
            paths.push(p);
        }
        self.record(&m, &paths, true)?;
        Ok(true)
    }

    /// Deletes an installed model's files (and any partial download) and forgets it. Returns the bytes freed, or
    /// `None` when the model was not installed.
    pub fn remove(&self, id: &str) -> Result<Option<u64>> {
        let m = self.manifest(id)?.clone();
        let _busy = Busy::take(&self.root, &m.id)?;
        let _lock = index_lock();
        let mut index = self.read_index();
        let dir = self.model_dir(&m);
        let mut freed = 0u64;
        let mut found = false;
        let mut keep = Vec::with_capacity(index.models.len());
        for rec in index.models.drain(..) {
            if rec.id != m.id {
                keep.push(rec);
                continue;
            }
            found = true;
            for f in &rec.files {
                let p = self.resolve(&f.path);
                if let Ok(md) = std::fs::metadata(&p) {
                    std::fs::remove_file(&p).map_err(|e| io_err("delete", &p, e))?;
                    freed += md.len();
                }
            }
        }
        index.models = keep;
        // Partial downloads and the empty folders of this model.
        for f in &m.files {
            let part = part_path(&dir.join(&f.name));
            if let Ok(md) = std::fs::metadata(&part) {
                let _ = std::fs::remove_file(&part);
                freed += md.len();
            }
        }
        let _ = std::fs::remove_dir(&dir);
        let _ = std::fs::remove_dir(self.root.join(&m.id));
        if found {
            self.write_index(&index)?;
        }
        Ok(found.then_some(freed))
    }

    /// Marks a model as used by a running job until the lease is dropped. [`Store::remove_when_free`] waits for it.
    /// Leases are seen within this process only.
    pub fn lease(&self, id: &str) -> Lease {
        let key = (self.root.clone(), id.to_string());
        let mut g = leases();
        match g.iter_mut().find(|(k, _)| *k == key) {
            Some((_, n)) => *n += 1,
            None => g.push((key.clone(), 1)),
        }
        Lease { key }
    }

    /// True while a job holds a lease on the model.
    pub fn in_use(&self, id: &str) -> bool {
        let key = (self.root.clone(), id.to_string());
        leases().iter().any(|(k, n)| *k == key && *n > 0)
    }

    /// Removes a model once no running job uses it. `on_wait` is called once, before waiting, when it is in use.
    pub fn remove_when_free(&self, id: &str, on_wait: &mut dyn FnMut()) -> Result<Option<u64>> {
        self.manifest(id)?;
        let key = (self.root.clone(), id.to_string());
        let mut g = leases();
        let mut told = false;
        while g.iter().any(|(k, n)| *k == key && *n > 0) {
            if !told {
                told = true;
                on_wait();
            }
            g = LEASE_FREED.wait(g).unwrap_or_else(|e| e.into_inner());
        }
        drop(g);
        self.remove(id)
    }

    fn record(&self, m: &Manifest, paths: &[PathBuf], adopted: bool) -> Result<()> {
        let _lock = index_lock();
        let mut index = self.read_index();
        index.models.retain(|r| r.id != m.id);
        index.models.push(Record {
            id: m.id.clone(),
            version: m.version.clone(),
            files: m
                .files
                .iter()
                .zip(paths)
                .map(|(f, p)| RecordFile {
                    name: f.name.clone(),
                    path: self.relative(p),
                    sha256: f.sha256.clone(),
                    bytes: f.bytes,
                })
                .collect(),
            installed_at_ms: now_ms(),
            adopted,
        });
        index.models.sort_by(|a, b| a.id.cmp(&b.id));
        self.write_index(&index)
    }

    fn resolve(&self, stored: &str) -> PathBuf {
        let p = Path::new(stored);
        if p.is_absolute() { p.to_path_buf() } else { self.root.join(stored) }
    }

    fn relative(&self, p: &Path) -> String {
        match p.strip_prefix(&self.root) {
            Ok(rel) => rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"),
            Err(_) => p.to_string_lossy().into_owned(),
        }
    }

    fn read_index(&self) -> Index {
        std::fs::read(self.root.join(INDEX_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice::<Index>(&b).ok())
            .unwrap_or(Index { version: 1, models: Vec::new() })
    }

    fn write_index(&self, index: &Index) -> Result<()> {
        std::fs::create_dir_all(&self.root).map_err(|e| io_err("create", &self.root, e))?;
        let path = self.root.join(INDEX_FILE);
        let tmp = self.root.join(format!("{INDEX_FILE}.tmp"));
        let body = serde_json::to_vec_pretty(&Index { version: 1, models: index.models.clone() })
            .map_err(|e| ModelError::Io(format!("could not encode {INDEX_FILE}: {e}")))?;
        std::fs::write(&tmp, body).map_err(|e| io_err("write", &tmp, e))?;
        std::fs::rename(&tmp, &path).map_err(|e| io_err("write", &path, e))
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Serialises read-modify-write of `installed.json` inside this process.
fn index_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Marks a model as being downloaded or removed in this process.
struct Busy {
    key: (PathBuf, String),
}

fn active() -> MutexGuard<'static, Vec<(PathBuf, String)>> {
    static ACTIVE: Mutex<Vec<(PathBuf, String)>> = Mutex::new(Vec::new());
    ACTIVE.lock().unwrap_or_else(|e| e.into_inner())
}

impl Busy {
    fn take(root: &Path, id: &str) -> Result<Busy> {
        let key = (root.to_path_buf(), id.to_string());
        let mut set = active();
        if set.contains(&key) {
            return Err(ModelError::Busy(id.into()));
        }
        set.push(key.clone());
        Ok(Busy { key })
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        active().retain(|k| k != &self.key);
    }
}

type LeaseKey = (PathBuf, String);

static LEASES: Mutex<Vec<(LeaseKey, usize)>> = Mutex::new(Vec::new());
static LEASE_FREED: Condvar = Condvar::new();

fn leases() -> MutexGuard<'static, Vec<(LeaseKey, usize)>> {
    LEASES.lock().unwrap_or_else(|e| e.into_inner())
}

/// A running job's hold on a model (see [`Store::lease`]).
#[derive(Debug)]
pub struct Lease {
    key: LeaseKey,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut g = leases();
        if let Some(i) = g.iter().position(|(k, _)| *k == self.key) {
            g[i].1 -= 1;
            if g[i].1 == 0 {
                g.swap_remove(i);
            }
        }
        LEASE_FREED.notify_all();
    }
}
