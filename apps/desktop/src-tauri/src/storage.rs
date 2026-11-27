//! Settings → Storage rows for derived data (add-background-tasks-and-media-derivatives, task 6.3): the size of each
//! cache kind in the open project and across the library, and a Clear action per kind for the open project.
//!
//! Clearing cancels the kind's running and queued tasks first, then removes the cache files. It removes only regular
//! files inside the kind's own folder of the bundle: never a media file (even one that sits inside that folder),
//! never the project document or its revisions, and never anything a link points to. Cleared data is derived, so
//! it is rebuilt when needed: thumbnails, skim sheets and waveforms on the next media check, proxies while "Use
//! proxies" is on, transcripts the next time captions or a transcript are asked for.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ove_store::Listing;
use ove_tasks::TaskFilter;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};

use crate::AppState;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::projects::{lock, serial, with_library};

/// A kind of derived data: its id, its name in Settings, its folder in the bundle and the task kinds that write it.
pub struct Kind {
    pub id: &'static str,
    pub label: &'static str,
    pub dir: &'static [&'static str],
    pub tasks: &'static [&'static str],
}

pub const KINDS: [Kind; 5] = [
    Kind { id: "thumbnails", label: "Thumbnails", dir: &["cache", "thumbs"], tasks: &["thumbnail"] },
    Kind { id: "skim", label: "Skim sheets", dir: &["cache", "skim"], tasks: &["skim"] },
    Kind { id: "proxies", label: "Proxies", dir: &["proxies"], tasks: &["proxy"] },
    Kind {
        id: "transcripts",
        label: "Transcripts",
        dir: &["cache", "transcripts"],
        tasks: &["captions", "transcribe"],
    },
    Kind { id: "waveforms", label: "Audio peak cache", dir: &["cache", "waveforms"], tasks: &["audio-analysis"] },
];

impl Kind {
    pub fn by_id(id: &str) -> Option<&'static Kind> {
        KINDS.iter().find(|k| k.id == id)
    }

    pub fn folder(&self, bundle: &Path) -> PathBuf {
        self.dir.iter().fold(bundle.to_path_buf(), |p, d| p.join(d))
    }
}

/// One Storage row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CacheRow {
    pub kind: String,
    pub label: String,
    /// Bytes in the open project (0 without one).
    pub project_bytes: f64,
    /// Bytes across every project in the library.
    pub total_bytes: f64,
}

/// The Storage rows, and the open project's "Use proxies" setting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StorageView {
    /// The open project's name, if one is open.
    pub project: Option<String>,
    pub rows: Vec<CacheRow>,
    pub use_proxies: bool,
}

/// What a clear did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ClearResult {
    pub freed_bytes: f64,
    pub cancelled_tasks: u32,
    /// Files that could not be removed (in use); they go on the next clear.
    pub left: u32,
    pub view: StorageView,
}

