//! Rebase over ripples (add-edit-planners-and-links 3.2, 3.3): a projection printed before a GUI ripple applies
//! as if it had been edited after it; conflicts with a ripple come back as one group.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use proptest::prelude::*;
use serde_json::{Value, json};

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_core::time::{Micros, micros_to_json};
use ove_core::timemap::{Ripple, TimeMap};
use ove_projection::{ApplyInput, Plan, Prefer, Refusal, plan_rebased, print};

const F: Micros = 40_000;
const S: Micros = 1_000_000;

fn cases() -> u32 {
    std::env::var("OVE_PROPTEST_CASES").ok().and_then(|s| s.parse().ok()).unwrap_or(10_000)
}

/// Items on `v1` as (gap, length) in frames, each linked to an audio partner on `a1` when `linked`; a music bed
/// on `music` with the given sync lock.
fn build(items: &[(i64, i64, bool)], music_sync: bool) -> Document {
    let mut ops = vec![
        Op::UpdateProject { set: json!({"fps": 25}).as_object().unwrap().clone(), unset: vec![] },
        Op::AddTrack { track: json!({"id": "v1", "name": "V1", "order": 0, "type": "video"}) },
        Op::AddTrack { track: json!({"id": "a1", "name": "A1", "order": 1, "type": "audio"}) },
        Op::AddTrack {
            track: json!({"id": "music", "name": "Music", "order": 2, "type": "audio", "syncLock": music_sync}),
        },
        Op::AddMedia {
            media: json!({"id": "m", "data": {"type": "video", "fileName": "cam.mov", "duration": 600}}),
            entry: Some(json!({"mediaId": "m", "nativePath": "/x/cam.mov", "ownedByBundle": false})),
        },
        Op::AddItem {
            item: json!({"id": "bed", "trackId": "music", "type": "audio", "startTime": 0, "endTime": 100, "mediaItemId": "m"}),
        },
    ];
    let mut cursor = 0;
    for (n, (gap, len, linked)) in items.iter().enumerate() {
        let s = cursor + gap * F;
        cursor = s + len * F;
        let mut it = json!({"id": format!("x{n}"), "trackId": "v1", "type": "video", "startTime": micros_to_json(s),
            "endTime": micros_to_json(cursor), "mediaItemId": "m", "trimStart": micros_to_json((n as i64 + 1) * S)});
        if *linked {
            it["link"] = json!(format!("L{n}"));
            let mut a = it.clone();
            a["id"] = json!(format!("y{n}"));
            a["trackId"] = json!("a1");
            a["type"] = json!("audio");
            ops.push(Op::AddItem { item: a });
        }
        ops.push(Op::AddItem { item: it });
    }
    let mut d = apply_batch(&Document::new("p", "Rebase", 1), &ops).unwrap().doc;
    d.project.rev = 100;
    d
}

/// The GUI ripple: plans a request, applies it as revision 200 and returns the document and its ripple.
fn ripple(d0: &Document, req: &ove_edit::Request) -> Option<(Document, Ripple)> {
    let p = ove_edit::plan(d0, req).ok()?;
    if p.ops.is_empty() {
        return None;
    }
    let mut d1 = apply_batch(d0, &p.ops).unwrap().doc;
    d1.project.rev = 200;
    let touched = p.touched.iter().cloned().collect();
    Some((d1, Ripple { rev: 200, map: p.time_map, touched }))
}

fn apply(text: &str, d0: &Document, d1: &Document, rip: &Ripple, prefer: Prefer) -> Result<(Plan, Document), Refusal> {
    let b = d0.clone();
    let r = rip.clone();
    let p = plan_rebased(
        &ApplyInput { text, current: d1, base_for: &move |_| Ok(b.clone()), bundle: None, prefer, now_ms: 1_000 },
        &move |rev| if rev < 200 { vec![r.clone()] } else { vec![] },
    )?;
    let out = if p.ops.is_empty() { d1.clone() } else { apply_batch(d1, &p.ops).unwrap().doc };
    Ok((p, out))
}

