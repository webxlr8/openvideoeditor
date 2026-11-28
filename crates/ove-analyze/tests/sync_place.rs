//! Task 2.4: placement rules on synthetic recordings — sound and timecode placement, confidence order and anchor
//! chaining, the group shift, overlap refusal, drift retiming of audio-only items, and unmatched targets.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::collections::BTreeMap;

use ove_analyze::sync::{Source, SyncParams, envelope};
use ove_analyze::sync_place::{MediaAudio, Mode, SyncPlan, plan};
use ove_analyze::synth::{Recording, Scene};
use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use serde_json::{Value, json};

struct Fake(BTreeMap<String, Recording>);

impl MediaAudio for Fake {
    fn envelope(&mut self, id: &str) -> ove_analyze::Result<Vec<f32>> {
        Ok(envelope(&self.0[id].envelope()))
    }
    fn source(&self, id: &str) -> ove_analyze::Result<Box<dyn Source>> {
        Ok(Box::new(self.0[id].clone()))
    }
}

/// A document from media `(id, type, seconds, timecode)` and items `(id, track, media, start, end, extra)`.
fn doc(media: &[(&str, &str, f64, Option<Value>)], items: &[(&str, &str, &str, f64, f64, Value)]) -> Document {
    let mut ops = vec![
        json!({"op": "addTrack", "track": {"id": "V", "name": "Main", "order": 0, "type": "video"}}),
        json!({"op": "addTrack", "track": {"id": "A", "name": "Audio", "order": 1, "type": "audio"}}),
        json!({"op": "addTrack", "track": {"id": "B", "name": "Audio 2", "order": 2, "type": "audio"}}),
    ];
    for (id, kind, secs, tc) in media {
        let mut data = json!({"type": kind, "fileName": format!("{id}.wav"), "duration": secs, "hasAudio": true});
        if let Some(tc) = tc {
            data["sourceTimecode"] = tc.clone();
        }
        ops.push(json!({"op": "addMedia", "media": {"id": id, "data": data},
            "entry": {"mediaId": id, "nativePath": format!("/x/{id}.wav"), "ownedByBundle": false}}));
    }
    for (id, track, m, s, e, extra) in items {
        let kind = if *track == "V" { "video" } else { "audio" };
        let mut v = json!({"id": id, "trackId": track, "type": kind, "startTime": s, "endTime": e, "mediaItemId": m});
        if let Value::Object(x) = extra {
            for (k, val) in x {
                v[k] = val.clone();
            }
        }
        ops.push(json!({"op": "addItem", "item": v}));
    }
    let ops: Vec<Op> = serde_json::from_value(Value::Array(ops)).unwrap();
    apply_batch(&Document::new("p", "t", 1), &ops).unwrap().doc
}

fn after(d: &Document, p: &SyncPlan) -> Document {
    apply_batch(d, &p.ops).unwrap().doc
}

fn start(d: &Document, id: &str) -> f64 {
    d.item(id).unwrap().start as f64 / 1e6
}

fn target<'a>(p: &'a SyncPlan, id: &str) -> &'a ove_analyze::sync_place::TargetResult {
    p.targets.iter().find(|t| t.item == id).unwrap()
}

fn fake(list: &[(&str, Recording)]) -> Fake {
    Fake(list.iter().map(|(k, r)| (k.to_string(), r.clone())).collect())
}

const NONE: Value = Value::Null;

#[test]
fn sound_places_the_recorder_and_trim_is_respected() {
    let scene = Scene::speech(7);
    let mut f = fake(&[
        ("cam", Recording::new(scene, 0.0, 60.0).noise(10.0, 1)),
        ("rec", Recording::new(scene, 12.345, 40.0).lowpass(4_000.0).noise(10.0, 2)),
    ]);
    // The camera clip starts at 5 s and skips its first 2 s; the recorder clip skips its first 1 s.
    let d = doc(
        &[("cam", "video", 60.0, None), ("rec", "audio", 40.0, None)],
        &[("c", "V", "cam", 5.0, 60.0, json!({"trimStart": 2})), ("r", "A", "rec", 0.0, 39.0, json!({"trimStart": 1}))],
    );
    let p = plan(&d, "c", &["r".into()], Mode::Auto, &SyncParams::default(), &mut f).unwrap();
    assert_eq!(p.placed, 1, "{p:?}");
    let t = target(&p, "r");
    assert_eq!((t.status, t.method), ("placed", Some("audio")));
    let d2 = after(&d, &p);
    // Recorder time 1 s (its first frame) is camera time 13.345 s, shown at timeline 5 + (13.345 − 2).
    let expect = 5.0 + 13.345 - 2.0;
    assert!((start(&d2, "r") - expect).abs() < 0.001, "{} vs {expect}", start(&d2, "r"));
    assert!((d2.item("r").unwrap().end - d2.item("r").unwrap().start - 39_000_000).abs() <= 1, "length kept");
    assert_eq!(p.ops.len(), 1, "only the recorder moves");
}