/// Bytes of the regular files under `dir` (links are not followed).
pub fn folder_size(dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => folder_size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

fn canonical(p: &Path) -> PathBuf {
    dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Removes the regular files under `dir`, except those in `keep` (canonical paths). Links are neither followed nor
/// removed. Returns (bytes freed, files left because they could not be removed).
pub fn clear_folder(dir: &Path, keep: &BTreeSet<PathBuf>) -> (u64, u32) {
    let Ok(rd) = std::fs::read_dir(dir) else { return (0, 0) };
    let (mut freed, mut left) = (0u64, 0u32);
    for e in rd.flatten() {
        let path = e.path();
        match e.file_type() {
            Ok(t) if t.is_dir() => {
                let (f, l) = clear_folder(&path, keep);
                freed += f;
                left += l;
                let _ = std::fs::remove_dir(&path);
            }
            Ok(t) if t.is_file() => {
                if keep.contains(&canonical(&path)) {
                    continue;
                }
                let len = e.metadata().map(|m| m.len()).unwrap_or(0);
                // A file still open elsewhere (Windows) may take a moment to be released.
                let mut removed = false;
                for attempt in 0..5 {
                    if std::fs::remove_file(&path).is_ok() {
                        removed = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(40 * (attempt + 1)));
                }
                if removed {
                    freed += len;
                } else {
                    left += 1;
                }
            }
            _ => {}
        }
    }
    (freed, left)
}

/// The media files of a document (canonical paths), which a clear never removes.
pub fn media_files(bundle: &Path, doc: &ove_core::Document) -> BTreeSet<PathBuf> {
    doc.registry.values().map(|e| canonical(&ove_store::media::resolve_media_path(bundle, &e.native_path))).collect()
}

fn library_bundles(state: &AppState) -> Vec<PathBuf> {
    with_library(&state.projects, |lib| Ok(lib.list()?))
        .map(|rows| {
            rows.into_iter()
                .map(|r| match r {
                    Listing::Ok { bundle, .. } | Listing::Damaged { bundle, .. } => bundle,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn usage(state: &AppState) -> StorageView {
    let open = lock(&state.projects.open).ok().and_then(|g| {
        g.as_ref()
            .map(|p| (p.bundle().to_path_buf(), p.doc().project.name.clone(), ove_render::proxy::enabled(p.doc())))
    });
    let mut bundles = library_bundles(state);
    if let Some((b, _, _)) = &open
        && !bundles.iter().any(|x| canonical(x) == canonical(b))
    {
        bundles.push(b.clone());
    }
    let rows = KINDS
        .iter()
        .map(|k| CacheRow {
            kind: k.id.into(),
            label: k.label.into(),
            project_bytes: open.as_ref().map_or(0, |(b, _, _)| folder_size(&k.folder(b))) as f64,
            total_bytes: bundles.iter().map(|b| folder_size(&k.folder(b))).sum::<u64>() as f64,
        })
        .collect();
    StorageView {
        project: open.as_ref().map(|(_, n, _)| n.clone()),
        rows,
        use_proxies: open.is_some_and(|(_, _, on)| on),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn storage_usage(app: AppHandle) -> AppResult<StorageView> {
    crate::worker::blocking("storage_usage", move || Ok(usage(&app.state::<AppState>()))).await
}

/// Clears one kind of derived data of the open project: cancels its tasks, then removes its files.
#[tauri::command]
#[specta::specta]
pub async fn clear_cache(app: AppHandle, kind: String) -> AppResult<ClearResult> {
    let k = Kind::by_id(&kind)
        .ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, format!("“{kind}” is not a kind of stored data.")))?;
    let r = serial(&app, "clear_cache", move |app, state| {
        let (bundle, doc) = {
            let g = lock(&state.projects.open)?;
            let p = g.as_ref().ok_or_else(|| AppError::new(ErrorCode::NotFound, "No project is open."))?;
            (p.bundle().to_path_buf(), p.doc().clone())
        };
        // Cancel first: a running task would write the files again.
        let project = crate::tasks::project_key(&bundle);
        let mut cancelled = 0u32;
        for t in k.tasks {
            let filter = TaskFilter { project: Some(project.clone()), kind: Some((*t).to_string()) };
            cancelled += state.tasks.cancel_matching(&filter) as u32;
            state.tasks.wait(&filter, Duration::from_secs(2));
        }
        // Playback lets go of proxy files before they are removed (Windows refuses to delete an open file).
        if k.id == "proxies" && ove_render::proxy::enabled(&doc) {
            let mut without = doc.clone();
            without.project.extra.shift_remove(ove_render::proxy::SETTING);
            crate::exporting::notify_doc(state, &without, &bundle);
            std::thread::sleep(Duration::from_millis(150));
        }
        let (freed, left) = clear_folder(&k.folder(&bundle), &media_files(&bundle, &doc));
        tracing::info!(kind = k.id, freed, left, cancelled, "cleared derived data");
        // Forget what was derived, so the next media check finds what is missing and rebuilds it.
        relock_media(state);
        if k.id == "proxies" {
            app.state::<crate::proxies::Proxies>().forget(&bundle);
        }
        Ok((freed, left, cancelled))
    })
    .await?;
    crate::projects::refresh_media(&app);
    if let Ok(g) = lock(&app.state::<AppState>().projects.open) {
        crate::projects::emit(&app, g.as_ref());
    }
    let view = usage(&app.state::<AppState>());
    Ok(ClearResult { freed_bytes: r.0 as f64, left: r.1, cancelled_tasks: r.2, view })
}

fn relock_media(state: &AppState) {
    crate::worker::relock(&state.projects.media).forget();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Clearing removes the kind's files only: media inside the folder, the document, revisions and other kinds stay.
    #[test]
    fn clearing_never_touches_media_or_the_document() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("P.ove");
        for k in &KINDS {
            std::fs::create_dir_all(k.folder(&bundle)).unwrap();
            std::fs::write(k.folder(&bundle).join("a.bin"), vec![1u8; 1000]).unwrap();
        }
        std::fs::create_dir_all(bundle.join(".revs")).unwrap();
        std::fs::write(bundle.join("project.json"), b"{}").unwrap();
        std::fs::write(bundle.join(".revs/1.json"), b"{}").unwrap();
        // A media file someone keeps inside the proxies folder, and a nested folder of cache files.
        let media = bundle.join("proxies/interview.mov");
        std::fs::write(&media, vec![2u8; 500]).unwrap();
        std::fs::create_dir_all(bundle.join("proxies/old")).unwrap();
        std::fs::write(bundle.join("proxies/old/b.bin"), vec![3u8; 200]).unwrap();
        let proxies = Kind::by_id("proxies").unwrap();
        assert_eq!(folder_size(&proxies.folder(&bundle)), 1700);
        let keep: BTreeSet<PathBuf> = [canonical(&media)].into();
        let (freed, left) = clear_folder(&proxies.folder(&bundle), &keep);
        assert_eq!((freed, left), (1200, 0));
        assert!(media.is_file(), "media is never removed");
        assert!(bundle.join("project.json").is_file() && bundle.join(".revs/1.json").is_file());
        assert!(!bundle.join("proxies/old").exists());
        for k in KINDS.iter().filter(|k| k.id != "proxies") {
            assert_eq!(folder_size(&k.folder(&bundle)), 1000, "{} untouched", k.id);
        }
        assert_eq!(folder_size(&proxies.folder(&bundle)), 500);
    }

    #[test]
    fn every_kind_has_its_own_folder() {
        let b = Path::new("P.ove");
        let folders: BTreeSet<PathBuf> = KINDS.iter().map(|k| k.folder(b)).collect();
        assert_eq!(folders.len(), KINDS.len());
        assert!(KINDS.iter().all(|k| k.folder(b).starts_with(b) && k.folder(b) != b));
        assert_eq!(Kind::by_id("waveforms").unwrap().label, "Audio peak cache");
        assert!(Kind::by_id("project").is_none());
    }
}
