//! An open project: write-ahead revision log + snapshot persistence (design D2), replay on open, retention and
//! periodic snapshots (D10), revision history and restore.
//!
//! Layout inside the bundle:
//! - `project.json` — the canonical snapshot, rewritten atomically at most once per second while edits continue
//!   and always on close.
//! - `.revs/<rev>.json` — one record per committed batch (forward ops, inverse ops, parent, time, source, summary).
//!   A batch is acknowledged only after its record is durable.
//! - `.revs/<rev>.snap.json` — full documents at the project's first revision and every 200 revisions, so that any
//!   retained revision can be reconstructed by replaying at most 200 records.
//! - `.revs/pre-migration-<version>.json` — the original file before a schema migration.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ove_core::migrate::{self, Opened};
use ove_core::model::Document;
use ove_core::ops::Op;
use ove_core::revision::{Clock, Prepared, RevisionRecord, Session, Source};
use ove_core::schema::Issue;

use crate::bundle::BUNDLE_DIRS;
use crate::error::{Result, StoreError};
use crate::fsio::{self, INDEX, RECORD, SNAPSHOT};
use crate::lock::WriteLock;

/// Periodic full snapshot interval, in revisions.
pub const SNAPSHOT_EVERY: usize = 200;
/// Retention: keep at least this many revisions…
pub const KEEP_REVISIONS: usize = 2000;
/// …or everything from the last 30 days, whichever keeps more.
pub const KEEP_MS: i64 = 30 * 24 * 3600 * 1000;
/// Minimum interval between snapshot rewrites while edits continue.
pub const SNAPSHOT_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Takes the write lock; edits allowed.
    Write,
    /// No lock; edits refused.
    ReadOnly,
}

/// Summary of one revision for history listings.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub rev: i64,
    pub parent: i64,
    pub time: i64,
    pub source: Source,
    pub summary: String,
    pub op_count: usize,
}

/// Result of a committed batch.
#[derive(Debug, Clone)]
pub struct Committed {
    pub record: RevisionRecord,
    pub warnings: Vec<Issue>,
}

/// An open project bundle.
pub struct Project {
    bundle: PathBuf,
    session: Session,
    mode: Mode,
    /// Declared before the lock so it is dropped (and joined) while the lock is still held.
    projection: Option<ProjectionWriter>,
    _lock: Option<WriteLock>,
    clock: Arc<dyn Clock>,
    /// Revision of the document in `project.json` on disk.
    snapshot_rev: i64,
    last_snapshot: Option<Instant>,
    /// Revisions of records on disk (ascending), with their commit time.
    records: BTreeMap<i64, i64>,
    /// Revisions of full snapshots in `.revs/` (ascending).
    snaps: Vec<i64>,
    /// Read-only reason when the document is newer than this build.
    pub read_only_reason: Option<String>,
    /// Lock takeover info for logging/UI.
    pub took_over_from: Option<crate::lock::Holder>,
    /// Version the document was migrated from on this open, if any.
    pub migrated_from: Option<u64>,
}

impl std::fmt::Debug for Project {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Project").field("bundle", &self.bundle).field("rev", &self.rev()).finish()
    }
}

fn revs_dir(bundle: &Path) -> PathBuf {
    bundle.join(".revs")
}

fn record_path(bundle: &Path, rev: i64) -> PathBuf {
    revs_dir(bundle).join(format!("{rev}.json"))
}

fn snap_path(bundle: &Path, rev: i64) -> PathBuf {
    revs_dir(bundle).join(format!("{rev}.snap.json"))
}

/// Lists record and snapshot revisions in `.revs/`.
fn scan_revs(bundle: &Path) -> (BTreeMap<i64, i64>, Vec<i64>) {
    let mut records = BTreeMap::new();
    let mut snaps = Vec::new();
    if let Ok(rd) = std::fs::read_dir(revs_dir(bundle)) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(stem) = name.strip_suffix(".snap.json") {
                if let Ok(r) = stem.parse() {
                    snaps.push(r);
                }
            } else if let Some(stem) = name.strip_suffix(".json")
                && let Ok(r) = stem.parse::<i64>()
            {
                records.insert(r, 0);
            }
        }
    }
    snaps.sort();
    (records, snaps)
}

