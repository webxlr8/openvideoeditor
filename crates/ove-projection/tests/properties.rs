//! Property tests (tasks 3.3 and 4.3): round trip, and merge properties 2–7 from design D5.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::collections::{BTreeMap, BTreeSet};

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_core::testgen::{arb_document, arb_op};
use ove_projection::merge::{changes, item_view, path_str};
use ove_projection::parse::parse;
use ove_projection::read::{Resolve, read};
use ove_projection::{ApplyInput, Prefer, normal, plan, print};
use proptest::prelude::*;

fn cases() -> u32 {
    std::env::var("OVE_PROPTEST_CASES").ok().and_then(|s| s.parse().ok()).unwrap_or(10_000)
}

/// Ops a projection can express (no media/project ops).
fn representable(ops: Vec<Op>) -> Vec<Op> {
    ops.into_iter()
        .filter(|o| {
            !matches!(
                o,
                Op::AddMedia { .. } | Op::UpdateMedia { .. } | Op::RemoveMedia { .. } | Op::UpdateProject { .. }
            )
        })
        .collect()
}

fn edited(b: &Document, ops: Vec<Op>) -> Document {
    match apply_batch(b, &representable(ops)) {
        Ok(a) => a.doc,
        Err(_) => b.clone(),
    }
}

fn arb_three() -> impl Strategy<Value = (Document, Document, Document)> {
    arb_document()
        .prop_flat_map(|b| {
            let f = proptest::collection::vec(arb_op(&b), 1..=5);
            let c = proptest::collection::vec(arb_op(&b), 0..=5);
            (Just(b), f, c)
        })
        .prop_map(|(b, f, c)| {
            let fd = edited(&b, f);
            let cd = edited(&b, c);
            (b, fd, cd)
        })
}

/// Applies the file printed from `f` (whose base is `b`) onto `c`.
fn apply_file(
    b: &Document,
    f: &Document,
    c: &Document,
    prefer: Prefer,
) -> Result<(ove_projection::Plan, Document), ove_projection::Refusal> {
    let text = print(f, &Default::default());
    let plan = plan(&ApplyInput {
        text: &text,
        current: c,
        base_for: &|rev| {
            if rev == b.project.rev {
                Ok(b.clone())
            } else {
                Err(ove_projection::BaseError { code: "rev.missing".into(), message: "x".into() })
            }
        },
        bundle: None,
        prefer,
        now_ms: 42,
    })?;
    let result =
        if plan.ops.is_empty() { c.clone() } else { apply_batch(c, &plan.ops).expect("planned ops apply").doc };
    Ok((plan, result))
}

fn norm(d: &Document) -> Document {
    normal::document(d)
}

/// (entity, field) pairs changed from a to b; track order, aspect and item existence count as fields.
fn touched(a: &Document, b: &Document) -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    let ids: BTreeSet<&String> = a.project.items.keys().chain(b.project.items.keys()).collect();
    for id in ids {
        match (a.project.items.get(id), b.project.items.get(id)) {
            (Some(x), Some(y)) => {
                for (p, _) in changes(&x.kind, &item_view(x), &item_view(y)) {
                    out.insert((id.clone(), p.0.clone()));
                }
            }
            (None, None) => {}
            _ => {
                out.insert((id.clone(), "*".into()));
            }
        }
    }
    let ta: BTreeMap<&String, _> = a.project.tracks.iter().map(|t| (&t.id, normal::track(t))).collect();
    let tb: BTreeMap<&String, _> = b.project.tracks.iter().map(|t| (&t.id, normal::track(t))).collect();
    for id in ta.keys().chain(tb.keys()) {
        let (x, y) = (ta.get(id), tb.get(id));
        if x.map(|t| (&t.name, &t.kind, &t.fields)) != y.map(|t| (&t.name, &t.kind, &t.fields)) {
            out.insert(((*id).clone(), "*".into()));
        }
    }
    let oa: Vec<_> = a.project.tracks.iter().map(|t| &t.id).collect();
    let ob: Vec<_> = b.project.tracks.iter().map(|t| &t.id).collect();
    if oa != ob {
        out.insert(("timeline".into(), "order".into()));
    }
    if a.project.aspect_ratio != b.project.aspect_ratio {
        out.insert(("timeline".into(), "aspect".into()));
    }
    out
}