/// The projection of `d` with item `id` moved to `[start, end)` and given opacity 0.5, plus a new item.
fn agent_edit(d: &Document, id: &str, start: Micros, end: Micros, new_at: Option<Micros>) -> String {
    let text = print(d, &Default::default());
    let mut out = String::new();
    let mut added = false;
    let mut in_v1 = false;
    for line in text.lines() {
        if line.contains("<track id=\"v1\"") {
            in_v1 = true;
        }
        if in_v1 && line.trim() == "</track>" {
            if let (Some(t), false) = (new_at, added) {
                out.push_str(&format!(
                    "      <text id=\"new1\" start={{{}}} end={{{}}} textContent=\"N\" />\n",
                    micros_to_json(t),
                    micros_to_json(t + S)
                ));
                added = true;
            }
            in_v1 = false;
        }
        if line.contains(&format!(" id=\"{id}\" ")) {
            let it = d.item(id).unwrap();
            let l = line
                .replacen(
                    &format!("start={{{}}}", micros_to_json(it.start)),
                    &format!("start={{{}}}", micros_to_json(start)),
                    1,
                )
                .replacen(
                    &format!("end={{{}}}", micros_to_json(it.end)),
                    &format!("end={{{}}}", micros_to_json(end)),
                    1,
                )
                .replacen(&format!(" id=\"{id}\""), &format!(" id=\"{id}\" opacity={{0.5}}"), 1);
            assert_ne!(l, line);
            out.push_str(&l);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// The sequential result: the agent's edit made on the rippled document.
fn sequential(d1: &Document, id: &str, start: Micros, end: Micros, new_at: Option<Micros>) -> Document {
    let mut ops = vec![Op::UpdateItem {
        id: id.into(),
        set: json!({"startTime": micros_to_json(start), "endTime": micros_to_json(end), "opacity": 0.5})
            .as_object()
            .unwrap()
            .clone(),
    }];
    if let Some(t) = new_at {
        ops.push(Op::AddItem {
            item: json!({"id": "new1", "trackId": "v1", "type": "text", "startTime": micros_to_json(t),
                "endTime": micros_to_json(t + S), "textContent": "N"}),
        });
    }
    apply_batch(d1, &ops).unwrap().doc
}

fn arb_items() -> impl Strategy<Value = Vec<(i64, i64, bool)>> {
    prop::collection::vec((0i64..30, 5i64..60, any::<bool>()), 2..8)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), ..ProptestConfig::default() })]

    /// GUI ripple, then an agent edit printed before it, equals the agent edit made after the ripple; added items
    /// keep the times the file gives them.
    #[test]
    fn gui_ripple_then_agent_edit_equals_sequential(
        items in arb_items(),
        music_sync in any::<bool>(),
        kind in 0usize..3,
        pick in 0usize..8,
        target in 0usize..8,
        frames in -20i64..20,
        new_at in prop::option::of(0i64..400),
    ) {
        let d0 = build(&items, music_sync);
        let ids: Vec<String> = d0.items_on_track("v1").iter().map(|i| i.id.clone()).collect();
        let y = &d0.item(&ids[pick % ids.len()]).unwrap().clone();
        let req = match kind {
            0 => ove_edit::Request::RippleDelete { items: vec![y.id.clone()], tracks: vec![], ranges: vec![], linked: true },
            1 => ove_edit::Request::RippleTrim { item: y.id.clone(), edge: ove_edit::Edge::End, to: y.end - 3 * F, linked: true },
            _ => ove_edit::Request::Insert {
                track: "v1".into(),
                at: y.start,
                items: vec![json!({"type": "text", "startTime": 0, "endTime": 2, "textContent": "I"})],
            },
        };
        let Some((d1, rip)) = ripple(&d0, &req) else { return Ok(()) };
        let x0 = d0.item(&ids[target % ids.len()]).unwrap().clone();
        // The agent edits an item the ripple only moved (not removed, not cut).
        let Some(x1) = d1.item(&x0.id) else { return Ok(()) };
        if x1.end - x1.start != x0.end - x0.start || x1.time_field("trimStart") != x0.time_field("trimStart") {
            return Ok(());
        }
        let k = frames * F;
        if x0.start + k < 0 || x1.start + k < 0 {
            return Ok(());
        }
        let new_at = new_at.map(|f| f * F);
        let text = agent_edit(&d0, &x0.id, x0.start + k, x0.end + k, new_at);
        let (plan, got) = apply(&text, &d0, &d1, &rip, Prefer::File).unwrap();
        prop_assert!(plan.conflicts.is_empty(), "{:?}", plan.conflicts);
        let want = sequential(&d1, &x0.id, x1.start + k, x1.end + k, new_at);
        prop_assert!(got.content_eq(&want), "rebased result differs from the sequential one\ngot {:?}\nwant {:?}",
            got.item(&x0.id), want.item(&x0.id));
        if let Some(t) = new_at {
            prop_assert_eq!(got.item("new1").unwrap().start, t);
        }
    }

    /// Conflicts with a ripple are reported as one group, and the side kept is the same for the whole group.
    #[test]
    fn conflicts_with_a_ripple_are_one_group(items in arb_items(), a in 0i64..200, l in 1i64..80, editor in any::<bool>()) {
        let d0 = build(&items, true);
        let range = ove_edit::Range { start: a * F, end: (a + l) * F };
        let Some((d1, rip)) = ripple(&d0, &ove_edit::Request::Extract { tracks: vec!["v1".into()], range }) else {
            return Ok(());
        };
        // Force conflicts: the file changes the in point of every item whose in point the ripple changed.
        let cut: Vec<String> = d0
            .items_on_track("v1")
            .iter()
            .filter(|i| d1.item(&i.id).is_some_and(|j| j.time_field("trimStart") != i.time_field("trimStart")))
            .map(|i| i.id.clone())
            .collect();
        let mut text = print(&d0, &Default::default());
        for id in &cut {
            let it = d0.item(id).unwrap();
            let from = format!("in={{{}}}", micros_to_json(it.time_field("trimStart")));
            let to = format!("in={{{}}}", micros_to_json(it.time_field("trimStart") + 3 * S));
            let line = text.lines().find(|l| l.contains(&format!(" id=\"{id}\" "))).unwrap().to_string();
            text = text.replacen(&line, &line.replacen(&from, &to, 1), 1);
        }
        let prefer = if editor { Prefer::Editor } else { Prefer::File };
        let (plan, got) = apply(&text, &d0, &d1, &rip, prefer).unwrap();
        let in_points: Vec<_> = plan.conflicts.iter().filter(|c| c.field == "trimStart").collect();
        prop_assert_eq!(in_points.len(), cut.len());
        for c in &plan.conflicts {
            prop_assert_eq!(c.group.as_deref(), Some("ripple 200"), "{:?}", c);
            prop_assert_eq!(c.kept, prefer);
        }
        for id in &cut {
            let want = if editor { d1.item(id).unwrap().time_field("trimStart") } else { d0.item(id).unwrap().time_field("trimStart") + 3 * S };
            prop_assert_eq!(got.item(id).unwrap().time_field("trimStart"), want);
        }
    }
}

