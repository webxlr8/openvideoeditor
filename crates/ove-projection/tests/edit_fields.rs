//! Frame rate, sync lock and link fields in the projection, and their lints (add-edit-planners-and-links 1.1, 1.3;
//! clip-links "Link lints" and "Links in the projection").
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::collections::BTreeSet;

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_projection::verify::{Context, Finding, Registry};
use ove_projection::{ApplyInput, Plan, Prefer, plan, print};
use serde_json::{Value, json};

fn base(fps: Option<f64>) -> Document {
    let mut d = Document::new("p", "Links", 100);
    let mut ops = vec![
        Op::AddTrack { track: json!({"id": "v1", "name": "V1", "order": 0, "type": "video"}) },
        Op::AddTrack { track: json!({"id": "a1", "name": "A1", "order": 1, "type": "audio"}) },
        Op::AddMedia {
            media: json!({"id": "m1", "data": {"type": "video", "fileName": "cam.mov", "duration": 30}}),
            entry: Some(
                json!({"mediaId": "m1", "nativePath": "/x/cam.mov", "ownedByBundle": false, "probeStatus": "probed"}),
            ),
        },
        Op::AddItem {
            item: json!({"id": "V", "trackId": "v1", "type": "video", "startTime": 2, "endTime": 6, "mediaItemId": "m1", "trimStart": 1, "link": "L"}),
        },
        Op::AddItem {
            item: json!({"id": "A", "trackId": "a1", "type": "audio", "startTime": 2, "endTime": 6, "mediaItemId": "m1", "trimStart": 1, "link": "L"}),
        },
    ];
    if let Some(f) = fps {
        ops.push(Op::UpdateProject { set: json!({"fps": f}).as_object().unwrap().clone(), unset: vec![] });
    }
    d = apply_batch(&d, &ops).unwrap().doc;
    d.project.rev = 100;
    d
}

fn apply(text: &str, doc: &Document) -> (Plan, Document) {
    let b = doc.clone();
    let p = plan(&ApplyInput {
        text,
        current: doc,
        base_for: &move |_| Ok(b.clone()),
        bundle: None,
        prefer: Prefer::File,
        now_ms: 1_000,
    })
    .unwrap();
    let out = if p.ops.is_empty() { doc.clone() } else { apply_batch(doc, &p.ops).unwrap().doc };
    (p, out)
}

fn lints(doc: &Document) -> Vec<Finding> {
    Registry::default().run(&Context { doc, offline: &BTreeSet::new() })
}

fn edit(text: &str, from: &str, to: &str) -> String {
    assert!(text.contains(from), "`{from}` not in projection:\n{text}");
    text.replacen(from, to, 1)
}

#[test]
fn default_fps_is_omitted_and_a_set_fps_prints() {
    let d = base(None);
    assert_eq!(d.fps(), 30.0);
    assert!(print(&d, &Default::default()).contains("<timeline aspect=\"16:9\">"));
    let d = base(Some(25.0));
    assert_eq!(d.fps(), 25.0);
    assert!(print(&d, &Default::default()).contains("<timeline aspect=\"16:9\" fps={25}>"));
}

#[test]
fn a_file_sets_and_clears_the_frame_rate() {
    let d = base(None);
    let text = print(&d, &Default::default());
    let (_, out) = apply(&edit(&text, "aspect=\"16:9\">", "aspect=\"16:9\" fps={25}>"), &d);
    assert_eq!(out.fps(), 25.0);
    // Writing the default explicitly is no change.
    let (p, _) = apply(&edit(&text, "aspect=\"16:9\">", "aspect=\"16:9\" fps={30}>"), &d);
    assert!(p.ops.is_empty(), "{:?}", p.ops);
    // Removing fps from the file returns to the default.
    let d25 = base(Some(25.0));
    let text = print(&d25, &Default::default());
    let (_, out) = apply(&edit(&text, " fps={25}", ""), &d25);
    assert_eq!(out.fps(), 30.0);
    assert!(!out.project.extra.contains_key("fps"));
}