pub fn read_record(bundle: &Path, rev: i64) -> Result<RevisionRecord> {
    let p = record_path(bundle, rev);
    let bytes = std::fs::read(&p).map_err(|e| StoreError::io("read revision", &p, e))?;
    serde_json::from_slice(&bytes).map_err(|e| {
        StoreError::refused("revision.invalid", format!("revision record {} is damaged: {e}", p.display()))
    })
}

fn read_doc_file(path: &Path) -> Result<Opened> {
    let text = std::fs::read_to_string(path).map_err(|e| StoreError::io("read", path, e))?;
    migrate::open_json(&text).map_err(|source| StoreError::Invalid { path: path.display().to_string(), source })
}

/// Reads `project.json` and replays newer records: the current document, without locking.
pub fn load_current(bundle: &Path) -> Result<(Opened, BTreeMap<i64, i64>, Vec<i64>)> {
    let opened = read_doc_file(&bundle.join("project.json"))?;
    let (mut records, snaps) = scan_revs(bundle);
    let opened = match opened {
        Opened::Current(doc) => Opened::Current(replay(bundle, doc, &mut records)?),
        Opened::Migrated { doc, from_version } => {
            Opened::Migrated { doc: replay(bundle, doc, &mut records)?, from_version }
        }
        newer => newer,
    };
    Ok((opened, records, snaps))
}

/// Applies records with rev greater than the document's, in order, while they chain from it.
fn replay(bundle: &Path, mut doc: Document, records: &mut BTreeMap<i64, i64>) -> Result<Document> {
    let pending: Vec<i64> = records.range(doc.project.rev + 1..).map(|(r, _)| *r).collect();
    for rev in pending {
        let rec = match read_record(bundle, rev) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(rev, error = %e, "stopping replay at a damaged revision record");
                break;
            }
        };
        if rec.parent != doc.project.rev {
            tracing::error!(rev, parent = rec.parent, at = doc.project.rev, "revision does not chain; replay stopped");
            break;
        }
        let applied = ove_core::apply_batch(&doc, &rec.ops)?;
        doc = applied.doc;
        doc.project.rev = rec.rev;
        doc.project.updated_at = rec.time;
        records.insert(rev, rec.time);
    }
    Ok(doc)
}

impl Project {
    /// Creates a bundle folder with the standard layout and an empty document, and opens it for writing.
    pub fn create(bundle: &Path, id: &str, name: &str, clock: Arc<dyn Clock>) -> Result<Project> {
        std::fs::create_dir_all(bundle).map_err(|e| StoreError::io("create", bundle, e))?;
        for d in BUNDLE_DIRS {
            let p = bundle.join(d);
            std::fs::create_dir_all(&p).map_err(|e| StoreError::io("create", &p, e))?;
        }
        let doc = Document::new(id, name, clock.now_ms());
        let text = doc.to_string_pretty();
        fsio::write_atomic(&snap_path(bundle, doc.project.rev), text.as_bytes(), SNAPSHOT, true)
            .map_err(|e| StoreError::io("write", bundle, e))?;
        fsio::write_atomic(&bundle.join("project.json"), text.as_bytes(), SNAPSHOT, true)
            .map_err(|e| StoreError::io("write", &bundle.join("project.json"), e))?;
        let p = Project::open(bundle, Mode::Write, clock)?;
        p.write_index()?;
        Ok(p)
    }

    /// Opens a bundle. `Write` takes the single-writer lock; a document newer than this build opens read-only.
    pub fn open(bundle: &Path, mode: Mode, clock: Arc<dyn Clock>) -> Result<Project> {
        let lock = match mode {
            Mode::Write => Some(WriteLock::acquire(bundle)?),
            Mode::ReadOnly => None,
        };
        if mode == Mode::Write {
            fsio::clean_temp_files(&revs_dir(bundle));
            fsio::clean_temp_files(bundle);
        }
        let (opened, records, snaps) = load_current(bundle)?;
        let took_over_from = lock.as_ref().and_then(|l| l.took_over_from.clone());
        let (doc, mut mode, reason, migrated_from) = match opened {
            Opened::Current(d) => (d, mode, None, None),
            Opened::Migrated { doc, from_version } => (doc, mode, None, Some(from_version)),
            Opened::Newer { doc, version } => {
                let doc = doc.ok_or_else(|| {
                    StoreError::refused(
                        "project.newer",
                        format!("this project was saved by a newer version of the app (format {version}); update the app to open it"),
                    )
                })?;
                (
                    doc,
                    Mode::ReadOnly,
                    Some(format!(
                        "this project was saved by a newer version of the app (format {version}); it is open read-only — update the app to edit it"
                    )),
                    None,
                )
            }
        };
        if reason.is_some() {
            mode = Mode::ReadOnly;
        }
        let snapshot_rev = doc.project.rev;
        let mut p = Project {
            bundle: bundle.to_path_buf(),
            session: Session::new(doc),
            mode,
            projection: (mode == Mode::Write).then(|| ProjectionWriter::spawn(bundle.to_path_buf())),
            _lock: if mode == Mode::Write { lock } else { None },
            clock,
            snapshot_rev,
            last_snapshot: None,
            records,
            snaps,
            read_only_reason: reason,
            took_over_from,
            migrated_from,
        };
        if let Some(from) = migrated_from
            && mode == Mode::Write
        {
            p.finish_migration(from)?;
        }
        // Replayed records are folded into the snapshot right away so project.json is current.
        if p.mode == Mode::Write && p.rev() != snapshot_on_disk(bundle) {
            p.snapshot_now()?;
        }
        if p.mode == Mode::Write && projection_rev_on_disk(bundle) != Some(p.rev()) {
            p.write_projection();
        }
        Ok(p)
    }

