//! Media organisation fields (add-semantic-media-search task 4.1): `name`, `folder`, `tags` and `subclips` on media
//! items and `folders` on the project are validated like every known field, and subclips keep their own invariants.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code: a failure here is a failed test")]

use ove_core::invariants::check_document;
use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_core::schema::Severity;
use serde_json::{Value, json};

fn base() -> Document {
    let ops = vec![Op::AddMedia {
        media: json!({"id": "m", "data": {"type": "video", "fileName": "a.mov", "duration": 10}}),
        entry: Some(json!({"mediaId": "m", "nativePath": "/a.mov", "ownedByBundle": false})),
    }];
    apply_batch(&Document::new("p", "P", 1), &ops).unwrap().doc
}

fn codes(set_media: Value, set_project: Option<Value>) -> Vec<&'static str> {
    let mut ops = vec![Op::UpdateMedia {
        id: "m".into(),
        set: set_media.as_object().unwrap().clone(),
        unset: vec![],
        entry_set: Default::default(),
        entry_unset: vec![],
    }];
    if let Some(p) = set_project {
        ops.push(Op::UpdateProject { set: p.as_object().unwrap().clone(), unset: vec![] });
    }
    let d = match apply_batch(&base(), &ops) {
        Ok(a) => a.doc,
        Err(e) => return vec![e.code],
    };
    check_document(&d).into_iter().filter(|i| i.severity == Severity::Error).map(|i| i.code).collect()
}

#[test]
fn valid_organisation_fields_pass() {
    let c = codes(
        json!({"name": "Harbour", "folder": "A/B", "tags": ["x"], "subclips": [{"id": "s1", "name": "n", "in": 1, "out": 2}]}),
        Some(json!({"folders": [{"path": "S", "query": "boat", "scope": "moments"}, {"path": "E"}]})),
    );
    assert!(c.is_empty(), "{c:?}");
}

#[test]
fn wrong_types_and_bad_subclips_are_reported() {
    assert!(codes(json!({"folder": 3}), None).contains(&"value.type"));
    assert!(codes(json!({"tags": "x"}), None).contains(&"value.type"));
    assert!(codes(json!({"name": false}), None).contains(&"value.type"));
    assert!(codes(json!({"subclips": [{"id": "s", "in": -1, "out": 2}]}), None).contains(&"value.nonNegative"));
    assert!(codes(json!({"subclips": [{"id": "s", "in": 3, "out": 2}]}), None).contains(&"subclip.range"));
    let dup = json!({"subclips": [{"id": "s", "in": 0, "out": 1}, {"id": "s", "in": 1, "out": 2}]});
    assert!(codes(dup, None).contains(&"subclip.id.unique"));
    let bad_scope = codes(json!({}), Some(json!({"folders": [{"path": "S", "query": "q", "scope": "everything"}]})));
    assert!(bad_scope.contains(&"enum.known"), "{bad_scope:?}");
}
