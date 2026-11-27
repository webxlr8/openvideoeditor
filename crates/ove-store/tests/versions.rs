//! Named versions (add-markers-keyframe-lanes-and-versions tasks 4.1 and 4.2): save, list, duplicate refusal, shared
//! structure between snapshots, fault injection during a save, survival through pruning, diff and restore with undo.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use common::{StepClock, batches, clock, set, setup_ops};
use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_core::semdiff::Class;
use ove_store::Library;
use ove_store::fsio::{FaultAction, clear_failpoints, set_failpoint};
use ove_store::project::{Mode, Project};
use serde_json::json;

fn project() -> (tempfile::TempDir, Project) {
    let d = tempfile::tempdir().unwrap();
    let lib = Library::open(d.path(), clock()).unwrap();
    let mut p = lib.create("Versions").unwrap();
    p.apply(setup_ops(), Source::Gui, None).unwrap();
    p.apply(
        vec![
            Op::AddItem {
                item: json!({"id": "a", "trackId": "t1", "type": "text", "startTime": 0, "endTime": 4, "name": "Intro"}),
            },
            Op::AddItem { item: json!({"id": "b", "trackId": "t1", "type": "text", "startTime": 4, "endTime": 8}) },
        ],
        Source::Gui,
        None,
    )
    .unwrap();
    (d, p)
}

fn trim_a(p: &mut Project, end: f64) {
    p.apply(vec![Op::UpdateItem { id: "a".into(), set: set(json!({"endTime": end})) }], Source::Gui, None).unwrap();
}

#[test]
fn save_list_print_and_refuse_a_duplicate_name() {
    let (_d, mut p) = project();
    let v1 = p.save_version("v1", Some("first cut"), None, 1).unwrap();
    assert_eq!((v1.rev, v1.note.as_deref()), (p.rev(), Some("first cut")));
    trim_a(&mut p, 3.0);
    p.save_version("client v2", None, None, 2).unwrap();
    let names: Vec<String> = p.versions().unwrap().into_iter().map(|v| v.name).collect();
    assert_eq!(names, ["v1", "client v2"]);
    assert_eq!(p.save_version("v1", None, None, 3).unwrap_err().code(), "version.exists");
    assert_eq!(p.save_version("  ", None, None, 3).unwrap_err().code(), "version.name");
    assert_eq!(p.version_doc("v1").unwrap().item("a").unwrap().end, 4_000_000);
    assert_eq!(p.version_doc("nope").unwrap_err().code(), "version.missing");
    // Reopened, the versions are still there.
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    let p = Project::open(&bundle, Mode::ReadOnly, clock()).unwrap();
    assert_eq!(p.versions().unwrap().len(), 2);
}

#[test]
fn unchanged_items_are_stored_once() {
    let (_d, mut p) = project();
    let first = p.save_version("v1", None, None, 1).unwrap();
    trim_a(&mut p, 3.0);
    let second = p.save_version("v2", None, None, 2).unwrap();
    let packs = ove_store::versions::pack_sizes(p.bundle()).unwrap();
    assert_eq!(
        packs.values().copied().collect::<Vec<_>>(),
        [2, 1],
        "the second version stores only the trimmed a: b is shared"
    );
    assert!(
        second.bytes < first.bytes,
        "the second version adds only what changed ({} vs {})",
        second.bytes,
        first.bytes
    );
}

