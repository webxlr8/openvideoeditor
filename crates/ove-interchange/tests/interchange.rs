//! Round trips through every format and import into a project.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_interchange::{ClipKind, ExportReport, Format, Rate, export, export_report, export_with_report, read, to_ops};
use serde_json::json;

/// Text title (top), image overlay, Main with three clips (trim, speed 2×, a gap), music on Audio.
fn doc() -> Document {
    let ops: Vec<Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "t_text", "name": "Text", "order": 0, "type": "video"}},
        {"op": "addTrack", "track": {"id": "t_overlay", "name": "Overlay", "order": 1, "type": "video"}},
        {"op": "addTrack", "track": {"id": "t_main", "name": "Main", "order": 2, "type": "video"}},
        {"op": "addTrack", "track": {"id": "t_audio", "name": "Audio", "order": 3, "type": "audio"}},
        {"op": "addMedia", "media": {"id": "m1", "data": {"type": "video", "fileName": "Day 1 take #2.mov", "duration": 20, "hasAudio": true}},
         "entry": {"mediaId": "m1", "nativePath": "/Volumes/Footage/Day 1 take #2.mov", "ownedByBundle": false}},
        {"op": "addMedia", "media": {"id": "m2", "data": {"type": "video", "fileName": "broll.mp4", "duration": 10, "hasAudio": false}},
         "entry": {"mediaId": "m2", "nativePath": "/Volumes/Footage/broll.mp4", "ownedByBundle": false}},
        {"op": "addMedia", "media": {"id": "m3", "data": {"type": "image", "fileName": "logo.png"}},
         "entry": {"mediaId": "m3", "nativePath": "/Volumes/Footage/logo.png", "ownedByBundle": false}},
        {"op": "addMedia", "media": {"id": "m4", "data": {"type": "audio", "fileName": "score.wav", "duration": 60}},
         "entry": {"mediaId": "m4", "nativePath": "/Volumes/Footage/score.wav", "ownedByBundle": false}},
        {"op": "addItem", "item": {"id": "a", "trackId": "t_main", "type": "video", "startTime": 0, "endTime": 4, "name": "Intro", "mediaItemId": "m1", "trimStart": 1.5}},
        {"op": "addItem", "item": {"id": "b", "trackId": "t_main", "type": "video", "startTime": 4, "endTime": 6, "name": "Fast", "mediaItemId": "m2", "playbackSpeed": 2}},
        {"op": "addItem", "item": {"id": "c", "trackId": "t_main", "type": "video", "startTime": 7, "endTime": 10, "name": "Outro", "mediaItemId": "m1", "trimStart": 12}},
        {"op": "addItem", "item": {"id": "o", "trackId": "t_overlay", "type": "image", "startTime": 1, "endTime": 3, "name": "Logo", "mediaItemId": "m3"}},
        {"op": "addItem", "item": {"id": "t", "trackId": "t_text", "type": "text", "startTime": 0.5, "endTime": 2.5, "name": "Title", "textContent": "Hello & <welcome>"}},
        {"op": "addItem", "item": {"id": "s", "trackId": "t_audio", "type": "audio", "startTime": 0, "endTime": 10, "name": "Score", "mediaItemId": "m4"}}
    ]))
    .unwrap();
    apply_batch(&Document::new("p", "Day 1 & cut", 1), &ops).unwrap().doc
}

fn main_cuts(t: &ove_interchange::Timeline) -> Vec<(i64, i64, i64, String)> {
    let v1 = t.video.first().expect("V1");
    v1.clips.iter().map(|c| (c.start, c.end, c.source_in, format!("{:.2}", c.speed))).collect()
}

#[test]
fn every_format_round_trips_the_main_cut() {
    let d = doc();
    let want = vec![(0, 120, 45, "1.00".into()), (120, 180, 0, "2.00".into()), (210, 300, 360, "1.00".into())];
    for f in [Format::Fcpxml, Format::Xml, Format::Edl, Format::Otio] {
        let text = export(&d, None, f, Rate::FPS30);
        let t = read(&text, f, Rate::FPS30).unwrap_or_else(|e| panic!("{f:?}: {e}\n{text}"));
        assert_eq!(main_cuts(&t), want, "{f:?}\n{text}");
        // Media paths survive (EDL via SOURCE FILE comments).
        assert_eq!(
            t.video[0].clips[0].path.as_deref(),
            Some(std::path::Path::new("/Volumes/Footage/Day 1 take #2.mov")),
            "{f:?}"
        );
        // Music on A1.
        assert_eq!(t.audio.first().map(|a| a.clips.len()), Some(1), "{f:?}");
    }
}