#[test]
fn sync_lock_prints_only_when_off_and_merges() {
    let d = base(None);
    let text = print(&d, &Default::default());
    assert!(!text.contains("syncLock"));
    let (_, out) = apply(&edit(&text, "kind=\"audio\">", "kind=\"audio\" syncLock={false}>"), &d);
    assert_eq!(out.track("a1").unwrap().fields.get("syncLock"), Some(&Value::Bool(false)));
    assert!(print(&out, &Default::default()).contains("syncLock={false}"));
}

#[test]
fn agent_links_two_items() {
    let mut d = base(None);
    d = apply_batch(&d, &[Op::UnsetItemFields { id: "V".into(), fields: vec!["link".into()] }]).unwrap().doc;
    d = apply_batch(&d, &[Op::UnsetItemFields { id: "A".into(), fields: vec!["link".into()] }]).unwrap().doc;
    let text = print(&d, &Default::default());
    let text = edit(&text, "<clip id=\"V\"", "<clip id=\"V\" link=\"pair\"");
    let text = edit(&text, "<audio id=\"A\"", "<audio id=\"A\" link=\"pair\"");
    let (_, out) = apply(&text, &d);
    assert_eq!(out.item("V").unwrap().fields["link"], "pair");
    assert_eq!(out.item("A").unwrap().fields["link"], "pair");
    assert!(lints(&out).iter().all(|f| !f.lint.starts_with("link.")));
}

#[test]
fn off_grid_time_from_a_file_applies_and_is_reported() {
    let d = base(Some(25.0));
    let text = print(&d, &Default::default());
    let (_, out) = apply(&edit(&text, "<clip id=\"V\" start={2}", "<clip id=\"V\" start={1.013}"), &d);
    assert_eq!(out.item("V").unwrap().start, 1_013_000);
    let f: Vec<_> = lints(&out).into_iter().filter(|f| f.lint == "time.offFrame").collect();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].item_ids, vec!["V".to_string()]);
}

#[test]
fn drift_after_a_file_edit_reports_frames() {
    let d = base(Some(25.0));
    let text = print(&d, &Default::default());
    // Move only the audio 3 frames (0.12 s) later.
    let (_, out) =
        apply(&edit(&text, "<audio id=\"A\" start={2} end={6}", "<audio id=\"A\" start={2.12} end={6.12}"), &d);
    let f: Vec<_> = lints(&out).into_iter().filter(|f| f.lint == "link.offset").collect();
    assert_eq!(f.len(), 1, "{f:?}");
    assert!(f[0].message.contains("3 frames"), "{}", f[0].message);
}

#[test]
fn orphaned_link_is_reported_by_apply_and_verify() {
    let d = base(None);
    let text = print(&d, &Default::default());
    let line = text.lines().find(|l| l.contains("<audio id=\"A\"")).unwrap();
    let (p, out) = apply(&text.replace(&format!("{line}\n"), ""), &d);
    assert_eq!(p.orphaned_links, vec!["V".to_string()]);
    let f: Vec<_> = lints(&out).into_iter().filter(|f| f.lint == "link.single").collect();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].item_ids, vec!["V".to_string()]);
}

/// The `fps` / `syncLock` / `link` example in docs/spec/projection-format.md checks clean against the corpus project
/// that defines its media (`ove source check` runs the same parse and read).
#[test]
fn docs_example_checks_clean() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let doc = std::fs::read_to_string(root.join("../../docs/spec/projection-format.md")).unwrap();
    let start = doc.find("```tsx\n<timeline aspect=\"16:9\" fps={25}>").expect("example present") + "```tsx\n".len();
    let end = start + doc[start..].find("```").unwrap();
    let text = format!("export default (\n{}\n);\n", &doc[start..end]);
    let json = std::fs::read_to_string(root.join("tests/corpus/valid/edit-fields.json")).unwrap();
    let project = ove_core::migrate::open_json(&json).unwrap().doc().unwrap().clone();
    let parsed = ove_projection::parse::parse(&text);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let resolve = ove_projection::read::Resolve { base: &project, current: &project, bundle: None };
    let (model, diags) = ove_projection::read::read(parsed.root.as_ref().unwrap(), resolve);
    assert!(diags.is_empty(), "{diags:?}");
    assert_eq!(model.fps, Some(json!(25)));
    assert_eq!(model.items.iter().filter(|i| i.fields.get("link") == Some(&json!("L1"))).count(), 2);
}