    fn finish_migration(&mut self, from: u64) -> Result<()> {
        let src = self.bundle.join("project.json");
        let copy = revs_dir(&self.bundle).join(format!("pre-migration-{from}.json"));
        if !copy.exists() {
            let bytes = std::fs::read(&src).map_err(|e| StoreError::io("read", &src, e))?;
            fsio::write_atomic(&copy, &bytes, SNAPSHOT, true).map_err(|e| StoreError::io("write", &copy, e))?;
        }
        // The migrated document becomes the new baseline: snapshot + base snap for history.
        self.snapshot_now()?;
        let rev = self.rev();
        let text = self.session.doc().to_string_pretty();
        fsio::write_atomic(&snap_path(&self.bundle, rev), text.as_bytes(), SNAPSHOT, true)
            .map_err(|e| StoreError::io("write", &snap_path(&self.bundle, rev), e))?;
        if !self.snaps.contains(&rev) {
            self.snaps.push(rev);
            self.snaps.sort();
        }
        tracing::info!(bundle = %self.bundle.display(), from, "migrated project document");
        Ok(())
    }

    pub fn bundle(&self) -> &Path {
        &self.bundle
    }

    pub fn doc(&self) -> &Document {
        self.session.doc()
    }

    pub fn rev(&self) -> i64 {
        self.session.rev()
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    fn ensure_writable(&self) -> Result<()> {
        if self.mode == Mode::Write {
            Ok(())
        } else {
            Err(StoreError::refused(
                "project.readOnly",
                self.read_only_reason.clone().unwrap_or_else(|| "the project is open read-only".into()),
            ))
        }
    }

    /// Validates, durably records and commits a batch. Returns only after the record is on disk.
    pub fn apply(&mut self, ops: Vec<Op>, source: Source, summary: Option<String>) -> Result<Committed> {
        self.ensure_writable()?;
        let prepared = self.session.prepare(ops, source, summary, self.clock.as_ref())?;
        self.commit_prepared(prepared)
    }

    fn commit_prepared(&mut self, prepared: Prepared) -> Result<Committed> {
        let path = record_path(&self.bundle, prepared.record.rev);
        let bytes = serde_json::to_vec(&prepared.record).expect("serialisable record");
        fsio::write_atomic(&path, &bytes, RECORD, false).map_err(|e| StoreError::io("save the change to", &path, e))?;
        let warnings = prepared.warnings.clone();
        let record = self.session.commit(prepared);
        self.records.insert(record.rev, record.time);
        self.after_commit();
        Ok(Committed { record, warnings })
    }

    fn after_commit(&mut self) {
        // The projection follows every revision (derived data: atomic rename, no flush).
        self.write_projection();
        // Periodic full snapshot for bounded reconstruction.
        let last_snap = self.snaps.last().copied().unwrap_or(i64::MIN);
        if self.records.range(last_snap + 1..).count() >= SNAPSHOT_EVERY {
            let rev = self.rev();
            let text = self.session.doc().to_string_pretty();
            match fsio::write_atomic(&snap_path(&self.bundle, rev), text.as_bytes(), SNAPSHOT, false) {
                Ok(()) => self.snaps.push(rev),
                Err(e) => tracing::error!(error = %e, "periodic snapshot failed; will retry"),
            }
        }
        // project.json at most once per second while edits continue.
        if self.last_snapshot.is_none_or(|t| t.elapsed() >= SNAPSHOT_INTERVAL)
            && let Err(e) = self.snapshot_now()
        {
            tracing::error!(error = %e, "snapshot failed; the revision log still holds every change");
        }
    }

    /// Rewrites `project.json` and the library index now.
    pub fn snapshot_now(&mut self) -> Result<()> {
        if self.mode != Mode::Write {
            return Ok(());
        }
        let path = self.bundle.join("project.json");
        let text = self.session.doc().to_string_pretty();
        fsio::write_atomic(&path, text.as_bytes(), SNAPSHOT, false).map_err(|e| StoreError::io("write", &path, e))?;
        self.snapshot_rev = self.rev();
        self.last_snapshot = Some(Instant::now());
        self.write_index()
    }

    /// True when `project.json` lags the latest revision.
    pub fn snapshot_pending(&self) -> bool {
        self.snapshot_rev != self.rev()
    }

    fn write_index(&self) -> Result<()> {
        let path = self.bundle.join("cache/index.json");
        let entry = crate::library::IndexEntry::from_doc(self.session.doc());
        let bytes = serde_json::to_vec_pretty(&entry).unwrap();
        fsio::write_atomic(&path, &bytes, INDEX, false).map_err(|e| StoreError::io("write", &path, e))
    }

    /// Undo the most recent batch of this session. `Ok(None)` when there is nothing to undo.
    pub fn undo(&mut self) -> Result<Option<Committed>> {
        self.ensure_writable()?;
        match self.session.prepare_undo(self.clock.as_ref()) {
            None => Ok(None),
            Some(p) => Ok(Some(self.commit_prepared(p?)?)),
        }
    }

    /// Redo the most recently undone batch of this session.
    pub fn redo(&mut self) -> Result<Option<Committed>> {
        self.ensure_writable()?;
        match self.session.prepare_redo(self.clock.as_ref()) {
            None => Ok(None),
            Some(p) => Ok(Some(self.commit_prepared(p?)?)),
        }
    }

    /// Restores the document to revision `rev` as a new revision (source `restore`, or the given source).
    /// `Ok(None)` when the content already equals that revision.
    pub fn restore(&mut self, rev: i64) -> Result<Option<Committed>> {
        self.restore_as(rev, Source::Restore)
    }

    pub fn restore_as(&mut self, rev: i64, source: Source) -> Result<Option<Committed>> {
        self.restore_with(rev, source, None)
    }

    /// [`Project::restore_as`] with the revision's summary (`None`: one naming the target revision).
    pub fn restore_with(&mut self, rev: i64, source: Source, summary: Option<String>) -> Result<Option<Committed>> {
        self.ensure_writable()?;
        let target = self.doc_at(rev)?;
        let ops = ove_core::diff::diff_documents(self.session.doc(), &target);
        if ops.is_empty() {
            return Ok(None);
        }
        let summary = summary.unwrap_or_else(|| match source {
            Source::Restore => format!("restore revision {rev}"),
            Source::Undo => format!("undo to revision {rev}"),
            Source::Redo => format!("redo to revision {rev}"),
            _ => format!("return to revision {rev}"),
        });
        let p = self.session.prepare(ops, source, Some(summary), self.clock.as_ref())?;
        Ok(Some(self.commit_prepared(p)?))
    }

    /// Revisions on disk, oldest first.
    pub fn history(&self) -> Result<Vec<HistoryEntry>> {
        let mut out = Vec::new();
        for rev in self.records.keys() {
            if *rev > self.rev() {
                continue;
            }
            let r = read_record(&self.bundle, *rev)?;
            out.push(HistoryEntry {
                rev: r.rev,
                parent: r.parent,
                time: r.time,
                source: r.source,
                summary: r.summary,
                op_count: r.ops.len(),
            });
        }
        Ok(out)
    }

    /// The latest revision record, if any is retained.
    pub fn latest_record(&self) -> Result<Option<RevisionRecord>> {
        match self.records.keys().next_back() {
            Some(r) => Ok(Some(read_record(&self.bundle, *r)?)),
            None => Ok(None),
        }
    }

    /// True when the document at `rev` can be reconstructed.
    pub fn is_retained(&self, rev: i64) -> bool {
        rev == self.rev()
            || self.snaps.contains(&rev)
            || (self.records.contains_key(&rev) && self.snaps.first().is_some_and(|s| *s <= rev))
    }

    /// Reconstructs the document at revision `rev` from the nearest snapshot at or before it plus forward records.
    pub fn doc_at(&self, rev: i64) -> Result<Document> {
        if rev == self.rev() {
            return Ok(self.session.doc().clone());
        }
        if rev > self.rev() {
            return Err(StoreError::refused(
                "revision.newer",
                format!("revision {rev} is newer than the current revision {}", self.rev()),
            ));
        }
        if !self.is_retained(rev) {
            return Err(StoreError::refused(
                "revision.missing",
                format!("revision {rev} is not available (it may have been pruned)"),
            ));
        }
        let base = *self.snaps.iter().rev().find(|s| **s <= rev).expect("retained implies a snapshot");
        let p = snap_path(&self.bundle, base);
        let mut doc = match read_doc_file(&p)? {
            Opened::Current(d) | Opened::Migrated { doc: d, .. } => d,
            Opened::Newer { .. } => {
                return Err(StoreError::refused("revision.invalid", "snapshot is from a newer version"));
            }
        };
        let replay: Vec<i64> =
            if base < rev { self.records.range(base + 1..=rev).map(|(r, _)| *r).collect() } else { Vec::new() };
        for r in replay {
            let rec = read_record(&self.bundle, r)?;
            doc = ove_core::apply_batch(&doc, &rec.ops)?.doc;
            doc.project.rev = rec.rev;
            doc.project.updated_at = rec.time;
        }
        Ok(doc)
    }

    /// Prunes revisions outside retention (keeps the last 2,000 or the last 30 days, whichever keeps more), keeping
    /// the snapshot needed to reconstruct the oldest retained revision. Runs on close.
    pub fn prune(&mut self) -> Result<usize> {
        self.ensure_writable()?;
        let now = self.clock.now_ms();
        let revs: Vec<i64> = self.records.keys().copied().collect();
        if revs.len() <= KEEP_REVISIONS {
            return Ok(0);
        }
        let by_count = revs[revs.len() - KEEP_REVISIONS];
        // Record times: rev is ms-based, so it is a lower bound of the commit time.
        let by_age = revs.iter().copied().find(|r| self.records[r].max(*r) >= now - KEEP_MS).unwrap_or(i64::MAX);
        let oldest_kept = by_count.min(by_age);
        // Cut at the newest full snapshot at or before the oldest revision that must stay reconstructable; records
        // after it are kept (so retention may keep slightly more than the minimum, never less).
        let Some(keep_snap) = self.snaps.iter().rev().find(|s| **s <= oldest_kept).copied() else {
            return Ok(0);
        };
        // Full flush of the current snapshot before deleting anything.
        self.snapshot_now()?;
        let mut removed = 0;
        for r in revs.into_iter().filter(|r| *r <= keep_snap) {
            if std::fs::remove_file(record_path(&self.bundle, r)).is_ok() {
                removed += 1;
            }
            self.records.remove(&r);
        }
        let snaps = std::mem::take(&mut self.snaps);
        for s in snaps {
            if s < keep_snap {
                let _ = std::fs::remove_file(snap_path(&self.bundle, s));
            } else {
                self.snaps.push(s);
            }
        }
        Ok(removed)
    }

    /// Writes the final snapshot with a full flush, prunes, and releases the lock.
    pub fn close(mut self) -> Result<()> {
        if self.mode == Mode::Write {
            self.snapshot_now()?;
            let f = std::fs::File::open(self.bundle.join("project.json"))
                .map_err(|e| StoreError::io("open", &self.bundle, e))?;
            fsio::flush_full(&f).map_err(|e| StoreError::io("flush", &self.bundle, e))?;
            self.prune()?;
        }
        Ok(())
    }
}

/// File names of the projection and its typings inside the bundle.
pub const PROJECTION_FILE: &str = "timeline.tsx";
pub const TYPINGS_FILE: &str = "timeline.d.ts";
const DIAGNOSTICS_FILE: &str = "cache/diagnostics.json";

fn projection_rev_on_disk(bundle: &Path) -> Option<i64> {
    let text = std::fs::read_to_string(bundle.join(PROJECTION_FILE)).ok()?;
    ove_projection::parse::header_rev(&text)?.ok()
}

impl Project {
    /// Header context for printing: diagnostics from the last projection apply and offline media.
    pub fn print_context(&self) -> ove_projection::PrintContext {
        let diagnostics = std::fs::read(self.bundle.join(DIAGNOSTICS_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
            .unwrap_or_default();
        let offline = crate::media::media_status(&self.bundle, self.doc())
            .into_iter()
            .filter(|m| m.state == crate::media::MediaState::Offline)
            .map(|m| m.media_id)
            .collect();
        ove_projection::PrintContext { diagnostics, offline }
    }

    /// The projection text of the current revision (printed now, in the calling thread).
    pub fn projection(&self) -> String {
        ove_projection::print(self.doc(), &self.print_context())
    }

    /// Records the warnings listed in the next printed header (empty clears them).
    pub fn set_diagnostics(&mut self, lines: Vec<String>) -> Result<()> {
        let path = self.bundle.join(DIAGNOSTICS_FILE);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        fsio::write_atomic(&path, &serde_json::to_vec(&lines).unwrap(), INDEX, false)
            .map_err(|e| StoreError::io("write", &path, e))?;
        self.write_projection();
        Ok(())
    }

    /// Queues a rewrite of `timeline.tsx` and `timeline.d.ts` for the current revision. A background writer
    /// coalesces bursts so a commit never waits for printing; failures are logged, never fatal (derived files).
    pub fn write_projection(&self) {
        if let Some(w) = &self.projection {
            w.submit(self.session.doc().clone());
        }
    }

    /// Waits until the projection on disk reflects the current revision.
    pub fn flush_projection(&self) {
        if let Some(w) = &self.projection {
            w.flush();
        }
    }
}

enum Job {
    Write(Box<Document>),
    Flush(std::sync::mpsc::SyncSender<()>),
}

/// Background projection writer: prints the latest submitted document, skipping superseded ones.
struct ProjectionWriter {
    tx: Option<std::sync::mpsc::Sender<Job>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn write_projection_files(bundle: &Path, doc: &Document) {
    let diagnostics = std::fs::read(bundle.join(DIAGNOSTICS_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
        .unwrap_or_default();
    let offline = crate::media::media_status(bundle, doc)
        .into_iter()
        .filter(|m| m.state == crate::media::MediaState::Offline)
        .map(|m| m.media_id)
        .collect();
    let text = ove_projection::print(doc, &ove_projection::PrintContext { diagnostics, offline });
    let tsx = bundle.join(PROJECTION_FILE);
    if let Err(e) = fsio::write_derived(&tsx, text.as_bytes()) {
        tracing::error!(error = %e, "could not write the timeline projection");
    }
    let dts = bundle.join(TYPINGS_FILE);
    let want = ove_projection::typings::typings();
    if std::fs::read_to_string(&dts).ok().as_deref() != Some(want.as_str())
        && let Err(e) = fsio::write_derived(&dts, want.as_bytes())
    {
        tracing::error!(error = %e, "could not write the timeline typings");
    }
}

impl ProjectionWriter {
    fn spawn(bundle: PathBuf) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        let thread = std::thread::Builder::new()
            .name("projection-writer".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    let mut latest = None;
                    let mut acks = Vec::new();
                    let mut take = |j: Job| match j {
                        Job::Write(d) => latest = Some(d),
                        Job::Flush(ack) => acks.push(ack),
                    };
                    take(job);
                    while let Ok(j) = rx.try_recv() {
                        take(j);
                    }
                    if let Some(doc) = latest {
                        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            write_projection_files(&bundle, &doc)
                        }));
                        if r.is_err() {
                            tracing::error!("projection writer panicked; the projection may be stale");
                        }
                    }
                    for a in acks {
                        let _ = a.send(());
                    }
                }
            })
            .expect("spawn projection writer");
        ProjectionWriter { tx: Some(tx), thread: Some(thread) }
    }

    fn submit(&self, doc: Document) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Job::Write(Box::new(doc)));
        }
    }

    fn flush(&self) {
        if let Some(tx) = &self.tx {
            let (ack, done) = std::sync::mpsc::sync_channel(1);
            if tx.send(Job::Flush(ack)).is_ok() {
                let _ = done.recv();
            }
        }
    }
}

impl Drop for ProjectionWriter {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn snapshot_on_disk(bundle: &Path) -> i64 {
    std::fs::read_to_string(bundle.join("project.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["project"]["stateUpdatedAt"].as_i64())
        .unwrap_or(i64::MIN)
}
