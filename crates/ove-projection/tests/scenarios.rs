//! One test per scenario in specs/timeline-projection (names follow the scenario titles).

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_projection::parse::parse;
use ove_projection::textedit::{TextEdit, apply_edits};
use ove_projection::verify::{Context, Registry, has_errors};
use ove_projection::{ApplyInput, BaseError, Plan, Prefer, Refusal, plan, print};
use serde_json::{Value, json};

fn set(v: Value) -> serde_json::Map<String, Value> {
    v.as_object().unwrap().clone()
}

/// Base: a main track with clips A, B and C and a titles track; revision 100.
fn base() -> Document {
    let mut d = Document::new("p", "Scenarios", 100);
    let ops = vec![
        Op::AddTrack { track: json!({"id": "titles", "name": "Titles", "order": 0, "type": "video"}) },
        Op::AddTrack { track: json!({"id": "main", "name": "Main", "order": 1, "type": "video"}) },
        Op::AddMedia {
            media: json!({"id": "m1", "data": {"type": "video", "fileName": "interview.mov", "duration": 30}}),
            entry: Some(
                json!({"mediaId": "m1", "nativePath": "/x/interview.mov", "ownedByBundle": false, "probeStatus": "probed"}),
            ),
        },
        Op::AddItem {
            item: json!({"id": "A", "trackId": "main", "type": "video", "startTime": 0, "endTime": 4, "name": "Intro", "mediaItemId": "m1"}),
        },
        Op::AddItem {
            item: json!({"id": "B", "trackId": "main", "type": "video", "startTime": 4, "endTime": 8, "name": "Middle", "mediaItemId": "m1", "trimStart": 4}),
        },
        Op::AddItem {
            item: json!({"id": "C", "trackId": "main", "type": "video", "startTime": 8, "endTime": 12, "name": "End", "mediaItemId": "m1", "trimStart": 8}),
        },
        Op::AddItem {
            item: json!({"id": "T", "trackId": "titles", "type": "text", "startTime": 0, "endTime": 3, "textContent": "Hello"}),
        },
    ];
    d = apply_batch(&d, &ops).unwrap().doc;
    d.project.rev = 100;
    d
}

/// The editor's view after GUI edits since the base (rev bumped).
fn gui(base: &Document, ops: Vec<Op>) -> Document {
    let mut d = apply_batch(base, &ops).unwrap().doc;
    d.project.rev = base.project.rev + 10;
    d
}

fn run(text: &str, base: &Document, current: &Document, prefer: Prefer) -> Result<(Plan, Document), Refusal> {
    let b = base.clone();
    let p = plan(&ApplyInput {
        text,
        current,
        base_for: &move |rev| {
            if rev == b.project.rev {
                Ok(b.clone())
            } else {
                Err(BaseError { code: "rev.missing".into(), message: format!("revision {rev} is not available") })
            }
        },
        bundle: None,
        prefer,
        now_ms: 1_000,
    })?;
    let doc = if p.ops.is_empty() { current.clone() } else { apply_batch(current, &p.ops).unwrap().doc };
    Ok((p, doc))
}

fn edit(text: &str, from: &str, to: &str) -> String {
    assert!(text.contains(from), "`{from}` not in projection:\n{text}");
    text.replacen(from, to, 1)
}

fn field(d: &Document, id: &str, f: &str) -> Option<Value> {
    d.item(id).and_then(|i| i.get(f))
}

#[test]
fn stable_output() {
    let d = base();
    let a = print(&d, &Default::default());
    assert_eq!(a, print(&d.clone(), &Default::default()));
    assert!(!a.contains('\r'));
    // Golden check of the exact layout lives in tests/corpus.rs; here: tracks in order, items by start then id.
    let main = a.find("id=\"main\"").unwrap();
    assert!(a.find("id=\"titles\"").unwrap() < main);
    assert!(a.find("id=\"A\"").unwrap() < a.find("id=\"B\"").unwrap());
}

#[test]
fn header_edits_ignored() {
    let d = base();
    let t = edit(&print(&d, &Default::default()), " * duration: 12s", " * duration: 99s");
    let (p, after) = run(&t, &d, &d, Prefer::File).unwrap();
    assert!(p.ops.is_empty());
    assert_eq!(after.duration(), d.duration());
}