#[test]
fn timecode_mode_places_to_the_sample() {
    let tc =
        |frames: i64| json!({"frames": frames, "rateNum": 25, "rateDen": 1, "dropFrame": false, "source": "container"});
    let wav = json!({"frames": 0, "rateNum": 25, "rateDen": 1, "dropFrame": false, "samples": 48_000u64 * 36_004 + 7, "sampleRate": 48_000, "source": "bwf"});
    let d = doc(
        &[("cam", "video", 60.0, Some(tc(25 * 36_000))), ("rec", "audio", 60.0, Some(wav))],
        &[("c", "V", "cam", 0.0, 60.0, NONE), ("r", "A", "rec", 0.0, 60.0, NONE)],
    );
    let mut f = fake(&[]);
    let p = plan(&d, "c", &["r".into()], Mode::Timecode, &SyncParams::default(), &mut f).unwrap();
    let t = target(&p, "r");
    assert_eq!((t.status, t.method, t.confidence), ("placed", Some("timecode"), 1.0));
    // 4 s and 7 samples later; the item time is kept to the microsecond.
    let d2 = after(&d, &p);
    assert_eq!(d2.item("r").unwrap().start, 4_000_146);
    // Timecode mode without timecode on the target is reported, not guessed.
    let d3 = doc(
        &[("cam", "video", 60.0, Some(tc(0))), ("rec", "audio", 60.0, None)],
        &[("c", "V", "cam", 0.0, 60.0, NONE), ("r", "A", "rec", 0.0, 60.0, NONE)],
    );
    let p = plan(&d3, "c", &["r".into()], Mode::Timecode, &SyncParams::default(), &mut f).unwrap();
    assert_eq!((p.placed, target(&p, "r").status), (0, "unmatched"));
}

#[test]
fn a_target_before_the_start_shifts_the_whole_group_right() {
    let scene = Scene::speech(8);
    // The recorder started 10 s before the camera.
    let mut f = fake(&[
        ("cam", Recording::new(scene, 10.0, 40.0).noise(20.0, 1)),
        ("rec", Recording::new(scene, 0.0, 60.0).noise(20.0, 2)),
    ]);
    let d = doc(
        &[("cam", "video", 40.0, None), ("rec", "audio", 60.0, None)],
        &[("c", "V", "cam", 0.0, 40.0, NONE), ("r", "A", "rec", 30.0, 90.0, NONE)],
    );
    let p = plan(&d, "c", &["r".into()], Mode::Audio, &SyncParams::default(), &mut f).unwrap();
    assert!((p.group_shift - 10.0).abs() < 0.001, "{p:?}");
    let d2 = after(&d, &p);
    assert!((start(&d2, "c") - 10.0).abs() < 0.001);
    assert!(start(&d2, "r").abs() < 0.001);
}

#[test]
fn an_overlapping_move_is_refused_and_names_the_item_in_the_way() {
    let scene = Scene::speech(7);
    let mut f = fake(&[
        ("cam", Recording::new(scene, 0.0, 60.0).noise(10.0, 1)),
        ("rec", Recording::new(scene, 12.345, 40.0).noise(10.0, 2)),
        ("music", Recording::new(Scene::speech(50), 0.0, 10.0)),
    ]);
    let d = doc(
        &[("cam", "video", 60.0, None), ("rec", "audio", 40.0, None), ("music", "audio", 10.0, None)],
        &[
            ("c", "V", "cam", 0.0, 60.0, NONE),
            ("r", "A", "rec", 100.0, 140.0, NONE),
            ("m", "A", "music", 20.0, 25.0, json!({"name": "Sting"})),
        ],
    );
    let p = plan(&d, "c", &["r".into()], Mode::Audio, &SyncParams::default(), &mut f).unwrap();
    let t = target(&p, "r");
    assert_eq!((t.status, t.blocked_by.as_deref()), ("refused", Some("m")), "{t:?}");
    assert!(t.reason.as_deref().unwrap().contains("Sting"));
    assert!(p.ops.is_empty());
}