#[test]
fn rich_formats_keep_overlays_and_titles() {
    let d = doc();
    for f in [Format::Fcpxml, Format::Xml, Format::Otio] {
        let t = read(&export(&d, None, f, Rate::FPS30), f, Rate::FPS30).unwrap();
        let all: Vec<_> = t.video.iter().flat_map(|tr| &tr.clips).collect();
        let title = all.iter().find(|c| c.kind == ClipKind::Title).unwrap_or_else(|| panic!("{f:?} lost the title"));
        assert_eq!(title.text.as_deref(), Some("Hello & <welcome>"), "{f:?}");
        assert_eq!((title.start, title.end), (15, 75), "{f:?}");
        let logo = all
            .iter()
            .find(|c| c.path.as_ref().is_some_and(|p| p.ends_with("logo.png")))
            .unwrap_or_else(|| panic!("{f:?} lost the overlay"));
        assert_eq!((logo.start, logo.end), (30, 90), "{f:?}");
        assert!(t.video.len() >= 3, "{f:?}: {} video tracks", t.video.len());
    }
}

#[test]
fn ntsc_rates_and_fcpxml_times() {
    let d = doc();
    let r = Rate::from_fps(29.97);
    let text = export(&d, None, Format::Fcpxml, r);
    assert!(text.contains("frameDuration=\"1001/30000s\""), "{text}");
    let t = read(&text, Format::Fcpxml, r).unwrap();
    assert_eq!(t.rate, r);
    assert_eq!(t.video[0].clips[0].end, r.frames(4_000_000));
    assert_eq!(ove_interchange::fcpxml::t(120, Rate::FPS30), "4s");
    assert_eq!(ove_interchange::fcpxml::t(1, r), "1001/30000s");
}

#[test]
fn import_adds_tracks_media_and_clips_as_one_valid_batch() {
    let src = doc();
    let otio = export(&src, None, Format::Otio, Rate::FPS30);
    let t = read(&otio, Format::Otio, Rate::FPS30).unwrap();
    // Into a project that already has one track and one of the media files.
    let mut target = Document::new("q", "Target", 1);
    target = apply_batch(
        &target,
        &serde_json::from_value::<Vec<Op>>(json!([
            {"op": "addTrack", "track": {"id": "t_main", "name": "Main", "order": 0, "type": "video"}},
            {"op": "addMedia", "media": {"id": "mx", "data": {"type": "video", "fileName": "broll.mp4"}},
             "entry": {"mediaId": "mx", "nativePath": "/Volumes/Footage/broll.mp4", "ownedByBundle": false}}
        ]))
        .unwrap(),
    )
    .unwrap()
    .doc;
    let (ops, summary) = to_ops(&target, &t, 1_700_000_000_000);
    let after = apply_batch(&target, &ops).expect("import batch is valid").doc;
    assert_eq!(summary.clips, 6);
    assert_eq!(summary.media_added, 3, "broll.mp4 is reused");
    assert!(summary.missing.is_empty());
    assert_eq!(after.project.tracks.len(), 1 + summary.tracks);
    // Imported video tracks sit above the existing ones, audio at the bottom.
    assert!(after.project.tracks[0].name.starts_with("Imported"));
    assert_eq!(after.project.tracks.last().unwrap().kind, "audio");
    let fast = after.project.items.values().find(|i| i.name() == "Fast").unwrap();
    assert_eq!(fast.media_id(), Some("mx"));
    assert_eq!(fast.fields["playbackSpeed"], json!(2.0));
    let intro = after.project.items.values().find(|i| i.name() == "Intro").unwrap();
    assert_eq!((intro.start, intro.end, intro.time_field("trimStart")), (0, 4_000_000, 1_500_000));
}

#[test]
fn edl_without_paths_matches_by_name_or_becomes_pending() {
    let edl = "TITLE: Cut\nFCM: NON-DROP FRAME\n\n001  AX       V     C        00:00:01:00 00:00:03:00 01:00:00:00 01:00:02:00\n* FROM CLIP NAME: broll.mp4\n\n002  AX       V     C        00:00:00:00 00:00:01:00 01:00:02:00 01:00:03:00\n* FROM CLIP NAME: lost.mov\n";
    let t = read(edl, Format::Edl, Rate::FPS30).unwrap();
    assert_eq!(main_cuts(&t), vec![(0, 60, 30, "1.00".into()), (60, 90, 0, "1.00".into())]);
    let target = apply_batch(
        &Document::new("q", "T", 1),
        &serde_json::from_value::<Vec<Op>>(json!([
            {"op": "addMedia", "media": {"id": "mx", "data": {"type": "video", "fileName": "broll.mp4"}}}
        ]))
        .unwrap(),
    )
    .unwrap()
    .doc;
    let (ops, summary) = to_ops(&target, &t, 1);
    let after = apply_batch(&target, &ops).unwrap().doc;
    assert_eq!(summary.missing, vec!["lost.mov".to_string()]);
    assert_eq!(after.project.items.values().filter(|i| i.is_pending()).count(), 1);
    assert_eq!(after.project.items.values().filter(|i| i.media_id() == Some("mx")).count(), 1);
}

