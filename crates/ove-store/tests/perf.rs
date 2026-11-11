//! Performance suite (task 6.1). Run in release: `cargo test --release -p ove-store --test perf -- --ignored --nocapture`.
//! Budgets (spec: Document performance) are asserted when `OVE_PERF_ENFORCE=1`.

mod common;

use std::time::{Duration, Instant};

use common::{clock, set};
use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_store::Library;
use ove_store::project::{Mode, Project};
use serde_json::json;

fn enforce() -> bool {
    std::env::var("OVE_PERF_ENFORCE").is_ok_and(|v| v == "1")
}

fn pct(v: &mut [Duration], p: f64) -> Duration {
    v.sort();
    v[((v.len() as f64 - 1.0) * p).round() as usize]
}

/// Ops building a realistic 5,000-item project: 8 tracks, mixed item types with typical fields.
fn big_project_ops(n: usize) -> Vec<Op> {
    let mut ops = Vec::new();
    for t in 0..8 {
        let kind = ["video", "video", "caption", "video", "video", "audio", "audio", "adjustment"][t];
        ops.push(Op::AddTrack {
            track: json!({"id": format!("t{t}"), "name": format!("Track {t}"), "order": t, "type": kind}),
        });
    }
    for m in 0..50 {
        ops.push(Op::AddMedia {
            media: json!({"id": format!("m{m}"), "createdAt": 1, "addedAt": 1, "data": {"type": "video", "fileName": format!("clip{m}.mov"), "duration": 120, "width": 3840, "height": 2160, "frameRate": 25, "hasAudio": true}}),
            entry: Some(json!({"mediaId": format!("m{m}"), "nativePath": format!("/Volumes/Footage/clip{m}.mov"), "ownedByBundle": false, "probeStatus": "probed", "size": 123456789, "modifiedAt": 1700000000000i64})),
        });
    }
    for i in 0..n {
        let t = i % 8;
        let start = (i / 8) as f64 * 2.5;
        let item = match t {
            2 => {
                json!({"id": format!("i{i}"), "trackId": "t2", "type": "caption", "startTime": start, "endTime": start + 2.4, "captionText": "Some words here", "captionSegments": [{"id": "s", "text": "Some words here", "startTime": 0, "endTime": 2.4, "wordTimings": [{"word": "Some", "start": 0, "end": 0.5}, {"word": "words", "start": 0.5, "end": 1.2}, {"word": "here", "start": 1.2, "end": 2.4}]}], "captionPresetId": "bold"})
            }
            5 | 6 => {
                json!({"id": format!("i{i}"), "trackId": format!("t{t}"), "type": "audio", "startTime": start, "endTime": start + 2.4, "mediaItemId": format!("m{}", i % 50), "volume": 80, "audioFadeIn": {"duration": 0.2}})
            }
            7 => {
                json!({"id": format!("i{i}"), "trackId": "t7", "type": "adjustment", "startTime": start, "endTime": start + 2.4, "colorGrading": {"contrast": 0.1, "saturation": 0.05}})
            }
            _ => {
                json!({"id": format!("i{i}"), "trackId": format!("t{t}"), "type": "video", "startTime": start, "endTime": start + 2.4, "name": format!("Shot {i}"), "trimStart": 1.25, "mediaItemId": format!("m{}", i % 50), "transform": {"x": 12, "y": -8, "scale": 1.1}, "opacity": 0.95, "volume": 90, "keyframeTracks": [{"property": "scale", "keyframes": [{"time": 0, "value": 1}, {"time": 2.4, "value": 1.1}]}]})
            }
        };
        ops.push(Op::AddItem { item });
    }
    ops
}