#[test]
fn a_target_without_overlap_is_placed_through_a_placed_target() {
    let scene = Scene::speech(9);
    // rec1 overlaps the camera; rec2 overlaps only rec1.
    let mut f = fake(&[
        ("cam", Recording::new(scene, 0.0, 60.0).noise(15.0, 1)),
        ("rec1", Recording::new(scene, 40.0, 60.0).noise(15.0, 2)),
        ("rec2", Recording::new(scene, 70.0, 40.0).noise(15.0, 3)),
    ]);
    let d = doc(
        &[("cam", "video", 60.0, None), ("rec1", "audio", 60.0, None), ("rec2", "audio", 40.0, None)],
        &[
            ("c", "V", "cam", 0.0, 60.0, NONE),
            ("r1", "A", "rec1", 200.0, 260.0, NONE),
            ("r2", "B", "rec2", 300.0, 340.0, NONE),
        ],
    );
    let p = plan(&d, "c", &["r2".into(), "r1".into()], Mode::Audio, &SyncParams::default(), &mut f).unwrap();
    assert_eq!(p.placed, 2, "{p:#?}");
    let t2 = target(&p, "r2");
    assert_eq!(t2.anchor.as_deref(), Some("r1"), "{t2:?}");
    // Results keep the order the targets were given in.
    assert_eq!(p.targets[0].item, "r2");
    let d2 = after(&d, &p);
    assert!((start(&d2, "r1") - 40.0).abs() < 0.001, "{}", start(&d2, "r1"));
    assert!((start(&d2, "r2") - 70.0).abs() < 0.001, "{}", start(&d2, "r2"));
}

#[test]
fn drift_retimes_audio_items_but_never_video_items() {
    let scene = Scene::speech(4);
    let clock = 1.0002; // 200 ppm
    let mut f = fake(&[
        ("cam", Recording::new(scene, 0.0, 220.0).noise(15.0, 1)),
        ("rec", Recording::new(scene, 5.0, 200.0).clock(clock).noise(15.0, 2)),
    ]);
    let d = doc(
        &[("cam", "video", 220.0, None), ("rec", "audio", 200.0, None), ("cam2", "video", 200.0, None)],
        &[("c", "V", "cam", 0.0, 220.0, NONE), ("r", "A", "rec", 0.0, 200.0, NONE)],
    );
    let p = plan(&d, "c", &["r".into()], Mode::Audio, &SyncParams::default(), &mut f).unwrap();
    let t = target(&p, "r");
    assert_eq!(t.drift_applied, Some(true), "{t:?}");
    assert!((t.drift_ppm.unwrap() - 200.0).abs() < 3.0, "{t:?}");
    let d2 = after(&d, &p);
    let r = d2.item("r").unwrap();
    let s = r.fields["playbackSpeed"].as_f64().unwrap();
    assert!((s - clock).abs() < 3e-6, "{s}");
    assert!((r.start as f64 / 1e6 - 5.0).abs() < 0.001);
    // The timeline length shrinks by the drift: 200 s of recorder time last 200 / 1.0002 s.
    assert!(((r.end - r.start) as f64 / 1e6 - 200.0 / clock).abs() < 0.001);

    // The same recording as a video item keeps its speed; the drift is reported only.
    f.0.insert("cam2".into(), f.0["rec"].clone());
    let d = doc(
        &[("cam", "video", 220.0, None), ("cam2", "video", 200.0, None)],
        &[("c", "V", "cam", 0.0, 220.0, NONE), ("v", "A", "cam2", 0.0, 200.0, NONE)],
    );
    let mut d = d;
    // Make `v` a video item on its own video track.
    let ops: Vec<Op> = serde_json::from_value(json!([
        {"op": "removeItems", "ids": ["v"]},
        {"op": "addTrack", "track": {"id": "V2", "name": "Cam 2", "order": 3, "type": "video"}},
        {"op": "addItem", "item": {"id": "v", "trackId": "V2", "type": "video", "startTime": 0, "endTime": 200, "mediaItemId": "cam2"}}
    ]))
    .unwrap();
    d = apply_batch(&d, &ops).unwrap().doc;
    let p = plan(&d, "c", &["v".into()], Mode::Audio, &SyncParams::default(), &mut f).unwrap();
    let t = target(&p, "v");
    assert_eq!((t.status, t.drift_applied), ("placed", Some(false)), "{t:?}");
    let d2 = after(&d, &p);
    assert!(d2.item("v").unwrap().fields.get("playbackSpeed").is_none(), "video speed unchanged");
}

#[test]
fn unrelated_sound_is_not_moved() {
    let mut f = fake(&[
        ("cam", Recording::new(Scene::speech(7), 0.0, 60.0).noise(10.0, 1)),
        ("other", Recording::new(Scene::speech(77), 0.0, 30.0).noise(10.0, 2)),
    ]);
    let d = doc(
        &[("cam", "video", 60.0, None), ("other", "audio", 30.0, None)],
        &[("c", "V", "cam", 0.0, 60.0, NONE), ("o", "A", "other", 0.0, 30.0, NONE)],
    );
    let p = plan(&d, "c", &["o".into()], Mode::Auto, &SyncParams::default(), &mut f).unwrap();
    let t = target(&p, "o");
    assert_eq!((p.placed, t.status), (0, "unmatched"), "{t:?}");
    assert!(t.reason.is_some() && p.ops.is_empty());
}
