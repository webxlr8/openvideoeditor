//! Regression: a file that changes an item's kind under the same id (found by the property test
//! `file_only_changes_and_idempotence`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::sync::Arc;

use ove_core::model::{Document, Item, Project, Track};
use ove_core::ops::apply_batch;
use ove_projection::{ApplyInput, Prefer, normal, plan, print};
use serde_json::json;

fn doc(item: Item) -> Document {
    let track = |i: u32| Track {
        id: format!("t{i}"),
        name: format!("Track {i}"),
        order: i,
        kind: "video".into(),
        fields: Default::default(),
    };
    Document {
        format: "ove-project".into(),
        version: 1,
        project: Project {
            id: "p".into(),
            name: "Generated".into(),
            created_at: 1,
            updated_at: 1,
            rev: 1,
            aspect_ratio: "1:1".into(),
            media: Default::default(),
            items: [(item.id.clone(), Arc::new(item))].into_iter().collect(),
            tracks: vec![track(0), track(1)],
            extra: Default::default(),
        },
        registry: Default::default(),
        extra: Default::default(),
    }
}

#[test]
fn a_file_that_changes_an_items_kind_is_applied_as_written() {
    let mut fields = ove_core::model::Fields::default();
    fields.insert("trimStart".into(), json!(0.205));
    fields.insert("volume".into(), json!(100));
    let b = doc(Item {
        id: "i0".into(),
        track_id: "t0".into(),
        kind: "video".into(),
        start: 3_637_000,
        end: 6_631_000,
        fields,
    });
    let f = doc(Item {
        id: "i0".into(),
        track_id: "t0".into(),
        kind: "image".into(),
        start: 0,
        end: 1000,
        fields: Default::default(),
    });
    let text = print(&f, &Default::default());
    let planned = plan(&ApplyInput {
        text: &text,
        current: &b,
        base_for: &|_| Ok(b.clone()),
        bundle: None,
        prefer: Prefer::File,
        now_ms: 42,
    })
    .unwrap();
    assert!(planned.conflicts.is_empty(), "{:?}", planned.conflicts);
    let result = apply_batch(&b, &planned.ops).unwrap().doc;
    assert_eq!(
        normal::document(&result).project.items.get("i0"),
        normal::document(&f).project.items.get("i0"),
        "the item after applying the file"
    );
}
