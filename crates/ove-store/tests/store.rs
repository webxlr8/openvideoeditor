//! Bundles, persistence, replay, faults, locking, library and media registry (project-document task groups 4–5).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

mod common;

use std::path::Path;
use std::process::Command;

use common::{batches, clock, set, setup_ops};
use ove_core::migrate::{Opened, open_json};
use ove_core::ops::{Op, apply_batch};
use ove_core::revision::Source;
use ove_store::fsio::{FaultAction, clear_failpoints, set_failpoint};
use ove_store::media::{MediaState, media_status};
use ove_store::project::{Mode, Project};
use ove_store::{Library, Listing};
use serde_json::{Value, json};

fn lib() -> (tempfile::TempDir, Library) {
    let dir = tempfile::tempdir().unwrap();
    let l = Library::open(dir.path(), clock()).unwrap();
    (dir, l)
}

fn read_json(p: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn new_project_layout() {
    let (_d, lib) = lib();
    let p = lib.create("Promo").unwrap();
    let b = p.bundle().to_path_buf();
    assert_eq!(b.file_name().unwrap(), "Promo.ove");
    for d in ["media", "cache", "proxies", ".revs", ".scratch"] {
        assert!(b.join(d).is_dir(), "{d}");
    }
    let text = std::fs::read_to_string(b.join("project.json")).unwrap();
    assert!(matches!(open_json(&text).unwrap(), Opened::Current(_)));
    drop(p);
    // Collision.
    let p2 = lib.create("Promo").unwrap();
    assert_eq!(p2.bundle().file_name().unwrap(), "Promo 2.ove");
    assert!(b.join("project.json").exists());
    // Forbidden characters: folder sanitised, name stored unchanged.
    let p3 = lib.create("Q3: \"Launch\" / Final?").unwrap();
    assert_eq!(p3.bundle().file_name().unwrap(), "Q3 Launch  Final.ove");
    assert_eq!(p3.doc().project.name, "Q3: \"Launch\" / Final?");
}

#[test]
fn reopen_after_replay_equals_memory_for_1000_sequences() {
    let (_d, lib) = lib();
    let n: u64 = std::env::var("OVE_REPLAY_SEQUENCES").ok().and_then(|s| s.parse().ok()).unwrap_or(1000);
    for seed in 1..=n {
        let mut p = lib.create(&format!("seq {seed}")).unwrap();
        p.apply(setup_ops(), Source::Gui, None).unwrap();
        for b in batches(seed, 1 + (seed % 7) as usize) {
            p.apply(b, Source::Agent, None).unwrap();
        }
        if seed % 3 == 0 {
            // Some undo/redo traffic too.
            p.undo().unwrap();
            if seed % 2 == 0 {
                p.redo().unwrap();
            }
        }
        let mem = p.doc().clone();
        let bundle = p.bundle().to_path_buf();
        // Drop without close: project.json may lag; replay must catch up.
        drop(p);
        let reopened = Project::open(&bundle, Mode::ReadOnly, clock()).unwrap();
        assert_eq!(reopened.doc(), &mem, "seed {seed}");
        std::fs::remove_dir_all(&bundle).unwrap();
    }
}

#[test]
fn snapshot_is_written_on_close_and_throttled_while_editing() {
    let (_d, lib) = lib();
    let mut p = lib.create("Snap").unwrap();
    p.apply(setup_ops(), Source::Gui, None).unwrap();
    for b in batches(9, 10) {
        p.apply(b, Source::Gui, None).unwrap();
    }
    // Ten quick batches: the snapshot lags (at most one rewrite per second).
    assert!(p.snapshot_pending());
    let bundle = p.bundle().to_path_buf();
    let rev = p.rev();
    p.close().unwrap();
    assert_eq!(read_json(&bundle.join("project.json"))["project"]["stateUpdatedAt"], json!(rev));
    assert_eq!(read_json(&bundle.join("cache/index.json"))["rev"], json!(rev));
}

#[test]
fn faults_reject_the_batch_and_keep_state() {
    let (_d, lib) = lib();
    let mut p = lib.create("Faults").unwrap();
    p.apply(setup_ops(), Source::Gui, None).unwrap();
    let bundle = p.bundle().to_path_buf();
    for (point, action, code) in [
        ("record.write", FaultAction::DiskFull, "disk.full"),
        ("record.flush", FaultAction::DiskFull, "disk.full"),
        ("record.write", FaultAction::Denied, "permission.denied"),
        ("record.rename", FaultAction::Busy, "file.busy"),
    ] {
        let before = p.doc().clone();
        set_failpoint(point, action, 1);
        let e = p.apply(vec![Op::SetAspectRatio { aspect_ratio: "1:1".into() }], Source::Gui, None).unwrap_err();
        clear_failpoints();
        assert_eq!(e.code(), code, "{point}");
        assert!(e.to_string().contains("could not save the change"), "{e}");
        assert_eq!(p.doc(), &before);
        // Nothing left behind: no temp files, no record for the failed batch.
        let leftovers: Vec<_> = std::fs::read_dir(bundle.join(".revs"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        let reopened = Project::open(&bundle, Mode::ReadOnly, clock()).unwrap();
        assert_eq!(reopened.doc(), &before);
    }
    // A failing snapshot does not fail the batch: the log holds it.
    set_failpoint("snapshot.write", FaultAction::DiskFull, 1);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    p.apply(vec![Op::SetAspectRatio { aspect_ratio: "4:5".into() }], Source::Gui, None).unwrap();
    clear_failpoints();
    let mem = p.doc().clone();
    drop(p);
    assert_eq!(Project::open(&bundle, Mode::ReadOnly, clock()).unwrap().doc(), &mem);
}

#[test]
fn history_restore_and_retention() {
    let (_d, lib) = lib();
    let mut p = lib.create("History").unwrap();
    p.apply(setup_ops(), Source::Gui, None).unwrap();
    let r1 = p.rev();
    let at_r1 = p.doc().clone();
    for b in batches(4, 450) {
        p.apply(b, Source::Cli, None).unwrap();
    }
    let hist = p.history().unwrap();
    assert_eq!(hist.len(), 451);
    assert_eq!(hist[0].source, Source::Gui);
    assert!(hist.windows(2).all(|w| w[1].rev > w[0].rev && w[1].parent == w[0].rev));
    // Periodic snapshots: creation + every 200 revisions.
    let snaps = std::fs::read_dir(p.bundle().join(".revs"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".snap.json"))
        .count();
    assert_eq!(snaps, 3);
    // Reconstruct any retained revision and restore it as a new revision.
    assert!(p.doc_at(r1).unwrap().content_eq(&at_r1));
    let mid = hist[300].rev;
    let at_mid = p.doc_at(mid).unwrap();
    let rec = p.restore(mid).unwrap().unwrap();
    assert_eq!(rec.record.source, Source::Restore);
    assert!(p.doc().content_eq(&at_mid));
    assert_eq!(p.restore(mid).unwrap().map(|c| c.record.rev), None, "already equal");
    // Restoring to before a revision, with a summary that says so, is one more undoable revision.
    let before_last = hist[450].parent;
    let c = p
        .restore_with(before_last, Source::Restore, Some("restore to before the last change".into()))
        .unwrap()
        .unwrap();
    assert_eq!(c.record.summary, "restore to before the last change");
    assert!(p.doc().content_eq(&p.doc_at(before_last).unwrap()));
    assert_eq!(p.session().undo_summary(), Some("restore to before the last change"));
    // A revision that is exactly a periodic snapshot rebuilds from the snapshot alone.
    let snap_rev = hist[199].rev;
    assert_eq!(p.doc_at(snap_rev).unwrap().project.rev, snap_rev);
    assert_eq!(p.doc_at(p.rev() + 10).unwrap_err().code(), "revision.newer");
    assert_eq!(p.doc_at(5).unwrap_err().code(), "revision.missing");
}

#[test]
#[ignore = "slow: 2,300 durable commits; run with --ignored"]
fn prune_keeps_last_2000_and_reconstructs_oldest() {
    let d = tempfile::tempdir().unwrap();
    let ck = std::sync::Arc::new(common::StepClock(std::sync::atomic::AtomicI64::new(1_700_000_000_000)));
    let lib = Library::open(d.path(), ck.clone()).unwrap();
    let mut p = lib.create("Prune").unwrap();
    p.apply(setup_ops(), Source::Gui, None).unwrap();
    for b in batches(5, 2300) {
        p.apply(b, Source::Cli, None).unwrap();
    }
    // Everything is younger than 30 days: retention keeps it all.
    assert_eq!(p.prune().unwrap(), 0);
    ck.0.fetch_add(31 * 24 * 3600 * 1000, std::sync::atomic::Ordering::SeqCst);
    let before = p.history().unwrap();
    let removed = p.prune().unwrap();
    // 2,301 records; snapshots every 200; the cut is at the snapshot before the 2,000th-newest record (record 200).
    assert_eq!(removed, 200);
    let hist = p.history().unwrap();
    assert!(hist.len() >= 2000);
    for h in [&hist[0], &hist[1], &hist[hist.len() / 2]] {
        let rebuilt = p.doc_at(h.rev).unwrap();
        assert_eq!(rebuilt.project.rev, h.rev);
    }
    assert_eq!(p.doc_at(before[10].rev).unwrap_err().code(), "revision.missing");
    // Reopen sees the same retained window.
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    let p = Project::open(&bundle, Mode::ReadOnly, clock()).unwrap();
    p.doc_at(hist[0].rev).unwrap();
}

#[test]
fn migration_keeps_pre_migration_copy() {
    let (d, _lib) = lib();
    let bundle = d.path().join("Old.ove");
    for sub in ["media", "cache", "proxies", ".revs", ".scratch"] {
        std::fs::create_dir_all(bundle.join(sub)).unwrap();
    }
    let v0 = json!({"version": 0, "project": {"id": "o", "name": "Old", "stateUpdatedAt": 5, "aspectRatio": "1:1",
        "trackList": [{"id": "t", "name": "T", "order": 0, "type": "video"}], "items": [], "media": []}, "media": []});
    std::fs::write(bundle.join("project.json"), v0.to_string()).unwrap();
    let mut p = Project::open(&bundle, Mode::Write, clock()).unwrap();
    assert_eq!(p.migrated_from, Some(0));
    assert_eq!(read_json(&bundle.join(".revs/pre-migration-0.json")), v0);
    assert_eq!(read_json(&bundle.join("project.json"))["version"], json!(1));
    p.apply(vec![Op::SetAspectRatio { aspect_ratio: "9:16".into() }], Source::Gui, None).unwrap();
    p.close().unwrap();
}

#[test]
fn newer_version_opens_read_only_without_writing() {
    let (_d, lib) = lib();
    let p = lib.create("Future").unwrap();
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    let mut v = read_json(&bundle.join("project.json"));
    v["version"] = json!(99);
    let text = serde_json::to_string_pretty(&v).unwrap();
    std::fs::write(bundle.join("project.json"), &text).unwrap();
    let mut p = Project::open(&bundle, Mode::Write, clock()).unwrap();
    assert_eq!(p.mode(), Mode::ReadOnly);
    assert!(p.read_only_reason.as_ref().unwrap().contains("newer version"));
    let e = p.apply(vec![Op::SetAspectRatio { aspect_ratio: "1:1".into() }], Source::Gui, None).unwrap_err();
    assert_eq!(e.code(), "project.readOnly");
    p.close().unwrap();
    assert_eq!(std::fs::read_to_string(bundle.join("project.json")).unwrap(), text);
}

#[test]
fn invalid_document_names_the_field_and_is_not_modified() {
    let (_d, lib) = lib();
    let p = lib.create("Broken").unwrap();
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    let text = "{\"format\":\"ove-project\",\"version\":1,\"project\":{\"id\":\"x\",\"name\":\"x\",\"stateUpdatedAt\":1,\"aspectRatio\":\"7:3\"},\"media\":[]}";
    std::fs::write(bundle.join("project.json"), text).unwrap();
    let e = Project::open(&bundle, Mode::Write, clock()).unwrap_err();
    assert!(e.to_string().contains("aspectRatio"), "{e}");
    assert_eq!(std::fs::read_to_string(bundle.join("project.json")).unwrap(), text);
}

#[test]
fn second_writer_in_process_is_refused() {
    let (_d, lib) = lib();
    let p = lib.create("Locked").unwrap();
    let e = Project::open(p.bundle(), Mode::Write, clock()).unwrap_err();
    assert_eq!(e.code(), "project.locked");
    assert!(e.to_string().contains(&format!("pid {}", std::process::id())), "{e}");
    // Read-only opens take no lock.
    Project::open(p.bundle(), Mode::ReadOnly, clock()).unwrap();
}

// ------------------------------------------------------------------------------------------- child processes

fn child(test: &str, envs: &[(&str, &str)]) -> Command {
    let mut c = Command::new(std::env::current_exe().unwrap());
    c.args(["--exact", test, "--ignored", "--nocapture", "--test-threads", "1"]);
    for (k, v) in envs {
        c.env(k, v);
    }
    c
}

/// Child: opens the bundle for writing, says so, and waits for stdin to close (or aborts if asked).
#[test]
#[ignore = "child process helper"]
fn child_hold_lock() {
    let Ok(bundle) = std::env::var("OVE_CHILD_BUNDLE") else { return };
    let _p = Project::open(Path::new(&bundle), Mode::Write, clock()).unwrap();
    println!("LOCKED");
    use std::io::Write;
    std::io::stdout().flush().unwrap();
    if std::env::var("OVE_CHILD_ABORT").is_ok() {
        std::process::abort();
    }
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
}

#[test]
fn second_writer_process_and_stale_lock() {
    use std::io::{BufRead, BufReader, Write};
    let (_d, lib) = lib();
    let p = lib.create("Shared").unwrap();
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    let b = bundle.to_string_lossy().into_owned();

    // A live holder in another process: open fails naming it.
    let mut c = child("child_hold_lock", &[("OVE_CHILD_BUNDLE", &b)])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(c.stdout.take().unwrap());
    let mut line = String::new();
    while !line.contains("LOCKED") {
        line.clear();
        assert!(out.read_line(&mut line).unwrap() > 0, "child exited early");
    }
    let e = Project::open(&bundle, Mode::Write, clock()).unwrap_err();
    assert_eq!(e.code(), "project.locked");
    assert!(e.to_string().contains(&format!("pid {}", c.id())), "{e}");
    c.stdin.take().unwrap().write_all(b"\n").unwrap();
    c.wait().unwrap();

    // A holder that crashed: the lock is free again and the takeover is reported.
    let st = child("child_hold_lock", &[("OVE_CHILD_BUNDLE", &b), ("OVE_CHILD_ABORT", "1")]).output().unwrap();
    assert!(!st.status.success());
    let p = Project::open(&bundle, Mode::Write, clock()).unwrap();
    let prev = p.took_over_from.clone().expect("takeover detected");
    assert_ne!(prev.pid, std::process::id());
}

// ------------------------------------------------------------------------------------------- library

#[test]
fn library_listing_rename_duplicate_trash() {
    let (_d, lib) = lib();
    let mut a = lib.create("Alpha").unwrap();
    a.apply(setup_ops(), Source::Gui, None).unwrap();
    a.apply(batches(1, 1).remove(0), Source::Gui, None).unwrap();
    let a_bundle = a.bundle().to_path_buf();
    a.close().unwrap();
    let b = lib.create("Beta").unwrap();
    let b_bundle = b.bundle().to_path_buf();
    b.close().unwrap();
    // A broken bundle.
    let broken = lib.root.join("Broken.ove");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("project.json"), "{ not json").unwrap();

    let rows = lib.list().unwrap();
    assert_eq!(rows.len(), 3);
    let damaged: Vec<_> = rows.iter().filter(|r| matches!(r, Listing::Damaged { .. })).collect();
    assert_eq!(damaged.len(), 1);
    match &rows.iter().find(|r| r.bundle() == a_bundle).unwrap() {
        Listing::Ok { entry, .. } => {
            assert_eq!(entry.name, "Alpha");
            assert!(entry.item_count >= 1);
            assert_eq!(entry.aspect_ratio, "16:9");
        }
        other => panic!("{other:?}"),
    }

    // Stale index (a record newer than the index) falls back to parsing.
    let mut p = Project::open(&a_bundle, Mode::Write, clock()).unwrap();
    p.apply(vec![Op::SetAspectRatio { aspect_ratio: "9:16".into() }], Source::Cli, None).unwrap();
    std::fs::remove_file(a_bundle.join("cache/index.json")).unwrap();
    drop(p);
    let rows = lib.list().unwrap();
    match rows.iter().find(|r| r.bundle() == a_bundle).unwrap() {
        Listing::Ok { entry, .. } => assert_eq!(entry.aspect_ratio, "9:16"),
        other => panic!("{other:?}"),
    }

    // Rename keeps the folder.
    lib.rename(&a_bundle, "Alpha Final", Source::Gui).unwrap();
    assert!(a_bundle.exists());
    assert_eq!(Project::open(&a_bundle, Mode::ReadOnly, clock()).unwrap().doc().project.name, "Alpha Final");
    assert_eq!(lib.find("Alpha Final").unwrap(), a_bundle);

    // Duplicate: new id, same content, owned media copied.
    let mut p = Project::open(&b_bundle, Mode::Write, clock()).unwrap();
    std::fs::write(b_bundle.join("media/still.png"), b"png").unwrap();
    p.apply(
        vec![Op::AddMedia {
            media: json!({"id": "m1", "data": {"type": "image", "fileName": "still.png"}}),
            entry: Some(json!({"mediaId": "m1", "nativePath": "media/still.png", "ownedByBundle": true})),
        }],
        Source::Gui,
        None,
    )
    .unwrap();
    p.close().unwrap();
    let dup = lib.duplicate(&b_bundle, None).unwrap();
    assert_eq!(dup.file_name().unwrap(), "Beta copy.ove");
    let (orig, copy) = (
        Project::open(&b_bundle, Mode::ReadOnly, clock()).unwrap(),
        Project::open(&dup, Mode::ReadOnly, clock()).unwrap(),
    );
    assert_ne!(orig.doc().project.id, copy.doc().project.id);
    assert_eq!(orig.doc().project.media, copy.doc().project.media);
    assert_eq!(std::fs::read(dup.join("media/still.png")).unwrap(), b"png");

    // Delete moves to the Trash and disappears from the list.
    drop((orig, copy));
    match lib.trash(&dup) {
        Ok(()) => {
            assert!(!dup.exists());
            assert!(lib.list().unwrap().iter().all(|r| r.bundle() != dup));
        }
        // Headless CI containers may have no Trash; the error must still be plain-language.
        Err(e) => assert_eq!(e.code(), "trash.failed", "{e}"),
    }
}

#[test]
fn network_library_is_refused() {
    let d = tempfile::tempdir().unwrap();
    let net = d.path().join("smb-share");
    // SAFETY: test-only environment change before any thread in this test reads it.
    unsafe { std::env::set_var("OVE_TEST_TREAT_AS_NETWORK", &net) };
    let e = ove_store::library::validate_library(&net).unwrap_err();
    assert_eq!(e.code(), "library.network");
    assert!(e.to_string().contains("local drive"));
    ove_store::library::validate_library(&d.path().join("local")).unwrap();
}

#[test]
fn default_library_location() {
    let p = ove_store::library::default_library();
    assert!(p.ends_with("OpenVideoEditor"));
    if cfg!(target_os = "macos") {
        assert!(p.to_string_lossy().contains("/Movies/"));
    } else {
        assert!(p.to_string_lossy().contains("Videos"));
    }
}

#[test]
fn missing_media_reported_offline_without_modifying_document() {
    let (d, lib) = lib();
    let footage = d.path().join("footage.mov");
    std::fs::write(&footage, b"12345").unwrap();
    let changed = d.path().join("changed.mov");
    std::fs::write(&changed, b"abc").unwrap();
    let mut p = lib.create("Media").unwrap();
    let mk = |id: &str, path: &Path, size: u64| Op::AddMedia {
        media: json!({"id": id, "data": {"type": "video"}}),
        entry: Some(json!({"mediaId": id, "nativePath": path.to_string_lossy(), "ownedByBundle": false, "size": size})),
    };
    p.apply(
        vec![mk("ok", &footage, 5), mk("moved", &d.path().join("gone.mov"), 1), mk("changed", &changed, 99)],
        Source::Gui,
        None,
    )
    .unwrap();
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    let before = std::fs::read_to_string(bundle.join("project.json")).unwrap();
    let p = Project::open(&bundle, Mode::Write, clock()).unwrap();
    let st = media_status(&bundle, p.doc());
    let state = |id: &str| st.iter().find(|s| s.media_id == id).unwrap().state;
    assert_eq!(state("ok"), MediaState::Online);
    assert_eq!(state("moved"), MediaState::Offline);
    assert_eq!(state("changed"), MediaState::Changed);
    p.close().unwrap();
    assert_eq!(std::fs::read_to_string(bundle.join("project.json")).unwrap(), before);
}

#[test]
fn same_doc_from_ops_json_cli_shape() {
    // The op wire format used by tools: a JSON array of ops.
    let ops: Vec<Op> = serde_json::from_value(
        json!([{"op": "addTrack", "track": {"id": "t", "name": "T", "order": 0, "type": "video"}}]),
    )
    .unwrap();
    let d = apply_batch(&ove_core::Document::new("x", "x", 1), &ops).unwrap();
    assert_eq!(d.doc.project.tracks.len(), 1);
    let _ = set(json!({}));
}
