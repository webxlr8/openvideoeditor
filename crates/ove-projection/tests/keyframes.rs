//! Keyframe tracks merge per property (add-markers-keyframe-lanes-and-versions task 3.2): concurrent edits to
//! different properties of one item never conflict and both are kept; edits to the same property conflict and follow
//! the chosen preference.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_projection::{ApplyInput, Prefer, plan, print};
use proptest::prelude::*;
use proptest::sample::select;
use serde_json::{Map, Value, json};

fn cases() -> u32 {
    std::env::var("OVE_PROPTEST_CASES").ok().and_then(|s| s.parse().ok()).unwrap_or(10_000)
}

const PROPS: &[&str] = &["x", "y", "scale", "rotation", "opacity", "borderRadius", "crop.left", "crop.top"];
const EASINGS: &[&str] = &["linear", "ease-in", "ease-out", "ease-in-out", "hold"];

/// A keyframe track for `prop`: 1–4 keyframes inside the 0–4 s item, values 0–100, maybe with easings.
fn arb_track(prop: &'static str) -> impl Strategy<Value = Value> {
    (
        proptest::collection::btree_map(0i64..4000, (0u32..=100, proptest::option::of(select(EASINGS))), 1..=4),
        proptest::option::of(select(EASINGS)),
    )
        .prop_map(move |(keys, easing)| {
            let kfs: Vec<Value> = keys
                .into_iter()
                .map(|(ms, (v, e))| {
                    let mut k = json!({"time": ms as f64 / 1000.0, "value": v});
                    if let Some(e) = e {
                        k["easing"] = e.into();
                    }
                    k
                })
                .collect();
            let mut t = json!({"property": prop, "keyframes": kfs});
            if let Some(e) = easing {
                t["easing"] = e.into();
            }
            t
        })
}

fn doc(tracks: &[Value]) -> Document {
    let mut item =
        json!({"id": "i", "trackId": "t", "type": "text", "startTime": 0, "endTime": 4, "textContent": "Hi"});
    if !tracks.is_empty() {
        item["keyframeTracks"] = Value::Array(tracks.to_vec());
    }
    apply_batch(
        &Document::new("p", "Keyframes", 1),
        &[Op::AddTrack { track: json!({"id": "t", "name": "T", "order": 0, "type": "video"}) }, Op::AddItem { item }],
    )
    .unwrap()
    .doc
}

/// `d` with the item's track for `prop` replaced (`Some`) or removed (`None`).
fn with_track(d: &Document, prop: &str, t: Option<Value>) -> Document {
    let mut list: Vec<Value> =
        d.item("i").unwrap().fields.get("keyframeTracks").and_then(Value::as_array).cloned().unwrap_or_default();
    list.retain(|x| x["property"] != prop);
    if let Some(t) = t {
        list.push(t);
    }
    let op = if list.is_empty() {
        Op::UnsetItemFields { id: "i".into(), fields: vec!["keyframeTracks".into()] }
    } else {
        let mut set = Map::new();
        set.insert("keyframeTracks".into(), Value::Array(list));
        Op::UpdateItem { id: "i".into(), set }
    };
    apply_batch(d, &[op]).unwrap().doc
}

fn apply_file(b: &Document, f: &Document, c: &Document, prefer: Prefer) -> (ove_projection::Plan, Document) {
    let text = print(f, &Default::default());
    let p =
        plan(&ApplyInput { text: &text, current: c, base_for: &|_| Ok(b.clone()), bundle: None, prefer, now_ms: 5 })
            .unwrap_or_else(|r| panic!("refused: {r}\n{text}"));
    let out = if p.ops.is_empty() { c.clone() } else { apply_batch(c, &p.ops).unwrap().doc };
    (p, out)
}

/// The item's track for `prop`, normalised as the projection prints it.
fn track(d: &Document, prop: &str) -> Option<Value> {
    let n = ove_projection::normal::item(d.item("i").unwrap());
    n.fields.get("keyframeTracks").and_then(Value::as_array)?.iter().find(|t| t["property"] == prop).cloned()
}