#[test]
fn round_trip_unchanged_is_no_op() {
    let d = base();
    let (p, _) = run(&print(&d, &Default::default()), &d, &d, Prefer::File).unwrap();
    assert!(p.ops.is_empty() && p.conflicts.is_empty());
}

#[test]
fn expression_rejected() {
    let d = base();
    let t = edit(&print(&d, &Default::default()), "start={4}", "start={2 + 3}");
    let line = t.lines().position(|l| l.contains("{2 + 3}")).unwrap() + 1;
    let e = run(&t, &d, &d, Prefer::File).unwrap_err();
    let diag = e.diagnostics.iter().find(|x| x.code == "literal.only").expect("literal.only");
    assert_eq!(diag.line, line);
    let col = t.lines().nth(line - 1).unwrap().find("2 + 3").unwrap() + 1;
    assert_eq!(diag.column, col);
    assert!(diag.message.contains("only literal values"));
    for bad in ["start={x}", "start={f()}", "start={`${1}`}", "{...rest}", "start={[...a]}"] {
        let t2 = edit(&print(&d, &Default::default()), "start={4}", bad);
        let e = run(&t2, &d, &d, Prefer::File).unwrap_err();
        assert!(e.diagnostics.iter().any(|x| x.code == "literal.only"), "{bad}: {:?}", e.diagnostics);
    }
}

