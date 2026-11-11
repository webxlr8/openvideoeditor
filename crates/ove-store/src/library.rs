//! The project library: location (spec: Project library location), listing via `cache/index.json` (design D9),
//! create, rename, duplicate and move to Trash / Recycle Bin.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ove_core::migrate::Opened;
use ove_core::model::Document;
use ove_core::ops::Op;
use ove_core::revision::{Clock, Source};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::bundle::{is_bundle, unique_bundle_path};
use crate::error::{Result, StoreError};
use crate::project::{Mode, Project, load_current};

/// Working app name used for the default library folder.
pub const APP_DIR_NAME: &str = "OpenVideoEditor";
/// Environment variable overriding the library location.
pub const LIBRARY_ENV: &str = "OVE_LIBRARY";

/// Platform default: `~/Movies/<App>/` on macOS, `%USERPROFILE%\Videos\<App>\` on Windows (`~/Videos/<App>` on
/// other systems, for development).
pub fn default_library() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "macos") {
        home.join("Movies").join(APP_DIR_NAME)
    } else {
        home.join("Videos").join(APP_DIR_NAME)
    }
}

/// Resolves the library: environment variable, then the settings value, then the platform default. Creates the
/// folder and refuses network volumes.
pub fn resolve_library(setting: Option<&Path>) -> Result<PathBuf> {
    let path = std::env::var_os(LIBRARY_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| setting.map(Path::to_path_buf))
        .unwrap_or_else(default_library);
    validate_library(&path)?;
    Ok(path)
}

/// Checks a library location: creates it if missing and refuses network volumes.
pub fn validate_library(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).map_err(|e| StoreError::io("create the library folder", path, e))?;
    if is_network_volume(path) {
        return Err(StoreError::refused(
            "library.network",
            format!(
                "{} is on a network drive. Projects must be on a local drive so they can be saved safely; media may still live on network drives.",
                path.display()
            ),
        ));
    }
    Ok(())
}

/// True when `path` is on a network file system (SMB/NFS/AFP/WebDAV…).
pub fn is_network_volume(path: &Path) -> bool {
    if std::env::var_os("OVE_TEST_TREAT_AS_NETWORK").is_some_and(|v| path.starts_with(PathBuf::from(v))) {
        return true;
    }
    imp::is_network(path)
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    pub fn is_network(path: &Path) -> bool {
        let Ok(c) = CString::new(path.as_os_str().as_bytes()) else { return false };
        let mut s: libc::statfs = unsafe { std::mem::zeroed() };
        // SAFETY: c is a valid NUL-terminated path; s is a valid out-pointer.
        if unsafe { libc::statfs(c.as_ptr(), &mut s) } != 0 {
            return false;
        }
        (s.f_flags & libc::MNT_LOCAL as u32) == 0
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    pub fn is_network(path: &Path) -> bool {
        let Ok(c) = CString::new(path.as_os_str().as_bytes()) else { return false };
        let mut s: libc::statfs = unsafe { std::mem::zeroed() };
        // SAFETY: c is a valid NUL-terminated path; s is a valid out-pointer.
        if unsafe { libc::statfs(c.as_ptr(), &mut s) } != 0 {
            return false;
        }
        // NFS, SMB, CIFS, SMB2, CODA, AFS, 9P, FUSE-based sshfs is not detectable here.
        const NETWORK: [i64; 7] = [0x6969, 0x517B, 0xFF534D42, 0xFE534D42, 0x73757245, 0x5346414F, 0x01021997];
        NETWORK.contains(&(s.f_type as i64))
    }
}

#[cfg(windows)]
mod imp {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Component, Path, Prefix};
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;

    const DRIVE_REMOTE: u32 = 4;

    pub fn is_network(path: &Path) -> bool {
        let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        match abs.components().next() {
            Some(Component::Prefix(p)) => match p.kind() {
                Prefix::UNC(..) | Prefix::VerbatimUNC(..) => true,
                Prefix::Disk(d) | Prefix::VerbatimDisk(d) => {
                    let root: Vec<u16> = format!("{}:\\", d as char).encode_utf16().chain([0]).collect();
                    // SAFETY: root is a valid NUL-terminated UTF-16 string.
                    unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_REMOTE }
                }
                _ => false,
            },
            _ => {
                let _ = abs.as_os_str().encode_wide();
                false
            }
        }
    }
}