type Case = (Vec<Value>, (&'static str, Option<Value>), (&'static str, Option<Value>));

/// A base item with tracks on some properties, a file edit of one property and an editor edit of another.
fn arb_disjoint() -> impl Strategy<Value = Case> {
    (proptest::sample::subsequence(PROPS.to_vec(), 2..=PROPS.len()), any::<bool>(), any::<bool>()).prop_flat_map(
        |(props, drop_f, drop_c)| {
            let base: Vec<_> = props.iter().skip(2).map(|p| arb_track(p).boxed()).collect();
            let (pf, pc) = (props[0], props[1]);
            let f = if drop_f { Just(None).boxed() } else { arb_track(pf).prop_map(Some).boxed() };
            let c = if drop_c { Just(None).boxed() } else { arb_track(pc).prop_map(Some).boxed() };
            (base, (Just(pf), f), (Just(pc), c))
        },
    )
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), ..ProptestConfig::default() })]

    #[test]
    fn different_properties_never_conflict((base, (pf, tf), (pc, tc)) in arb_disjoint()) {
        let b = doc(&base);
        let f = with_track(&b, pf, tf);
        let c = with_track(&b, pc, tc);
        for prefer in [Prefer::File, Prefer::Editor] {
            let (p, out) = apply_file(&b, &f, &c, prefer);
            prop_assert!(p.conflicts.is_empty(), "{:?}", p.conflicts);
            prop_assert_eq!(track(&out, pf), track(&f, pf), "the file's edit of {} is kept", pf);
            prop_assert_eq!(track(&out, pc), track(&c, pc), "the editor's edit of {} is kept", pc);
            for t in &base {
                let prop = t["property"].as_str().unwrap();
                prop_assert_eq!(track(&out, prop), track(&b, prop), "{} is untouched", prop);
            }
        }
    }

    #[test]
    fn the_same_property_conflicts_by_the_preference(prop in select(PROPS), (a, bt, ct) in (arb_track("x"), arb_track("x"), arb_track("x"))) {
        let rename = |mut t: Value| { t["property"] = prop.into(); t };
        let b = doc(&[rename(a)]);
        let f = with_track(&b, prop, Some(rename(bt)));
        let c = with_track(&b, prop, Some(rename(ct)));
        prop_assume!(track(&f, prop) != track(&c, prop) && track(&f, prop) != track(&b, prop) && track(&c, prop) != track(&b, prop));
        let (p, out) = apply_file(&b, &f, &c, Prefer::File);
        prop_assert_eq!(p.conflicts.len(), 1);
        prop_assert_eq!(p.conflicts[0].field.clone(), format!("keyframeTracks.{prop}"));
        prop_assert_eq!(track(&out, prop), track(&f, prop));
        let (p, out) = apply_file(&b, &f, &c, Prefer::Editor);
        prop_assert_eq!(p.conflicts.len(), 1);
        prop_assert_eq!(track(&out, prop), track(&c, prop));
    }
}

#[test]
fn user_opacity_and_agent_scale_both_land() {
    let opacity =
        |v: u32| json!({"property": "opacity", "keyframes": [{"time": 0, "value": 0}, {"time": 1, "value": v}]});
    let scale = |v: u32| json!({"property": "scale", "keyframes": [{"time": 0, "value": 1}, {"time": 2, "value": v}]});
    let b = doc(&[opacity(1), scale(1)]);
    let user = with_track(
        &b,
        "opacity",
        Some(
            json!({"property": "opacity", "keyframes": [{"time": 0, "value": 1, "easing": "hold"}, {"time": 2, "value": 0}]}),
        ),
    );
    let agent_file = with_track(&b, "scale", Some(scale(2)));
    let (p, out) = apply_file(&b, &agent_file, &user, Prefer::File);
    assert!(p.conflicts.is_empty(), "{:?}", p.conflicts);
    assert_eq!(track(&out, "opacity"), track(&user, "opacity"));
    assert_eq!(track(&out, "scale"), track(&agent_file, "scale"));
}