#[test]
fn a_failed_save_leaves_no_version_and_a_retry_works() {
    let (_d, mut p) = project();
    for point in [
        "version.pack.write",
        "version.pack.flush",
        "version.pack.rename",
        "version.snap.write",
        "version.snap.flush",
        "version.snap.rename",
        "versions.write",
        "versions.flush",
        "versions.rename",
    ] {
        set_failpoint(point, FaultAction::DiskFull, 1);
        let e = p.save_version("v", None, None, 1).unwrap_err();
        clear_failpoints();
        assert!(!e.code().starts_with("version."), "{point}: the write failed: {e}");
        assert!(p.versions().unwrap().is_empty(), "{point}: no half-saved version is listed");
        assert_eq!(p.version_doc("v").unwrap_err().code(), "version.missing");
        // The project itself is unaffected.
        let end = if p.doc().item("a").unwrap().end == 4_000_000 { 3.5 } else { 4.0 };
        trim_a(&mut p, end);
    }
    // No temp files are left behind, and the save then succeeds.
    let leftovers = std::fs::read_dir(p.bundle().join(".revs/versions"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp-"))
        .count();
    assert_eq!(leftovers, 0);
    let v = p.save_version("v", None, None, 2).unwrap();
    assert!(p.version_doc("v").unwrap().content_eq(p.doc()));
    assert_eq!(v.rev, p.rev());
}

#[test]
fn diff_lists_the_trimmed_item_with_old_and_new_times() {
    let (_d, mut p) = project();
    p.save_version("v1", None, None, 1).unwrap();
    trim_a(&mut p, 3.0);
    p.apply(
        vec![
            Op::AddMarker { marker: json!({"id": "m", "at": 1, "note": "tighten"}) },
            Op::RemoveItems { ids: vec!["b".into()] },
        ],
        Source::Gui,
        None,
    )
    .unwrap();
    p.save_version("v2", None, None, 2).unwrap();
    let d = p.version_diff("v1", Some("v2")).unwrap();
    let classes: Vec<(&str, &str, Class)> = d.iter().map(|c| (c.entity, c.id.as_str(), c.class)).collect();
    assert_eq!(classes, [("item", "a", Class::Trimmed), ("item", "b", Class::Removed), ("marker", "m", Class::Added)]);
    assert_eq!((d[0].before.as_ref().unwrap().end, d[0].after.as_ref().unwrap().end), (4.0, 3.0));
    // Against the current document.
    assert!(p.version_diff("v2", None).unwrap().is_empty());
}

#[test]
fn restore_is_one_revision_and_undo_returns_to_before() {
    let (_d, mut p) = project();
    p.save_version("client v2", None, None, 1).unwrap();
    trim_a(&mut p, 2.0);
    p.apply(vec![Op::RemoveItems { ids: vec!["b".into()] }], Source::Gui, None).unwrap();
    let before = p.doc().clone();
    let revs = p.history().unwrap().len();
    let c = p.restore_version("client v2", None).unwrap().unwrap();
    assert_eq!(c.record.source, Source::Restore);
    assert_eq!(p.history().unwrap().len(), revs + 1, "one new revision");
    assert!(p.doc().content_eq(&p.version_doc("client v2").unwrap()));
    p.undo().unwrap().unwrap();
    assert!(p.doc().content_eq(&before), "undo returns to the state before the restore");
    assert!(p.restore_version("client v2", None).unwrap().is_some());
    assert!(p.restore_version("client v2", None).unwrap().is_none(), "already equal");
}

#[test]
#[ignore = "slow: 3,000 durable commits; run with --ignored"]
fn a_version_survives_pruning_after_3000_revisions() {
    let d = tempfile::tempdir().unwrap();
    let ck = Arc::new(StepClock(AtomicI64::new(1_700_000_000_000)));
    let lib = Library::open(d.path(), ck.clone()).unwrap();
    let mut p = lib.create("Prune").unwrap();
    p.apply(setup_ops(), Source::Gui, None).unwrap();
    let mut all = batches(9, 3020).into_iter();
    for b in all.by_ref().take(20) {
        p.apply(b, Source::Cli, None).unwrap();
    }
    let saved = p.doc().clone();
    let v = p.save_version("v1", None, None, 1).unwrap();
    for b in all {
        p.apply(b, Source::Cli, None).unwrap();
    }
    ck.0.fetch_add(31 * 24 * 3600 * 1000, Ordering::SeqCst);
    assert!(p.prune().unwrap() > 0, "pruning removed old revisions");
    assert_eq!(p.doc_at(v.rev).unwrap_err().code(), "revision.missing", "the revision itself was pruned");
    assert!(p.version_doc("v1").unwrap().content_eq(&saved), "the version still prints");
    p.restore_version("v1", None).unwrap().unwrap();
    assert!(p.doc().content_eq(&saved), "and restores");
}
