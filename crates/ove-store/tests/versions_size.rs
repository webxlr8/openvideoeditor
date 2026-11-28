//! Size and speed of named versions (add-markers-keyframe-lanes-and-versions task 5.2): 20 versions of a 5,000-item
//! project, 1% of the items edited between versions. Prints the bundle growth and the `version diff` times, and
//! checks that shared structure keeps the growth far below 20 full snapshots. Run with
//! `cargo test -p ove-store --test versions_size -- --ignored --nocapture` (debug build; release is faster).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    reason = "test code: a failure here is a failed test; this measurement prints its numbers"
)]

mod common;

use std::time::Instant;

use common::{clock, setup_ops};
use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_store::Library;
use serde_json::json;

fn dir_bytes(p: &std::path::Path) -> u64 {
    let mut total = 0;
    for e in std::fs::read_dir(p).unwrap().flatten() {
        let m = e.metadata().unwrap();
        total += if m.is_dir() { dir_bytes(&e.path()) } else { m.len() };
    }
    total
}

#[test]
#[ignore = "measurement: 5,000 items, 20 versions; run with --ignored --nocapture"]
fn twenty_versions_of_a_5000_item_project() {
    let d = tempfile::tempdir().unwrap();
    let lib = Library::open(d.path(), clock()).unwrap();
    let mut p = lib.create("Big").unwrap();
    p.apply(setup_ops(), Source::Gui, None).unwrap();
    let items: Vec<Op> = (0..5000)
        .map(|i| Op::AddItem {
            item: json!({"id": format!("i{i}"), "trackId": "t1", "type": "text", "startTime": i, "endTime": i + 1,
                "name": format!("Item {i}"), "textContent": "Lorem ipsum dolor sit amet", "opacity": 0.9,
                "transform": {"x": 10, "y": -20, "scale": 1.1}}),
        })
        .collect();
    p.apply(items, Source::Gui, None).unwrap();
    let versions = p.bundle().join(".revs").join("versions");
    let mut first_bytes = 0;
    let mut save_ms = Vec::new();
    for v in 0..20 {
        if v > 0 {
            // 1% of the items change between versions.
            let ops: Vec<Op> = (0..50)
                .map(|k| {
                    let id = format!("i{}", (v * 97 + k * 101) % 5000);
                    let mut set = serde_json::Map::new();
                    set.insert("name".into(), json!(format!("v{v} edit {k}")));
                    Op::UpdateItem { id, set }
                })
                .collect();
            p.apply(ops, Source::Gui, None).unwrap();
        }
        let t = Instant::now();
        let saved = p.save_version(&format!("v{v}"), None, None, v as i64).unwrap();
        save_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        if v == 0 {
            first_bytes = saved.bytes;
        }
    }
    let total = dir_bytes(&versions);
    let t = Instant::now();
    let d_first_last = p.version_diff("v0", Some("v19")).unwrap();
    let diff_ms = t.elapsed().as_secs_f64() * 1000.0;
    let t = Instant::now();
    let d_now = p.version_diff("v10", None).unwrap();
    let diff_now_ms = t.elapsed().as_secs_f64() * 1000.0;
    let project_json = std::fs::metadata(p.bundle().join("project.json")).unwrap().len();
    eprintln!("project.json: {project_json} bytes");
    eprintln!(
        "first version: {first_bytes} bytes; 20 versions: {total} bytes on disk ({:.2}x the first)",
        total as f64 / first_bytes as f64
    );
    eprintln!("save: first {:.0} ms, later median {:.0} ms", save_ms[0], {
        let mut s = save_ms[1..].to_vec();
        s.sort_by(f64::total_cmp);
        s[s.len() / 2]
    });
    eprintln!(
        "diff v0..v19: {} changes in {diff_ms:.0} ms; diff v10..now: {} changes in {diff_now_ms:.0} ms",
        d_first_last.len(),
        d_now.len()
    );
    assert!(d_first_last.len() >= 50, "edits between v0 and v19 are listed");
    assert!(total < first_bytes * 3, "shared structure: 20 versions take less than 3 full snapshots");
}