#[test]
fn backslash_in_quoted_attribute() {
    let d = base();
    let t = edit(&print(&d, &Default::default()), "textContent=\"Hello\"", "textContent=\"C:\\new\"");
    let (_, after) = run(&t, &d, &d, Prefer::File).unwrap();
    assert_eq!(field(&after, "T", "textContent"), Some(json!("C:\\new")));
    // Printed back as a braced string that reads the same.
    let again = print(&after, &Default::default());
    assert!(again.contains(r#"textContent={"C:\\new"}"#), "{again}");
}

#[test]
fn multiple_errors() {
    let d = base();
    // Pad the file so the two errors are far apart.
    let mut t = print(&d, &Default::default());
    t = edit(&t, "name=\"Intro\"", "name=\"Intro\" bad={=}");
    t = edit(&t, "textContent=\"Hello\"", "textContent=\"Hello\" blendMode=\"glow\"");
    let e = run(&t, &d, &d, Prefer::File).unwrap_err();
    let codes: Vec<&str> = e.diagnostics.iter().map(|x| x.code.as_str()).collect();
    assert!(codes.contains(&"syntax"), "{:?}", e.diagnostics);
    assert!(codes.contains(&"enum.known"), "{:?}", e.diagnostics);
    assert!(e.diagnostics.iter().all(|x| x.line > 0 && x.column > 0));
}

#[test]
fn unknown_key_in_a_known_object() {
    let d = base();
    let t =
        edit(&print(&d, &Default::default()), "name=\"Middle\"", "name=\"Middle\" transform={{x: 0, scael: 2, y: 10}}");
    let (p, after) = run(&t, &d, &d, Prefer::File).unwrap();
    let w = p.warnings.iter().find(|w| w.code == "object.key.unknown").expect("warning");
    assert!(w.message.contains("scael"));
    assert_eq!(field(&after, "B", "transform"), Some(json!({"scael": 2, "y": 10})));
}

#[test]
fn removal_is_reported() {
    let d = base();
    let text = print(&d, &Default::default());
    let t: String = text
        .lines()
        .filter(|l| !(l.contains("id=\"A\"") || l.contains("id=\"B\"") || l.contains("id=\"C\"")))
        .map(|l| format!("{l}\n"))
        .collect();
    let (p, after) = run(&t, &d, &d, Prefer::File).unwrap();
    assert_eq!(p.removed.len(), 3);
    assert!(p.summary().contains("removed 3 items"), "{}", p.summary());
    assert!(p.summary().contains("A \"Intro\""));
    assert!(after.item("A").is_none());
}

#[test]
fn new_item_gets_an_id() {
    let d = base();
    let t = edit(
        &print(&d, &Default::default()),
        "    </track>\n  </timeline>",
        "      <clip start={12} end={14} src=\"interview\" />\n    </track>\n  </timeline>",
    );
    let (p, after) = run(&t, &d, &d, Prefer::File).unwrap();
    assert_eq!(p.counts.added, 1);
    let new = after.project.items.values().find(|i| i.start == 12_000_000).unwrap();
    assert!(!new.id.is_empty());
    assert!(print(&after, &Default::default()).contains(&format!("id=\"{}\"", new.id)));
}

#[test]
fn concurrent_edits_to_different_items() {
    let b = base();
    let c = gui(&b, vec![Op::UpdateItem { id: "A".into(), set: set(json!({"startTime": 0.5})) }]);
    let t = edit(&print(&b, &Default::default()), "name=\"Middle\"", "name=\"Middle v2\"");
    let (p, after) = run(&t, &b, &c, Prefer::File).unwrap();
    assert!(p.conflicts.is_empty());
    assert_eq!(after.item("A").unwrap().start, 500_000);
    assert_eq!(field(&after, "B", "name"), Some(json!("Middle v2")));
}

#[test]
fn concurrent_edits_to_different_fields_of_one_item() {
    let b = base();
    let c = gui(&b, vec![Op::UpdateItem { id: "A".into(), set: set(json!({"opacity": 0.4})) }]);
    let t = edit(&print(&b, &Default::default()), "name=\"Intro\" start={0}", "name=\"Intro\" start={1}");
    let (p, after) = run(&t, &b, &c, Prefer::File).unwrap();
    assert!(p.conflicts.is_empty());
    assert_eq!(field(&after, "A", "opacity"), Some(json!(0.4)));
    assert_eq!(after.item("A").unwrap().start, 1_000_000);
}

#[test]
fn unchanged_file_applied_after_gui_edits() {
    let b = base();
    let c = gui(
        &b,
        vec![
            Op::UpdateItem { id: "C".into(), set: set(json!({"name": "Finale"})) },
            Op::RemoveItems { ids: vec!["T".into()] },
        ],
    );
    let (p, after) = run(&print(&b, &Default::default()), &b, &c, Prefer::File).unwrap();
    assert!(p.ops.is_empty());
    assert_eq!(after, c);
}

#[test]
fn same_file_applied_twice() {
    let b = base();
    let t = edit(&print(&b, &Default::default()), "name=\"End\"", "name=\"End v2\"");
    let t = edit(
        &t,
        "    </track>\n  </timeline>",
        "      <clip start={12} end={13} src=\"interview\" />\n    </track>\n  </timeline>",
    );
    let (p1, once) = run(&t, &b, &b, Prefer::File).unwrap();
    assert!(!p1.ops.is_empty());
    let (p2, twice) = run(&t, &b, &once, Prefer::File).unwrap();
    assert!(p2.ops.is_empty(), "{:?}", p2.ops);
    assert!(p2.conflicts.is_empty());
    assert_eq!(twice, once);
}

#[test]
fn different_keys_of_one_object() {
    let mut b = base();
    b = apply_batch(&b, &[Op::UpdateItem { id: "A".into(), set: set(json!({"transform": {"x": 10, "scale": 1.5}})) }])
        .unwrap()
        .doc;
    let c = gui(&b, vec![Op::UpdateItem { id: "A".into(), set: set(json!({"transform": {"x": 99, "scale": 1.5}})) }]);
    let t = edit(&print(&b, &Default::default()), "transform={{x: 10, scale: 1.5}}", "transform={{x: 10, scale: 2}}");
    let (p, after) = run(&t, &b, &c, Prefer::File).unwrap();
    assert!(p.conflicts.is_empty(), "{:?}", p.conflicts);
    assert_eq!(field(&after, "A", "transform"), Some(json!({"x": 99, "scale": 2})));
}

#[test]
fn same_field_default_policy_and_prefer_editor() {
    let b = base();
    let c = gui(&b, vec![Op::UpdateItem { id: "A".into(), set: set(json!({"endTime": 3.5})) }]);
    let t = edit(&print(&b, &Default::default()), "start={0} end={4}", "start={0} end={3}");
    let (p, after) = run(&t, &b, &c, Prefer::File).unwrap();
    assert_eq!(after.item("A").unwrap().end, 3_000_000);
    assert_eq!(p.conflicts.len(), 1);
    let k = &p.conflicts[0];
    assert_eq!((k.id.as_str(), k.field.as_str()), ("A", "endTime"));
    assert_eq!(k.file_value, Some(json!(3)));
    assert_eq!(k.editor_value, Some(json!(3.5)));
    let (p, after) = run(&t, &b, &c, Prefer::Editor).unwrap();
    assert_eq!(after.item("A").unwrap().end, 3_500_000);
    assert_eq!(p.conflicts.len(), 1);
}

#[test]
fn file_removes_an_item_the_editor_changed() {
    let b = base();
    let c = gui(&b, vec![Op::UpdateItem { id: "A".into(), set: set(json!({"opacity": 0.5})) }]);
    let t: String =
        print(&b, &Default::default()).lines().filter(|l| !l.contains("id=\"A\"")).map(|l| format!("{l}\n")).collect();
    let (p, after) = run(&t, &b, &c, Prefer::File).unwrap();
    assert!(after.item("A").is_none());
    assert_eq!(p.conflicts.len(), 1);
    let (p, after) = run(&t, &b, &c, Prefer::Editor).unwrap();
    assert!(after.item("A").is_some());
    assert_eq!(p.conflicts.len(), 1);
}

#[test]
fn file_edits_an_item_the_editor_removed() {
    let b = base();
    let c = gui(&b, vec![Op::RemoveItems { ids: vec!["A".into()] }]);
    let t = edit(&print(&b, &Default::default()), "name=\"Intro\" start={0}", "name=\"Intro\" start={1}");
    let (p, after) = run(&t, &b, &c, Prefer::File).unwrap();
    assert_eq!(after.item("A").unwrap().start, 1_000_000);
    assert_eq!(p.conflicts.len(), 1);
    let (p, after) = run(&t, &b, &c, Prefer::Editor).unwrap();
    assert!(after.item("A").is_none());
    assert_eq!(p.conflicts.len(), 1);
}

#[test]
fn base_revision_availability() {
    let b = base();
    let text = print(&b, &Default::default());
    let pruned = text.replace("rev: 100", "rev: 50");
    assert_eq!(run(&pruned, &b, &b, Prefer::File).unwrap_err().code, "rev.missing");
    let newer = text.replace("rev: 100", "rev: 900");
    assert_eq!(run(&newer, &b, &b, Prefer::File).unwrap_err().code, "rev.newer");
    let malformed = text.replace("rev: 100", "rev: soon");
    assert_eq!(run(&malformed, &b, &b, Prefer::File).unwrap_err().code, "rev.malformed");
    let missing = text.replace(" * rev: 100\n", "");
    let e = run(&missing, &b, &b, Prefer::File).unwrap_err();
    assert_eq!(e.code, "rev.missing");
    assert!(e.message.contains("print the timeline again"));
}

#[test]
fn ambiguous_text_edit() {
    let b = base();
    let text = print(&b, &Default::default());
    let e = apply_edits(&text, &[TextEdit { old_text: "src=\"interview\"".into(), new_text: "src=\"other\"".into() }])
        .unwrap_err();
    assert_eq!(e.code, "edit.ambiguous");
    assert_eq!(e.index, 0);
    assert_eq!(e.positions.len(), 3);
    // A unique edit applies through the normal path with the printed rev as base.
    let t = apply_edits(&text, &[TextEdit { old_text: "name=\"Middle\"".into(), new_text: "name=\"Middle!\"".into() }])
        .unwrap();
    let (p, after) = run(&t, &b, &b, Prefer::File).unwrap();
    assert_eq!(p.counts.updated, 1);
    assert_eq!(field(&after, "B", "name"), Some(json!("Middle!")));
}

#[test]
fn new_file_path_and_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let take = dir.path().join("take3.mov");
    std::fs::write(&take, b"not really a movie").unwrap();
    let b = base();
    let src = take.to_string_lossy().replace('\\', "/");
    let t = edit(
        &print(&b, &Default::default()),
        "    </track>\n  </timeline>",
        &format!("      <clip start={{12}} end={{14}} src=\"{src}\" />\n    </track>\n  </timeline>"),
    );
    let (p, after) = run(&t, &b, &b, Prefer::File).unwrap();
    assert_eq!(p.counts.added, 1);
    let new = after.project.items.values().find(|i| i.start == 12_000_000).unwrap();
    let mid = new.media_id().unwrap();
    assert_eq!(after.registry[mid].probe_status(), "awaiting-probe");
    let printed = print(&after, &Default::default());
    assert!(printed.contains("src=\"take3\""), "{printed}");
    // Applying the same file again does not register it twice.
    let (p2, _) = run(&t, &b, &after, Prefer::File).unwrap();
    assert!(p2.ops.is_empty());
    // Relative paths resolve against the bundle.
    let bundle = dir.path().join("Proj.ove");
    std::fs::create_dir_all(&bundle).unwrap();
    let rel = edit(
        &print(&b, &Default::default()),
        "    </track>\n  </timeline>",
        "      <clip start={12} end={14} src=\"../take3.mov\" />\n    </track>\n  </timeline>",
    );
    let p3 = plan(&ApplyInput {
        text: &rel,
        current: &b,
        base_for: &|_| Ok(b.clone()),
        bundle: Some(&bundle),
        prefer: Prefer::File,
        now_ms: 1,
    })
    .unwrap();
    assert_eq!(p3.counts.added, 1);
    // Missing file.
    let gone = edit(
        &print(&b, &Default::default()),
        "    </track>\n  </timeline>",
        "      <clip start={12} end={14} src=\"/no/such/take9.mov\" />\n    </track>\n  </timeline>",
    );
    let e = run(&gone, &b, &b, Prefer::File).unwrap_err();
    let d = e.diagnostics.iter().find(|d| d.code == "src.unresolved").unwrap();
    assert!(d.message.contains("take9.mov"));
}

#[test]
fn fill_keeps_id() {
    let mut b = base();
    b = apply_batch(&b, &[Op::AddItem { item: json!({"id": "P", "trackId": "main", "type": "video", "startTime": 12, "endTime": 16, "pending": true, "pendingDescription": "aerial"}) }]).unwrap().doc;
    let text = print(&b, &Default::default());
    assert!(text.contains(" * pending: 1"));
    let t = edit(&text, "pending={true}", "src=\"interview\"");
    let (_, after) = run(&t, &b, &b, Prefer::File).unwrap();
    let p = after.item("P").unwrap();
    assert!(!p.is_pending());
    assert_eq!((p.start, p.end), (12_000_000, 16_000_000));
    assert_eq!(p.media_id(), Some("m1"));
    assert!(print(&after, &Default::default()).contains(" * pending: 0"));
}

#[test]
fn verify_clean_and_pending() {
    let b = base();
    let offline = Default::default();
    let reg = Registry::default();
    let f = reg.run(&Context { doc: &b, offline: &offline });
    assert!(f.is_empty(), "{f:?}");
    assert!(!has_errors(&f));
    let d = apply_batch(&b, &[
        Op::AddItem { item: json!({"id": "P1", "trackId": "main", "type": "video", "startTime": 12, "endTime": 13, "pending": true}) },
        Op::AddItem { item: json!({"id": "P2", "trackId": "main", "type": "video", "startTime": 13, "endTime": 14, "pending": true}) },
    ]).unwrap().doc;
    let f = reg.run(&Context { doc: &d, offline: &offline });
    assert_eq!(f.iter().filter(|x| x.lint == "pending").count(), 2);
    assert!(has_errors(&f));
}

#[test]
fn each_lint_positive_and_negative() {
    let b = base();
    let offline: std::collections::BTreeSet<String> = Default::default();
    let reg = Registry::default();
    let run_on = |ops: Vec<Op>, off: &std::collections::BTreeSet<String>| {
        let d = apply_batch(&b, &ops).unwrap().doc;
        reg.run(&Context { doc: &d, offline: off }).into_iter().map(|f| f.lint).collect::<Vec<_>>()
    };
    assert_eq!(
        run_on(vec![Op::UpdateItem { id: "B".into(), set: set(json!({"startTime": 3})) }], &offline),
        vec!["overlap"]
    );
    assert_eq!(
        run_on(vec![Op::UpdateItem { id: "B".into(), set: set(json!({"startTime": 5})) }], &offline),
        vec!["gap"]
    );
    assert_eq!(
        run_on(vec![Op::UpdateItem { id: "C".into(), set: set(json!({"trimStart": 27})) }], &offline),
        vec!["past-source"]
    );
    let off: std::collections::BTreeSet<String> = ["m1".to_string()].into();
    assert_eq!(run_on(vec![], &off).iter().filter(|l| **l == "missing-media").count(), 3);
    // Negatives: the base has none of these.
    assert!(run_on(vec![], &offline).is_empty());
    assert!(parse(&print(&b, &Default::default())).diagnostics.is_empty());
}
