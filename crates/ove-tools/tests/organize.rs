//! Media organisation as document data (add-semantic-media-search tasks 4.1 and 4.2): a batch is one revision that
//! one undo restores, concurrent folder renames and tagging keep both changes, and random organise batches from the
//! same base commute when they touch different fields.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_core::revision::Source;
use ove_store::project::{Mode, Project};
use ove_tools::tools::organize::plan;
use ove_tools::{BundleCtx, Receipt, Surface, ToolCtx};
use proptest::prelude::*;
use serde_json::{Value, json};

fn clock() -> Arc<dyn ove_core::Clock> {
    Arc::new(ove_core::SystemClock)
}

/// A library with one project, "Lib", holding `n` registered media files `clip00.mov` … (no files on disk needed).
fn fixture(n: usize) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, clock()).unwrap();
    let mut p = library.create("Lib").unwrap();
    let ops: Vec<Op> = (0..n)
        .map(|i| Op::AddMedia {
            media: json!({"id": format!("m{i:02}"), "data": {"type": "video", "fileName": format!("clip{i:02}.mov"), "duration": 30}}),
            entry: Some(
                json!({"mediaId": format!("m{i:02}"), "nativePath": format!("/footage/clip{i:02}.mov"), "ownedByBundle": false}),
            ),
        })
        .collect();
    p.apply(ops, Source::Gui, None).unwrap();
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    (d, lib, bundle)
}

fn call(ctx: &mut dyn ToolCtx, name: &str, a: Value) -> Receipt {
    ove_tools::call(ctx, name, a.as_object().unwrap()).unwrap_or_else(|f| panic!("{name}: {f:?}"))
}

fn doc(bundle: &Path) -> Document {
    Project::open(bundle, Mode::ReadOnly, clock()).unwrap().doc().clone()
}

fn field(d: &Document, id: &str, k: &str) -> Option<Value> {
    d.project.media.get(id).and_then(|m| m.fields.get(k)).cloned()
}

#[test]
fn a_batch_is_one_revision_and_one_undo_restores_it() {
    let (_d, lib, bundle) = fixture(20);
    let before = doc(&bundle);
    let mut ctx = BundleCtx::open(Some(&lib), Surface::Mcp, Source::Mcp).unwrap();
    let mut ops: Vec<Value> = vec![
        json!({"op": "move", "media": (0..10).map(|i| format!("m{i:02}")).collect::<Vec<_>>(), "folder": "Interviews/Day 1"}),
        json!({"op": "move", "media": (10..20).map(|i| format!("clip{i:02}.mov")).collect::<Vec<_>>(), "folder": "B-roll"}),
    ];
    for (i, name) in [(0, "Harbour dusk"), (5, "Market"), (19, "Boat")] {
        ops.push(json!({"op": "rename", "media": format!("m{i:02}"), "name": name}));
    }
    let r = call(&mut ctx, "organize_media", json!({"project": "Lib", "ops": ops}));
    assert_eq!(r.json["changed"], true);
    let after = doc(&bundle);
    assert_eq!(after.project.rev, r.json["rev"].as_i64().unwrap(), "one revision");
    let history = call(&mut ctx, "timeline_history", json!({"project": "Lib"}));
    let entries = history.json["revisions"].as_array().or_else(|| history.json.as_array()).cloned().unwrap_or_default();
    let organise_revs = entries.iter().filter(|e| e.to_string().contains("organise media")).count();
    assert_eq!(organise_revs, 1, "{history:?}");
    assert_eq!(field(&after, "m03", "folder"), Some(json!("Interviews/Day 1")));
    assert_eq!(field(&after, "m15", "folder"), Some(json!("B-roll")));
    assert_eq!(field(&after, "m05", "name"), Some(json!("Market")));
    // The projection's media names follow renames, so `src` names do too.
    let names = ove_projection::print::media_names(&after);
    assert_eq!(names["m19"], "Boat");
    let printed = ove_projection::print(&after, &Default::default());
    assert!(printed.contains("Interviews/Day 1"), "folders print in the header's media table");

    call(&mut ctx, "undo_timeline", json!({"project": "Lib"}));
    let undone = doc(&bundle);
    assert!(undone.content_eq(&before), "one undo restores the previous state");
}