fn overlaps(x: &BTreeSet<(String, String)>, y: &BTreeSet<(String, String)>) -> bool {
    x.iter().any(|(e, f)| y.iter().any(|(e2, f2)| e == e2 && (f == f2 || f == "*" || f2 == "*")))
        || (x.iter().any(|(e, _)| e != "timeline")
            && y.contains(&("timeline".into(), "order".into()))
            && x.contains(&("timeline".into(), "order".into())))
}

/// Both sides edit the same items: overlapping fields (opacity, name, start, transform keys) with random values.
fn arb_conflicting() -> impl Strategy<Value = (Document, Document, Document)> {
    arb_document()
        .prop_filter("has items", |d| !d.project.items.is_empty())
        .prop_flat_map(|b| {
            let ids: Vec<String> = b.project.items.keys().cloned().collect();
            let edit = proptest::collection::vec(
                (
                    proptest::sample::select(ids),
                    proptest::sample::select(
                        &["opacity", "name", "transform.x", "transform.scale", "zExtra", "endTime"][..],
                    ),
                    0u32..5,
                ),
                1..6,
            );
            (Just(b), edit.clone(), edit)
        })
        .prop_map(|(b, ef, ec)| {
            let side = |edits: Vec<(String, &str, u32)>| {
                let mut d = b.clone();
                for (id, field, v) in edits {
                    let mut item = (*d.project.items[&id]).clone();
                    match field {
                        "opacity" => {
                            item.fields.insert("opacity".into(), serde_json::json!(v as f64 / 5.0));
                        }
                        "name" => {
                            item.fields.insert("name".into(), format!("n{v}").into());
                        }
                        "zExtra" => {
                            item.fields.insert("zExtra".into(), serde_json::json!(v));
                        }
                        "endTime" => {
                            item.end = item.start + 1_000_000 + v as i64 * 250_000;
                        }
                        f => {
                            let key = f.split_once('.').unwrap().1;
                            let t = item.fields.entry("transform").or_insert(serde_json::json!({}));
                            t[key] =
                                serde_json::json!(if key == "x" { v as f64 * 10.0 } else { 1.0 + v as f64 / 10.0 });
                        }
                    }
                    d.project.items.insert(id, std::sync::Arc::new(item));
                }
                d
            };
            let f = side(ef);
            let c = side(ec);
            (b, f, c)
        })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), ..ProptestConfig::default() })]

    /// 5b. Same as 5 with edits that overlap by construction.
    #[test]
    fn overlapping_edits_conflict_per_field((b, f, c) in arb_conflicting(), editor in any::<bool>()) {
        conflicts_hold(&b, &f, &c, editor)?;
    }

    /// 1. parse(print(D)) == normalise(D), and printing is idempotent.
    #[test]
    fn round_trip(d in arb_document()) {
        let text = print(&d, &Default::default());
        let parsed = parse(&text);
        prop_assert!(parsed.diagnostics.is_empty(), "{:?}\n{}", parsed.diagnostics, text);
        let (model, diags) = read(parsed.root.as_ref().unwrap(), Resolve { base: &d, current: &d, bundle: None });
        prop_assert!(diags.is_empty(), "{:?}", diags);
        let n = norm(&d);
        prop_assert_eq!(model.aspect.as_deref(), Some(n.project.aspect_ratio.as_str()));
        prop_assert_eq!(model.tracks.len(), n.project.tracks.len());
        for (ft, t) in model.tracks.iter().zip(&n.project.tracks) {
            prop_assert_eq!(ft.id.as_ref(), Some(&t.id));
            prop_assert_eq!(&ft.name, &t.name);
            prop_assert_eq!(&ft.kind, &t.kind);
            prop_assert_eq!(&ft.fields, &t.fields);
        }
        prop_assert_eq!(model.items.len(), n.project.items.len());
        for fi in &model.items {
            let i = &n.project.items[fi.id.as_ref().unwrap()];
            prop_assert_eq!(&model.tracks[fi.track].id.clone().unwrap(), &i.track_id);
            prop_assert_eq!((&fi.kind, fi.start, fi.end), (&i.kind, i.start, i.end));
            prop_assert_eq!(&fi.fields, &i.fields);
        }
        // Printing the merge of an unchanged file onto itself is the same text.
        let (_, again) = apply_file(&d, &d, &d, Prefer::File).unwrap();
        prop_assert_eq!(print(&again, &Default::default()), text);
    }

    /// 2. Unchanged file onto C yields C exactly and no revision.
    #[test]
    fn unchanged_file_is_a_no_op((b, _f, c) in arb_three()) {
        let (plan, result) = apply_file(&b, &b, &c, Prefer::File).unwrap();
        prop_assert!(plan.ops.is_empty(), "{:?}", plan.ops);
        prop_assert_eq!(result, c);
    }

    /// 3. File-only changes: when C == B the result equals F.  6. Applying the same file twice is a no-op.
    #[test]
    fn file_only_changes_and_idempotence((b, f, _c) in arb_three()) {
        let (plan, result) = apply_file(&b, &f, &b, Prefer::File).unwrap();
        prop_assert!(plan.conflicts.is_empty());
        prop_assert!(norm(&result).content_eq(&norm(&f)), "result != F");
        let (again, result2) = apply_file(&b, &f, &result, Prefer::File).unwrap();
        prop_assert!(again.ops.is_empty(), "second apply changed {:?}", again.ops);
        prop_assert!(again.conflicts.is_empty());
        prop_assert_eq!(result2, result);
    }

    /// 4. Disjoint edits: both edits present, nothing else.  7. Validity: valid or refused.
    #[test]
    fn disjoint_edits_merge((b, f, c) in arb_three()) {
        let tf = touched(&b, &f);
        let tc = touched(&b, &c);
        let outcome = apply_file(&b, &f, &c, Prefer::File);
        if let Ok((_, r)) = &outcome {
            let errs = ove_core::invariants::errors(ove_core::invariants::check_document(r));
            prop_assert!(errs.is_empty());
        }
        if overlaps(&tf, &tc) { return Ok(()); }
        let Ok(oracle) = apply_batch(&norm(&f), &ove_core::diff::diff_documents(&norm(&b), &norm(&c))) else { return Ok(()); };
        let (plan, result) = outcome.expect("disjoint edits apply");
        if std::env::var("OVE_STATS").is_ok() { eprintln!("STAT disjoint-checked f={} c={}", tf.len(), tc.len()); }
        prop_assert!(plan.conflicts.is_empty(), "{:?}", plan.conflicts);
        let (nr, no) = (norm(&result), norm(&oracle.doc));
        prop_assert!(nr.content_eq(&no), "result differs from F + C: diff result->oracle {:?}; file changes {:?}; editor changes {:?}",
            ove_core::diff::diff_documents(&nr, &no), tf, tc);
    }

    /// 5. Conflicts hold the policy's value and are reported exactly once.
    #[test]
    fn conflicts_follow_policy((b, f, c) in arb_three(), editor in any::<bool>()) {
        conflicts_hold(&b, &f, &c, editor)?;
    }
}