#[test]
#[ignore = "performance suite; run in release"]
fn large_project_open_and_apply() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path(), clock()).unwrap();
    let mut p = lib.create("Big").unwrap();
    p.apply(big_project_ops(5000), Source::Gui, None).unwrap();
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    let size = std::fs::metadata(bundle.join("project.json")).unwrap().len();

    let mut opens = Vec::new();
    for _ in 0..10 {
        let t = Instant::now();
        let p = Project::open(&bundle, Mode::Write, clock()).unwrap();
        opens.push(t.elapsed());
        assert_eq!(p.doc().project.items.len(), 5000);
    }
    let open_p50 = pct(&mut opens, 0.5);

    let mut p = Project::open(&bundle, Mode::Write, clock()).unwrap();
    let mut applies = Vec::new();
    for k in 0..1000 {
        let id = format!("i{}", (k * 37) % 5000);
        let t = Instant::now();
        p.apply(vec![Op::UpdateItem { id, set: set(json!({"opacity": (k % 100) as f64 / 100.0})) }], Source::Gui, None)
            .unwrap();
        applies.push(t.elapsed());
    }
    let (a50, a95, amax) = (pct(&mut applies, 0.5), pct(&mut applies, 0.95), pct(&mut applies, 1.0));
    eprintln!(
        "PERF 5000-item project.json {:.1} MB; open p50 {:?} (min {:?}); 1-op apply p50 {:?} p95 {:?} max {:?}",
        size as f64 / 1e6,
        open_p50,
        opens[0],
        a50,
        a95,
        amax
    );
    if enforce() {
        assert!(open_p50 <= Duration::from_millis(200), "open {open_p50:?}");
        assert!(a95 <= Duration::from_millis(10), "apply p95 {a95:?}");
    }
}

#[test]
#[ignore = "performance suite; run in release"]
fn ten_thousand_revisions_open_and_prune() {
    let dir = tempfile::tempdir().unwrap();
    let ck = std::sync::Arc::new(common::StepClock(std::sync::atomic::AtomicI64::new(1_700_000_000_000)));
    let lib = Library::open(dir.path(), ck.clone()).unwrap();
    let mut p = lib.create("Revs").unwrap();
    p.apply(big_project_ops(500), Source::Gui, None).unwrap();
    let t = Instant::now();
    for k in 0..10_000 {
        p.apply(
            vec![Op::UpdateItem {
                id: format!("i{}", k % 500),
                set: set(json!({"opacity": (k % 100) as f64 / 100.0})),
            }],
            Source::Gui,
            None,
        )
        .unwrap();
    }
    let commit_all = t.elapsed();
    let bundle = p.bundle().to_path_buf();
    drop(p); // no close: the open below must replay up to 1 s of records
    let t = Instant::now();
    let mut p = Project::open(&bundle, Mode::Write, ck.clone()).unwrap();
    let open = t.elapsed();
    let t = Instant::now();
    let old = p.doc_at(p.history().unwrap()[100].rev).unwrap();
    let reconstruct = t.elapsed();
    assert!(old.project.items.len() == 500);
    ck.0.fetch_add(31 * 24 * 3600 * 1000, std::sync::atomic::Ordering::SeqCst);
    let t = Instant::now();
    let removed = p.prune().unwrap();
    let prune = t.elapsed();
    eprintln!(
        "PERF 10,000 revisions: commit total {commit_all:?} ({:?}/commit); open {open:?}; reconstruct an old revision {reconstruct:?}; prune {prune:?} ({removed} removed)",
        commit_all / 10_000
    );
}

#[test]
#[ignore = "performance suite; run in release"]
fn breakdown_of_one_op_apply() {
    use ove_store::fsio;
    let dir = tempfile::tempdir().unwrap();
    // Durable write alone (same primitive as a revision record).
    let mut w = Vec::new();
    for k in 0..300 {
        let t = Instant::now();
        fsio::write_atomic(&dir.path().join(format!("{k}.json")), &[b'x'; 600], fsio::RECORD, false).unwrap();
        w.push(t.elapsed());
    }
    // In-memory validate + apply on a 5,000-item document.
    let doc = ove_core::apply_batch(&ove_core::Document::new("p", "p", 1), &big_project_ops(5000)).unwrap().doc;
    let mut m = Vec::new();
    for k in 0..300 {
        let t = Instant::now();
        let a =
            ove_core::apply_batch(&doc, &[Op::UpdateItem { id: format!("i{k}"), set: set(json!({"opacity": 0.5})) }])
                .unwrap();
        m.push(t.elapsed());
        drop(a);
    }
    eprintln!(
        "PERF breakdown: durable record write p50 {:?} p95 {:?}; in-memory apply on 5,000 items p50 {:?} p95 {:?}",
        pct(&mut w, 0.5),
        pct(&mut w, 0.95),
        pct(&mut m, 0.5),
        pct(&mut m, 0.95)
    );
}