/// Spec scenario "Agent edit after a GUI ripple" and "Added items are not mapped".
#[test]
fn agent_edit_after_a_gui_ripple() {
    // A [0,2), gap-free B [2,4) … the user ripple-deletes 2 s before B.
    let d0 = build(&[(0, 50, false), (0, 50, false), (0, 50, false)], true);
    let (d1, rip) = ripple(
        &d0,
        &ove_edit::Request::RippleDelete { items: vec!["x0".into()], tracks: vec![], ranges: vec![], linked: true },
    )
    .unwrap();
    assert_eq!(d1.item("x1").unwrap().start, 0);
    // The agent printed before the ripple: B is at [2, 4); it moves B 1 s later and sets its opacity, and adds an
    // item at 20 s.
    let text = agent_edit(&d0, "x1", 3 * S, 5 * S, Some(20 * S));
    let (_, got) = apply(&text, &d0, &d1, &rip, Prefer::File).unwrap();
    let b = got.item("x1").unwrap();
    assert_eq!((b.start, b.end), (S, 3 * S), "1 s later than its rippled position");
    assert_eq!(b.fields["opacity"], json!(0.5));
    assert_eq!(got.item("new1").unwrap().start, 20 * S);
    // Without the rebase the same file conflicts on B's times.
    let (p, _) = {
        let b0 = d0.clone();
        let p = ove_projection::plan(&ApplyInput {
            text: &text,
            current: &d1,
            base_for: &move |_| Ok(b0.clone()),
            bundle: None,
            prefer: Prefer::File,
            now_ms: 1,
        })
        .unwrap();
        (p, ())
    };
    assert!(!p.conflicts.is_empty());
    let _: Option<Value> = None;
    let _ = TimeMap::default();
}