#[test]
fn malformed_inputs_are_errors_not_panics() {
    for (f, text) in [
        (Format::Fcpxml, "<fcpxml><resources/></fcpxml>"),
        (Format::Fcpxml, "not xml"),
        (Format::Xml, "<other/>"),
        (Format::Otio, "{\"OTIO_SCHEMA\": \"Clip.2\"}"),
        (Format::Otio, "[]"),
    ] {
        assert!(read(text, f, Rate::FPS30).is_err(), "{f:?}: {text}");
    }
    assert!(read("garbage\n001 x", Format::Edl, Rate::FPS30).unwrap().video.is_empty());
}

/// Writes the four files for external validation (`OVE_INTERCHANGE_OUT=dir`).
#[test]
fn write_samples_when_asked() {
    let Ok(dir) = std::env::var("OVE_INTERCHANGE_OUT") else { return };
    let d = doc();
    for f in [Format::Fcpxml, Format::Xml, Format::Edl, Format::Otio] {
        std::fs::write(
            std::path::Path::new(&dir).join(format!("sample.{}", f.extension())),
            export(&d, None, f, Rate::FPS30),
        )
        .unwrap();
    }
}

/// Files written by the OpenTimelineIO adapters (from our own exports) read back with the same cut.
#[test]
fn reads_files_written_by_opentimelineio() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    // EDL: zero record start, `* FROM CLIP: file://…` paths, M2 with a clip name instead of a reel.
    let edl = std::fs::read_to_string(fixtures.join("otio-adapter.edl")).unwrap();
    let t = read(&edl, Format::Edl, Rate::FPS30).unwrap();
    assert_eq!(
        main_cuts(&t),
        vec![(0, 120, 45, "1.00".into()), (120, 180, 0, "2.00".into()), (210, 300, 360, "1.00".into())]
    );
    assert_eq!(t.video[0].clips[0].path.as_deref(), Some(std::path::Path::new("/Volumes/Footage/Day 1 take #2.mov")));
    assert_eq!(t.video[0].clips[1].name, "Fast");
    // OTIO converted from our FCP7 XML by the adapter (carries fcp_xml metadata, rate in every time).
    let otio = std::fs::read_to_string(fixtures.join("otio-adapter-from-xml.otio")).unwrap();
    let t = read(&otio, Format::Otio, Rate::FPS30).unwrap();
    assert_eq!(
        main_cuts(&t),
        vec![(0, 120, 45, "1.00".into()), (120, 180, 0, "2.00".into()), (210, 300, 360, "1.00".into())],
        "{t:#?}"
    );
    assert!(t.audio.iter().any(|a| a.clips.iter().any(|c| c.path.as_ref().is_some_and(|p| p.ends_with("score.wav")))));
}

/// `doc()` plus what timeline files can't hold: a caption, motion graphics, a shader and an adjustment layer, a
/// placeholder, a hidden clip, and clips using effects, a transform, volume and a transition.
fn rich_doc() -> Document {
    let ops: Vec<Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "t_cap", "name": "Captions", "order": 0, "type": "caption"}},
        {"op": "addTrack", "track": {"id": "t_adj", "name": "Adjust", "order": 1, "type": "adjustment"}},
        {"op": "addItem", "item": {"id": "cap", "trackId": "t_cap", "type": "caption", "startTime": 0, "endTime": 2, "captionText": "Hello"}},
        {"op": "addItem", "item": {"id": "adj", "trackId": "t_adj", "type": "adjustment", "startTime": 0, "endTime": 2}},
        {"op": "addItem", "item": {"id": "code", "trackId": "t_text", "type": "code", "startTime": 5, "endTime": 7, "codeComposition": {"template": "lower-third"}}},
        {"op": "addItem", "item": {"id": "sh", "trackId": "t_text", "type": "shader", "startTime": 8, "endTime": 9, "shaderComposition": {"template": "aurora"}}},
        {"op": "addItem", "item": {"id": "pend", "trackId": "t_overlay", "type": "image", "startTime": 5, "endTime": 6, "pending": true, "pendingDescription": "A logo"}},
        {"op": "addItem", "item": {"id": "hid", "trackId": "t_overlay", "type": "image", "startTime": 7, "endTime": 8, "mediaItemId": "m3", "hidden": true}},
        {"op": "updateItem", "id": "a", "set": {"effects": [{"id": "duotone", "params": {}}], "transitionIn": {"type": "crossfade", "duration": 0.5}}},
        {"op": "updateItem", "id": "o", "set": {"transform": {"x": 100, "y": 0, "scale": 0.5}}},
        {"op": "updateItem", "id": "s", "set": {"volume": 40}}
    ]))
    .unwrap();
    apply_batch(&doc(), &ops).unwrap().doc
}