/// One row of the library listing, also the content of `cache/index.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexEntry {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub aspect_ratio: String,
    /// Seconds.
    pub duration: f64,
    pub item_count: usize,
    pub rev: i64,
}

impl IndexEntry {
    pub fn from_doc(d: &Document) -> Self {
        IndexEntry {
            id: d.project.id.clone(),
            name: d.project.name.clone(),
            created_at: d.project.created_at,
            updated_at: d.project.updated_at,
            aspect_ratio: d.project.aspect_ratio.clone(),
            duration: d.duration() as f64 / 1e6,
            item_count: d.project.items.len(),
            rev: d.project.rev,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum Listing {
    Ok {
        bundle: PathBuf,
        #[serde(flatten)]
        entry: IndexEntry,
    },
    Damaged {
        bundle: PathBuf,
        reason: String,
    },
}

impl Listing {
    pub fn bundle(&self) -> &Path {
        match self {
            Listing::Ok { bundle, .. } | Listing::Damaged { bundle, .. } => bundle,
        }
    }
}

/// Newest revision record on disk (by file name), or `None`.
fn newest_record(bundle: &Path) -> Option<i64> {
    std::fs::read_dir(bundle.join(".revs"))
        .ok()?
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.strip_suffix(".json").filter(|s| !s.ends_with(".snap")).and_then(|s| s.parse::<i64>().ok())
        })
        .max()
}

fn list_one(bundle: &Path) -> Listing {
    let index_path = bundle.join("cache/index.json");
    let newest = newest_record(bundle).unwrap_or(i64::MIN);
    if let Some(entry) = std::fs::read(&index_path).ok().and_then(|b| serde_json::from_slice::<IndexEntry>(&b).ok())
        && entry.rev >= newest
    {
        return Listing::Ok { bundle: bundle.to_path_buf(), entry };
    }
    // Index missing or stale: parse the full document (with replay) and rewrite the index.
    match load_current(bundle) {
        Ok((opened, _, _)) => match opened.doc() {
            Some(doc) => {
                let entry = IndexEntry::from_doc(doc);
                let _ = std::fs::create_dir_all(bundle.join("cache"));
                let _ = crate::fsio::write_atomic(
                    &index_path,
                    &serde_json::to_vec_pretty(&entry).unwrap(),
                    crate::fsio::INDEX,
                    false,
                );
                Listing::Ok { bundle: bundle.to_path_buf(), entry }
            }
            None => {
                Listing::Damaged { bundle: bundle.to_path_buf(), reason: "saved by a newer version of the app".into() }
            }
        },
        Err(e) => Listing::Damaged { bundle: bundle.to_path_buf(), reason: e.to_string() },
    }
}

/// A project library folder.
#[derive(Clone)]
pub struct Library {
    pub root: PathBuf,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for Library {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Library").field("root", &self.root).finish()
    }
}

/// New random-ish project id (time + counter + pid), no external dependency.
pub fn new_id(prefix: &str, clock: &dyn Clock) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let mixed = (clock.now_ms() as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(n.wrapping_mul(0xBF58_476D_1CE4_E5B9))
        .wrapping_add(std::process::id() as u64);
    format!("{prefix}{:012x}", mixed & 0xFFFF_FFFF_FFFF)
}

impl Library {
    pub fn open(root: impl Into<PathBuf>, clock: Arc<dyn Clock>) -> Result<Library> {
        let root = root.into();
        validate_library(&root)?;
        Ok(Library { root, clock })
    }

    /// Creates a project bundle and opens it for writing.
    pub fn create(&self, name: &str) -> Result<Project> {
        let bundle = unique_bundle_path(&self.root, name);
        let id = new_id("p_", self.clock.as_ref());
        Project::create(&bundle, &id, name, self.clock.clone())
    }