#[test]
fn renaming_a_folder_while_tagging_its_files_keeps_both() {
    let (_d, _lib, bundle) = fixture(6);
    let seed =
        plan(&doc(&bundle), &bundle, &[json!({"op": "move", "media": ["m00", "m01", "m02"], "folder": "Interviews"})])
            .unwrap();
    let base = apply_batch(&doc(&bundle), &seed.ops).unwrap().doc;
    // The person renames the folder and an agent tags its files, both from the same base.
    let rename =
        plan(&base, &bundle, &[json!({"op": "renameFolder", "from": "Interviews", "to": "Talking heads"})]).unwrap();
    let tag = plan(&base, &bundle, &[json!({"op": "tag", "media": ["m00", "m01"], "add": ["approved"]})]).unwrap();
    for (first, second) in [(&rename, &tag), (&tag, &rename)] {
        let one = apply_batch(&base, &first.ops).unwrap().doc;
        let both = apply_batch(&one, &second.ops).unwrap().doc;
        assert_eq!(field(&both, "m00", "folder"), Some(json!("Talking heads")));
        assert_eq!(field(&both, "m01", "tags"), Some(json!(["approved"])));
        assert_eq!(field(&both, "m02", "folder"), Some(json!("Talking heads")));
        assert_eq!(field(&both, "m02", "tags"), None);
    }
}

#[test]
fn subclips_smart_folders_and_errors() {
    let (_d, lib, bundle) = fixture(2);
    let mut ctx = BundleCtx::open(Some(&lib), Surface::Mcp, Source::Mcp).unwrap();
    let r = call(
        &mut ctx,
        "organize_media",
        json!({"project": "Lib", "ops": [
            {"op": "saveSubclip", "media": "m00", "name": "Wave", "in": 1.25, "out": 4.5},
            {"op": "saveSubclip", "media": "m00", "in": 6, "out": 8},
            {"op": "smartFolder", "path": "Interviews", "query": "interview", "scope": "spoken"},
            {"op": "createFolder", "path": "Selects"}
        ]}),
    );
    assert_eq!(r.json["ops"][0]["id"], "s1");
    let d = doc(&bundle);
    assert_eq!(
        field(&d, "m00", "subclips"),
        Some(
            json!([{"id": "s1", "name": "Wave", "in": 1.25, "out": 4.5}, {"id": "s2", "name": "6.0–8.0 s", "in": 6, "out": 8}])
        )
    );
    assert_eq!(
        d.project.extra.get("folders"),
        Some(&json!([{"path": "Interviews", "query": "interview", "scope": "spoken"}, {"path": "Selects"}]))
    );
    let errs = ove_core::invariants::check_document(&d);
    assert!(errs.iter().all(|i| i.severity != ove_core::schema::Severity::Error), "{errs:?}");
    // A bad op refuses the whole call and changes nothing.
    let rev = d.project.rev;
    let e = ove_tools::call(
        &mut ctx,
        "organize_media",
        json!({"project": "Lib", "ops": [
            {"op": "removeSubclip", "media": "m00", "id": "s1"},
            {"op": "saveSubclip", "media": "m00", "in": 5, "out": 2}
        ]})
        .as_object()
        .unwrap(),
    )
    .unwrap_err();
    assert!(e.message.contains("op 1"), "{e:?}");
    assert_eq!(doc(&bundle).project.rev, rev);
    // Deleting a folder moves its media to the parent; repeating a move changes nothing.
    call(
        &mut ctx,
        "organize_media",
        json!({"project": "Lib", "ops": [{"op": "move", "media": "m01", "folder": "A/B"}]}),
    );
    let r = call(
        &mut ctx,
        "organize_media",
        json!({"project": "Lib", "ops": [{"op": "move", "media": "m01", "folder": "A/B/"}]}),
    );
    assert_eq!(r.json["changed"], false);
    call(&mut ctx, "organize_media", json!({"project": "Lib", "ops": [{"op": "deleteFolder", "path": "A/B"}]}));
    assert_eq!(field(&doc(&bundle), "m01", "folder"), Some(json!("A")));
}

/// Documents with organisation fields keep them through save and load, byte for byte, and older documents without
/// them load unchanged.
#[test]
fn organisation_fields_round_trip_and_older_documents_load_unchanged() {
    let (_d, _lib, bundle) = fixture(1);
    let old = doc(&bundle);
    let text = old.to_string_pretty();
    assert!(!text.contains("\"folder\"") && !text.contains("\"folders\""));
    let p = plan(
        &old,
        &bundle,
        &[
            json!({"op": "move", "media": "m00", "folder": "X/Y"}),
            json!({"op": "tag", "media": "m00", "add": ["a", "b"]}),
            json!({"op": "saveSubclip", "media": "m00", "in": 0.5, "out": 1}),
            json!({"op": "smartFolder", "path": "S", "query": "boat"}),
        ],
    )
    .unwrap();
    let new = apply_batch(&old, &p.ops).unwrap().doc;
    let text = new.to_string_pretty();
    let back = match ove_core::migrate::open_json(&text).unwrap() {
        ove_core::migrate::Opened::Current(d) => d,
        other => panic!("{other:?}"),
    };
    assert_eq!(back, new);
    assert_eq!(back.to_string_pretty(), text);
}