fn omitted(r: &ExportReport, kind: &str) -> usize {
    r.omitted.iter().filter(|o| o.kind == kind).map(|o| o.count).sum()
}

#[test]
fn report_is_empty_when_nothing_is_lost() {
    let d = doc();
    for f in [Format::Fcpxml, Format::Xml] {
        let r = export_report(&d, None, f, Rate::FPS30);
        assert!(r.is_empty(), "{f:?}: {r:?}");
    }
}

#[test]
fn report_lists_items_left_out_and_properties_not_carried() {
    let d = rich_doc();
    for f in [Format::Fcpxml, Format::Xml, Format::Edl, Format::Otio] {
        let r = export_report(&d, None, f, Rate::FPS30);
        for kind in
            ["captions", "motion graphics", "shader layers", "adjustment layers", "placeholders", "hidden items"]
        {
            assert_eq!(omitted(&r, kind), 1, "{f:?} {kind}: {r:?}");
        }
        assert!(r.omitted.iter().all(|o| !o.reason.is_empty()), "{f:?}");
        for p in ["effects", "position, scale and rotation", "volume", "transitions"] {
            assert!(r.not_carried.iter().any(|x| x == p), "{f:?} {p}: {r:?}");
        }
        assert!(!r.not_carried.iter().any(|x| x == "crop"), "{f:?}: only used properties are listed");
        // The report written with the file is the preview's.
        let e = export_with_report(&d, None, f, Rate::FPS30);
        assert_eq!(e.report, r, "{f:?}");
        assert_eq!(e.text, export(&d, None, f, Rate::FPS30), "{f:?}");
    }
}

#[test]
fn report_says_what_each_format_drops() {
    let d = doc();
    // EDL: one video track and no titles; the logo on V2 and the title on V3 are left out.
    let edl = export_report(&d, None, Format::Edl, Rate::FPS30);
    assert_eq!(omitted(&edl, "video clips above the main track"), 1, "{edl:?}");
    assert_eq!(omitted(&edl, "titles"), 1, "{edl:?}");
    assert!(edl.not_carried.is_empty(), "{edl:?}");
    // At NTSC rates the EDL's non-drop-frame timecode is called out.
    let ntsc = export_report(&d, None, Format::Edl, Rate::from_fps(29.97));
    assert!(ntsc.not_carried.iter().any(|p| p.starts_with("drop-frame timecode")), "{ntsc:?}");
    // OTIO keeps titles only as empty clips.
    let otio = export_report(&d, None, Format::Otio, Rate::FPS30);
    assert_eq!(omitted(&otio, "titles"), 1, "{otio:?}");
    // FCPXML and XML keep both.
    for f in [Format::Fcpxml, Format::Xml] {
        let r = export_report(&d, None, f, Rate::FPS30);
        assert_eq!(omitted(&r, "titles") + omitted(&r, "video clips above the main track"), 0, "{f:?}");
    }
}

#[test]
fn report_notes_vertical_crop_muted_tracks_and_title_styling() {
    let ops: Vec<Op> = serde_json::from_value(json!([
        {"op": "setAspectRatio", "aspectRatio": "9:16"},
        {"op": "updateTrack", "id": "t_audio", "set": {"muted": true}},
        {"op": "updateItem", "id": "t", "set": {"fontFamily": "Inter", "fontSize": 96}}
    ]))
    .unwrap();
    let d = apply_batch(&doc(), &ops).unwrap().doc;
    let r = export_report(&d, None, Format::Fcpxml, Rate::FPS30);
    for p in ["fill-the-frame crop", "muted tracks (written as audible)", "title styling (font, size, colours)"] {
        assert!(r.not_carried.iter().any(|x| x == p), "{p}: {r:?}");
    }
}