    /// Lists every bundle without loading full documents (unless an index is missing or stale). Damaged bundles are
    /// listed with the reason.
    pub fn list(&self) -> Result<Vec<Listing>> {
        let rd = std::fs::read_dir(&self.root).map_err(|e| StoreError::io("read the library", &self.root, e))?;
        let mut bundles: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| is_bundle(p)).collect();
        bundles.sort();
        Ok(bundles.iter().map(|b| list_one(b)).collect())
    }

    /// Resolves a project by bundle path, id or name (exact, then case-insensitive).
    pub fn find(&self, query: &str) -> Result<PathBuf> {
        let as_path = PathBuf::from(query);
        if is_bundle(&as_path) {
            return Ok(as_path);
        }
        let rows = self.list()?;
        let ok = || {
            rows.iter().filter_map(|r| match r {
                Listing::Ok { bundle, entry } => Some((bundle, entry)),
                _ => None,
            })
        };
        if let Some((b, _)) = ok().find(|(_, e)| e.id == query) {
            return Ok(b.clone());
        }
        let by_name: Vec<_> = ok().filter(|(_, e)| e.name == query).collect();
        let by_name = if by_name.is_empty() {
            ok().filter(|(_, e)| e.name.eq_ignore_ascii_case(query)).collect()
        } else {
            by_name
        };
        match by_name.len() {
            1 => Ok(by_name[0].0.clone()),
            0 => Err(StoreError::refused("project.notFound", format!("no project named or with id \"{query}\""))),
            n => Err(StoreError::refused(
                "project.ambiguous",
                format!("{n} projects are named \"{query}\"; use the project id or bundle path"),
            )),
        }
    }

    /// Renames a project: the document name changes (as a revision); the bundle folder never does.
    pub fn rename(&self, bundle: &Path, name: &str, source: Source) -> Result<()> {
        let mut p = Project::open(bundle, Mode::Write, self.clock.clone())?;
        p.apply(
            vec![Op::UpdateProject { set: json!({"name": name}).as_object().unwrap().clone(), unset: vec![] }],
            source,
            Some(format!("rename to {name}")),
        )?;
        p.close()
    }

    /// Duplicates a project: new id, same content, media copied only when owned by the bundle. History starts fresh.
    pub fn duplicate(&self, bundle: &Path, new_name: Option<&str>) -> Result<PathBuf> {
        let (opened, _, _) = load_current(bundle)?;
        let mut doc = match opened {
            Opened::Current(d) | Opened::Migrated { doc: d, .. } => d,
            Opened::Newer { .. } => {
                return Err(StoreError::refused(
                    "project.newer",
                    "cannot duplicate a project saved by a newer version",
                ));
            }
        };
        let name = new_name.map(str::to_string).unwrap_or_else(|| format!("{} copy", doc.project.name));
        let dest = unique_bundle_path(&self.root, &name);
        let now = self.clock.now_ms();
        let id = new_id("p_", self.clock.as_ref());
        let mut p = Project::create(&dest, &id, &name, self.clock.clone())?;
        // Copy owned media files.
        for e in doc.registry.values().filter(|e| e.owned_by_bundle) {
            let src = bundle.join(&e.native_path);
            let dst = dest.join(&e.native_path);
            if let Some(parent) = dst.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if src.exists() {
                std::fs::copy(&src, &dst).map_err(|err| StoreError::io("copy media", &src, err))?;
            }
        }
        // Same content through the pipeline: diff the fresh document to the source content.
        doc.project.id = id;
        doc.project.name = name;
        doc.project.created_at = now;
        let ops = ove_core::diff::diff_documents(p.doc(), &doc);
        if !ops.is_empty() {
            p.apply(ops, Source::Gui, Some("duplicate".into()))?;
        }
        p.close()?;
        Ok(dest)
    }

    /// Moves a project bundle to the system Trash / Recycle Bin. Refused while another process has it open.
    pub fn trash(&self, bundle: &Path) -> Result<()> {
        // Taking the write lock proves nobody else is editing it.
        let lock = crate::lock::WriteLock::acquire(bundle)?;
        drop(lock);
        trash::delete(bundle).map_err(|e| {
            StoreError::refused("trash.failed", format!("could not move {} to the Trash: {e}", bundle.display()))
        })
    }
}