fn conflicts_hold(b: &Document, f: &Document, c: &Document, editor: bool) -> Result<(), TestCaseError> {
    let prefer = if editor { Prefer::Editor } else { Prefer::File };
    let Ok((plan, result)) = apply_file(b, f, c, prefer) else {
        return Ok(());
    };
    for (id, bi) in &b.project.items {
        let (Some(fi), Some(ci)) = (f.project.items.get(id), c.project.items.get(id)) else { continue };
        let (bv, fv, cv) = (item_view(bi), item_view(fi), item_view(ci));
        let cf: BTreeMap<String, Option<serde_json::Value>> =
            changes(&bi.kind, &bv, &fv).into_iter().map(|(p, v)| (path_str(&p), v)).collect();
        let cc: BTreeMap<String, Option<serde_json::Value>> =
            changes(&bi.kind, &bv, &cv).into_iter().map(|(p, v)| (path_str(&p), v)).collect();
        for (p, fval) in &cf {
            let Some(cval) = cc.get(p) else { continue };
            if fval == cval {
                continue;
            }
            let n = plan.conflicts.iter().filter(|k| &k.id == id && &k.field == p).count();
            prop_assert_eq!(n, 1, "conflict {}.{} reported {} times", id, p, n);
            if std::env::var("OVE_STATS").is_ok() {
                eprintln!("STAT conflict-checked");
            }
            let want = if editor { cval } else { fval };
            let r = result.project.items.get(id).map(|i| item_view(i));
            let got = r.as_ref().and_then(|r| {
                let (field, key) = p.split_once('.').map(|(a, b)| (a, Some(b))).unwrap_or((p.as_str(), None));
                match key {
                    None => r.get(field).cloned(),
                    Some(k) => r.get(field).and_then(|o| o.get(k)).cloned(),
                }
            });
            prop_assert_eq!(&got, want, "{}.{}", id, p);
        }
    }
    Ok(())
}