fn arb_batch(n_media: usize) -> impl Strategy<Value = Vec<Value>> {
    let media = proptest::sample::select((0..n_media).map(|i| format!("m{i:02}")).collect::<Vec<_>>());
    let folder = proptest::sample::select(vec!["A", "A/B", "B", "C/D", ""]);
    let tag = proptest::sample::select(vec!["x", "y", "z"]);
    let op = prop_oneof![
        (proptest::collection::vec(media.clone(), 1..4), folder.clone())
            .prop_map(|(m, f)| json!({"op": "move", "media": m, "folder": f})),
        (proptest::collection::vec(media.clone(), 1..4), tag.clone(), tag)
            .prop_map(|(m, a, r)| json!({"op": "tag", "media": m, "add": [a], "remove": [r]})),
        (media.clone(), "[a-z]{1,8}").prop_map(|(m, n)| json!({"op": "rename", "media": m, "name": n})),
        (folder.clone(), folder.clone()).prop_map(|(a, b)| json!({"op": "renameFolder", "from": a, "to": b})),
        (media, 0.0f64..10.0, 0.1f64..5.0)
            .prop_map(|(m, a, d)| json!({"op": "saveSubclip", "media": m, "in": a, "out": a + d})),
    ];
    proptest::collection::vec(op, 1..6)
}

fn cases() -> u32 {
    std::env::var("OVE_PROPTEST_CASES").ok().and_then(|s| s.parse().ok()).unwrap_or(1_000)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases() / 4, ..ProptestConfig::default() })]

    /// Any organise batch that plans is valid, invertible and survives save and load.
    #[test]
    fn organise_batches_are_valid_invertible_and_round_trip(ops in arb_batch(6)) {
        let (_d, _lib, bundle) = fixture(6);
        let base = doc(&bundle);
        if let Ok(p) = plan(&base, &bundle, &ops) {
            let applied = apply_batch(&base, &p.ops).unwrap();
            let errs: Vec<_> = ove_core::invariants::check_document(&applied.doc)
                .into_iter()
                .filter(|i| i.severity == ove_core::schema::Severity::Error)
                .collect();
            prop_assert!(errs.is_empty(), "{:?}", errs);
            let back = apply_batch(&applied.doc, &applied.inverse).unwrap().doc;
            prop_assert_eq!(&back, &base);
            let text = applied.doc.to_string_pretty();
            let reread = match ove_core::migrate::open_json(&text).unwrap() {
                ove_core::migrate::Opened::Current(d) => d,
                other => panic!("{other:?}"),
            };
            prop_assert_eq!(reread, applied.doc);
        }
    }

    /// Two batches planned on the same base, one only moving and renaming folders (the person) and one only tagging
    /// and saving subclips (an agent), give the same document in either order, with both changes kept.
    #[test]
    fn folder_and_tag_batches_merge_in_either_order(a in arb_batch(6), b in arb_batch(6)) {
        let (_d, _lib, bundle) = fixture(6);
        let base = doc(&bundle);
        let folders: Vec<Value> = a.into_iter().filter(|o| matches!(o["op"].as_str(), Some("move" | "renameFolder"))).collect();
        let tags: Vec<Value> = b.into_iter().filter(|o| matches!(o["op"].as_str(), Some("tag" | "saveSubclip"))).collect();
        prop_assume!(!folders.is_empty() && !tags.is_empty());
        let (Ok(pa), Ok(pb)) = (plan(&base, &bundle, &folders), plan(&base, &bundle, &tags)) else { return Ok(()) };
        let ab = apply_batch(&apply_batch(&base, &pa.ops).unwrap().doc, &pb.ops).unwrap().doc;
        let ba = apply_batch(&apply_batch(&base, &pb.ops).unwrap().doc, &pa.ops).unwrap().doc;
        prop_assert!(ab.content_eq(&ba));
        let only_a = apply_batch(&base, &pa.ops).unwrap().doc;
        let only_b = apply_batch(&base, &pb.ops).unwrap().doc;
        for id in base.project.media.keys() {
            prop_assert_eq!(field(&ab, id, "folder"), field(&only_a, id, "folder"));
            prop_assert_eq!(field(&ab, id, "tags"), field(&only_b, id, "tags"));
            prop_assert_eq!(field(&ab, id, "subclips"), field(&only_b, id, "subclips"));
        }
    }
}
