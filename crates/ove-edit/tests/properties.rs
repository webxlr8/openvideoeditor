//! Planner properties over random timelines (tasks 2.1, 2.2, 2.3, 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "test code")]

mod common;

use std::collections::BTreeSet;

use proptest::prelude::*;

use common::*;
use ove_core::model::{Document, Item};
use ove_core::time::Micros;
use ove_edit::retime::{property_at, speed};
use ove_edit::{Edge, Range, Request, plan};

fn items_on(doc: &Document, track: &str) -> Vec<Item> {
    doc.items_on_track(track).into_iter().cloned().collect()
}

/// Opacity of the item covering `t` on `track` (by the first item found).
fn opacity_at(doc: &Document, track: &str, t: Micros) -> Option<f64> {
    doc.items_on_track(track).into_iter().find(|i| i.start <= t && t < i.end).and_then(|i| property_at(i, "opacity", t))
}

/// Source time shown at timeline time `t` by an item.
fn source_at(it: &Item, t: Micros) -> f64 {
    let s = speed(it);
    let ts = it.time_field("trimStart") as f64;
    if ove_edit::retime::reversed(it) { ts + (it.end - t) as f64 * s } else { ts + (t - it.start) as f64 * s }
}

fn overlap_pairs(doc: &Document) -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    for t in &doc.project.tracks {
        let items = doc.items_on_track(&t.id);
        for (i, a) in items.iter().enumerate() {
            for b in &items[i + 1..] {
                if b.start < a.end {
                    out.insert((a.id.clone(), b.id.clone()));
                }
            }
        }
    }
    out
}

fn blocked_by_lock(doc: &Document, r: &ove_edit::Refusal) -> bool {
    r.code == "ripple.blocked"
        && r.culprits.iter().all(|c| {
            let tr = doc.item(c).map(|i| i.track_id.clone()).unwrap_or_default();
            doc.track(&tr).and_then(|t| t.fields.get("locked")).and_then(|v| v.as_bool()).unwrap_or(false)
        })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// 2.1: a split or a trim never changes a keyframed value at any timeline time the item still covers.
    #[test]
    fn keyframed_values_survive_split_and_trim(doc in arb_doc(), pick in 0usize..8, frac in 0.05f64..0.95, trim_start in any::<bool>()) {
        let v = items_on(&doc, "v1");
        let it = &v[pick % v.len()];
        let cut = it.start + (((it.end - it.start) as f64 * frac) as Micros / F) * F;
        for req in [
            Request::Split { items: vec![it.id.clone()], tracks: vec![], at: cut, linked: true },
            Request::Trim { item: it.id.clone(), edge: if trim_start { Edge::Start } else { Edge::End }, to: cut, linked: true, crossfade: false },
        ] {
            let Ok(p) = plan(&doc, &req) else { continue };
            let after = apply_checked(&doc, &p);
            for x in items_on(&doc, "v1") {
                let mut t = x.start;
                while t < x.end {
                    if let Some(now) = opacity_at(&after, "v1", t) {
                        let was = property_at(&x, "opacity", t).unwrap();
                        prop_assert!((now - was).abs() < 1e-9, "{} at {t}: {was} became {now}", x.id);
                    }
                    t += F;
                }
            }
            // And every source frame shown stays the same.
            for x in items_on(&after, "v1") {
                let orig = doc.items_on_track("v1").into_iter().find(|o| o.start <= x.start && x.start < o.end).unwrap();
                let mut t = x.start;
                while t < x.end {
                    prop_assert!((source_at(&x, t) - source_at(orig, t)).abs() <= 1.0, "{} shows other source at {t}", x.id);
                    t += F;
                }
            }
        }
    }

    /// 2.2: ripple delete of an item: later items on sync-locked tracks move by the removed length, gaps between them
    /// are kept, unlocked-sync tracks do not move, and apply-then-inverse restores the document.
    #[test]
    fn ripple_delete_closes_exactly_the_removed_length(doc in arb_doc(), pick in 0usize..8) {
        let v = items_on(&doc, "v1");
        let it = v[pick % v.len()].clone();
        match plan(&doc, &Request::RippleDelete { items: vec![it.id.clone()], tracks: vec![], ranges: vec![], linked: true }) {
            Err(r) => prop_assert!(blocked_by_lock(&doc, &r), "unexpected refusal {r:?}"),
            Ok(p) => {
                let after = apply_checked(&doc, &p);
                let len = it.end - it.start;
                for t in &doc.project.tracks {
                    let sync = t.fields.get("syncLock").and_then(|v| v.as_bool()).unwrap_or(true);
                    let linked_partner_here = t.id == "a1" && it.fields.contains_key("link");
                    let moves = sync || t.id == "v1" || linked_partner_here;
                    for x in doc.items_on_track(&t.id) {
                        if x.id == it.id || (linked_partner_here && x.fields.get("link") == it.fields.get("link")) {
                            prop_assert!(after.item(&x.id).is_none());
                            continue;
                        }
                        if x.start >= it.end {
                            let y = after.item(&x.id).unwrap();
                            let want = if moves { x.start - len } else { x.start };
                            prop_assert_eq!(y.start, want, "{} on {}", x.id.clone(), t.id.clone());
                            prop_assert_eq!(y.end - y.start, x.end - x.start);
                        }
                    }
                }
                prop_assert_eq!(p.time_map.map("v1", it.end + 7 * F), it.end + 7 * F - len);
            }
        }
    }

    /// 2.2: extract of a range shortens the timeline by the range; insert lengthens it by the inserted length.
    #[test]
    fn extract_and_insert_change_length_by_the_range(doc in arb_doc(), a in 0i64..200, l in 1i64..50) {
        let range = Range { start: a * F, end: (a + l) * F };
        let dur = |d: &Document, tr: &str| d.items_on_track(tr).iter().map(|i| i.end).max().unwrap_or(0);
        match plan(&doc, &Request::Extract { tracks: vec!["v1".into()], range }) {
            Err(r) => prop_assert!(blocked_by_lock(&doc, &r), "unexpected refusal {r:?}"),
            Ok(p) => {
                let after = apply_checked(&doc, &p);
                let len = range.end - range.start;
                let map = |t: Micros| if t >= range.end { t - len } else if t > range.start { range.start } else { t };
                let want = doc
                    .items_on_track("v1")
                    .iter()
                    .filter(|i| !(i.start >= range.start && i.end <= range.end))
                    .map(|i| map(i.end))
                    .max()
                    .unwrap_or(0);
                prop_assert_eq!(dur(&after, "v1"), want);
                prop_assert_eq!(p.time_map.map("v1", range.end + F), range.end + F - len);
            }
        }
        let new = serde_json::json!([{"type": "text", "startTime": 0, "endTime": common::secs(l * F), "textContent": "new"}]);
        match plan(&doc, &Request::Insert { track: "v1".into(), at: a * F, items: new.as_array().unwrap().clone() }) {
            Err(r) => prop_assert!(blocked_by_lock(&doc, &r), "unexpected refusal {r:?}"),
            Ok(p) => {
                let after = apply_checked(&doc, &p);
                let before = dur(&doc, "v1");
                prop_assert_eq!(dur(&after, "v1"), (before + l * F).max(a * F + l * F));
            }
        }
    }

    /// 2.3: slip keeps start and end; roll keeps the sum of the two durations; slide keeps duration and source.
    #[test]
    fn slip_slide_roll_keep_their_invariants(doc in arb_doc(), pick in 0usize..8, d in -60i64..60) {
        let v = items_on(&doc, "v1");
        let it = v[pick % v.len()].clone();
        if let Ok(p) = plan(&doc, &Request::Slip { item: it.id.clone(), delta: d * F, linked: true }) {
            let after = apply_checked(&doc, &p);
            let y = after.item(&it.id).unwrap();
            prop_assert_eq!((y.start, y.end), (it.start, it.end));
            prop_assert!(y.time_field("trimStart") >= 0);
            if !p.limited.is_empty() || d == 0 { } else {
                prop_assert_ne!(y.time_field("trimStart"), it.time_field("trimStart"));
            }
        }
        if let Ok(p) = plan(&doc, &Request::Slide { item: it.id.clone(), delta: d * F, linked: true }) {
            let after = apply_checked(&doc, &p);
            let y = after.item(&it.id).unwrap();
            prop_assert_eq!(y.end - y.start, it.end - it.start);
            prop_assert_eq!(y.time_field("trimStart"), it.time_field("trimStart"));
            // Neighbours that touch it keep their far edges; every other item keeps its place.
            for x in doc.items_on_track("v1") {
                let z = after.item(&x.id).unwrap();
                if x.id == it.id {
                    continue;
                } else if x.end == it.start {
                    prop_assert_eq!(z.start, x.start);
                    prop_assert_eq!(z.end, y.start);
                } else if x.start == it.end {
                    prop_assert_eq!(z.end, x.end);
                    prop_assert_eq!(z.start, y.end);
                } else {
                    prop_assert_eq!((z.start, z.end), (x.start, x.end));
                }
            }
        }
        let next = v.iter().find(|n| n.start == it.end).cloned();
        let roll = next.as_ref().map(|n| Request::Roll { left: it.id.clone(), right: n.id.clone(), to: it.end + d * F, linked: true });
        if let (Some(n), Some(Ok(p))) = (next, roll.map(|r| plan(&doc, &r))) {
            {
                let after = apply_checked(&doc, &p);
                let (a, b) = (after.item(&it.id).unwrap(), after.item(&n.id).unwrap());
                prop_assert_eq!((a.end - a.start) + (b.end - b.start), (it.end - it.start) + (n.end - n.start));
                prop_assert_eq!(a.end, b.start);
                prop_assert_eq!((a.start, b.end), (it.start, n.end));
            }
        }
    }

    /// 2.5: no request creates an overlap that was not there, and a refusal changes nothing.
    #[test]
    fn no_new_overlaps(doc in arb_doc(), pick in 0usize..8, which in 0usize..9, d in -40i64..40, t in 0i64..300) {
        let v = items_on(&doc, "v1");
        let it = v[pick % v.len()].clone();
        let req = match which {
            0 => Request::Split { items: vec![], tracks: vec![], at: t * F, linked: true },
            1 => Request::Trim { item: it.id.clone(), edge: Edge::End, to: it.end + d * F, linked: true, crossfade: false },
            2 => Request::Trim { item: it.id.clone(), edge: Edge::Start, to: it.start + d * F, linked: true, crossfade: false },
            3 => Request::RippleTrim { item: it.id.clone(), edge: Edge::End, to: it.end + d * F, linked: true },
            4 => Request::RippleTrim { item: it.id.clone(), edge: Edge::Start, to: it.start + d * F, linked: true },
            5 => Request::Move { item: it.id.clone(), start: it.start + d * F, track: None, linked: true, crossfade: false },
            6 => Request::Lift { tracks: vec![], range: Range { start: t * F, end: (t + d.abs() + 1) * F } },
            7 => Request::CloseGap { track: "v1".into(), at: t * F },
            _ => Request::Overwrite { track: "v1".into(), at: t * F, items: vec![serde_json::json!({"type": "text", "startTime": 0, "endTime": 1, "textContent": "x"})] },
        };
        let before = overlap_pairs(&doc);
        if let Ok(p) = plan(&doc, &req) {
            let after = apply_checked(&doc, &p);
            for pair in overlap_pairs(&after) {
                prop_assert!(before.contains(&pair), "{req:?} created overlap {pair:?}");
            }
        }
    }

    /// 4.2: move and ripple keep `start − in` equal across a link group unless the override is set.
    #[test]
    fn linked_groups_stay_in_sync(doc in arb_doc(), pick in 0usize..8, d in -40i64..40) {
        let v = items_on(&doc, "v1");
        let it = v[pick % v.len()].clone();
        let zero = |i: &Item| i.start as f64 - i.time_field("trimStart") as f64 / speed(i);
        for req in [
            Request::Move { item: it.id.clone(), start: it.start + d * F, track: None, linked: true, crossfade: true },
            Request::RippleTrim { item: it.id.clone(), edge: Edge::End, to: it.end + d * F, linked: true },
            Request::Trim { item: it.id.clone(), edge: Edge::Start, to: it.start + d * F, linked: true, crossfade: true },
            Request::Slip { item: it.id.clone(), delta: d * F, linked: true },
        ] {
            let Ok(p) = plan(&doc, &req) else { continue };
            let after = apply_checked(&doc, &p);
            let groups = ove_projection_free_groups(&after);
            for members in groups.values() {
                if members.len() > 1 && !members.iter().any(ove_edit::retime::reversed) {
                    let z0 = zero(&members[0]);
                    for m in members {
                        prop_assert!((zero(m) - z0).abs() <= 2.0, "{req:?}: {} drifted", m.id);
                    }
                }
            }
        }
    }
}

/// Link groups by value.
fn ove_projection_free_groups(doc: &Document) -> std::collections::BTreeMap<String, Vec<Item>> {
    let mut g: std::collections::BTreeMap<String, Vec<Item>> = Default::default();
    for it in doc.project.items.values() {
        if let Some(l) = it.fields.get("link").and_then(|v| v.as_str()) {
            g.entry(l.to_string()).or_default().push((**it).clone());
        }
    }
    g
}

/// The properties above only check plans that succeed: make sure most random requests do succeed, so they are not
/// passing vacuously.
#[test]
fn most_random_requests_plan() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let mut runner = TestRunner::deterministic();
    let mut ok = [0usize; 6];
    let n = 300;
    for _ in 0..n {
        let doc = arb_doc().new_tree(&mut runner).unwrap().current();
        let v = items_on(&doc, "v1");
        let it = v[v.len() / 2].clone();
        let mid = it.start + ((it.end - it.start) / 2 / F) * F;
        let reqs = [
            Request::Split { items: vec![it.id.clone()], tracks: vec![], at: mid, linked: true },
            Request::Trim { item: it.id.clone(), edge: Edge::Start, to: mid, linked: true, crossfade: false },
            Request::RippleDelete { items: vec![it.id.clone()], tracks: vec![], ranges: vec![], linked: true },
            Request::Slip { item: it.id.clone(), delta: 3 * F, linked: true },
            Request::Slide { item: it.id.clone(), delta: -2 * F, linked: true },
            Request::RippleTrim { item: it.id.clone(), edge: Edge::End, to: mid, linked: true },
        ];
        for (k, r) in reqs.iter().enumerate() {
            if let Ok(p) = plan(&doc, r) {
                apply_checked(&doc, &p);
                ok[k] += 1;
            }
        }
    }
    eprintln!("successful plans out of {n}: {ok:?}");
    for (k, c) in ok.iter().enumerate() {
        assert!(*c * 10 >= n * 7, "request {k} planned only {c} of {n} times");
    }
}
