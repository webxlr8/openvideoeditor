//! Export never damages what is already on disk: it writes `name.partial.ext`, renames over the destination only
//! when complete, and removes only its temporary file on failure or cancel. Export is refused while media is
//! offline. Uses a text-only timeline, so no media fixtures (or ffmpeg CLI) are needed.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_render::export::{encoder_warning, offline_media, partial_path};
use ove_render::{ExportError, ExportSettings, export};
use serde_json::json;

const EARLIER: &[u8] = b"an earlier export the user wants to keep";

fn title_doc() -> Document {
    let ops = vec![
        Op::AddTrack { track: json!({"id": "text", "name": "Text", "order": 0, "type": "video"}) },
        Op::AddItem {
            item: json!({"id": "t", "trackId": "text", "type": "text", "startTime": 0, "endTime": 0.6, "textContent": "HI"}),
        },
    ];
    apply_batch(&Document::new("p", "Safety", 1), &ops).unwrap().doc
}

fn small() -> ExportSettings {
    ExportSettings { height: 90, fps: 10, ..Default::default() }
}

/// No `.partial` file is left in `dir`.
fn no_partials(dir: &Path) {
    let left: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".partial"))
        .collect();
    assert!(left.is_empty(), "temporary files left: {left:?}");
}

#[test]
fn partial_path_is_a_sibling() {
    assert_eq!(partial_path(Path::new("/v/My Film.mp4")), Path::new("/v/My Film.partial.mp4"));
    assert_eq!(partial_path(Path::new("/v/film")), Path::new("/v/film.partial"));
}

#[test]
fn success_replaces_the_target_atomically() {
    let d = tempfile::tempdir().unwrap();
    let out = d.path().join("film.mp4");
    std::fs::write(&out, EARLIER).unwrap();
    let mut encoders = Vec::new();
    let r = export(&title_doc(), None, &out, &small(), &AtomicBool::new(false), |p| encoders.push(p.encoder)).unwrap();
    assert_eq!(r.path, out.display().to_string());
    let bytes = std::fs::read(&out).unwrap();
    assert_ne!(bytes, EARLIER);
    assert_eq!(&bytes[4..8], b"ftyp", "an MP4 replaced the earlier file");
    no_partials(d.path());
    // The encoder is reported during progress and in the result, with a warning only for a fallback encoder.
    assert!(!encoders.is_empty() && encoders.iter().all(|e| *e == r.video_encoder));
    assert_eq!(r.encoder_warning, encoder_warning(&r.video_encoder));
    eprintln!("encoder {} warning {:?}", r.video_encoder, r.encoder_warning);
}

#[test]
fn cancel_keeps_the_earlier_file() {
    let d = tempfile::tempdir().unwrap();
    let out = d.path().join("film.mp4");
    std::fs::write(&out, EARLIER).unwrap();
    let cancel = AtomicBool::new(false);
    let mut partial_seen = false;
    let r = export(&title_doc(), None, &out, &small(), &cancel, |p| {
        // Mid-export: the work goes to the temporary file, the earlier export is untouched.
        partial_seen |= partial_path(&out).exists();
        if p.frame == 2 {
            cancel.store(true, Ordering::SeqCst);
        }
    });
    assert!(matches!(r, Err(ExportError::Cancelled)), "{r:?}");
    assert!(partial_seen, "export wrote to the temporary file");
    assert_eq!(std::fs::read(&out).unwrap(), EARLIER);
    no_partials(d.path());
}

#[test]
fn failure_keeps_the_earlier_file() {
    let d = tempfile::tempdir().unwrap();
    let out = d.path().join("film.mp4");
    std::fs::write(&out, EARLIER).unwrap();
    // Fails while opening the encoder.
    let bad = ExportSettings { encoder: Some("no-such-encoder".into()), ..small() };
    let e = export(&title_doc(), None, &out, &bad, &AtomicBool::new(false), |_| {}).unwrap_err();
    assert!(matches!(e, ExportError::Ffmpeg(_)), "{e}");
    assert_eq!(std::fs::read(&out).unwrap(), EARLIER);
    no_partials(d.path());

    // Fails after encoding everything, when the finished file cannot take the destination's place (a folder).
    let folder = d.path().join("taken.mp4");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("keep.txt"), EARLIER).unwrap();
    let e = export(&title_doc(), None, &folder, &small(), &AtomicBool::new(false), |_| {}).unwrap_err();
    assert!(e.to_string().contains("could not save"), "{e}");
    assert_eq!(std::fs::read(folder.join("keep.txt")).unwrap(), EARLIER);
    no_partials(d.path());
}

#[test]
fn offline_media_refuses_export_by_name() {
    let d = tempfile::tempdir().unwrap();
    let gone = d.path().join("Interview A.mov");
    let ops = vec![
        Op::AddTrack { track: json!({"id": "main", "name": "Main", "order": 1, "type": "video"}) },
        Op::AddMedia {
            media: json!({"id": "m1", "data": {"type": "video", "fileName": "Interview A.mov", "duration": 2}}),
            entry: Some(json!({"mediaId": "m1", "nativePath": gone.display().to_string(), "ownedByBundle": false})),
        },
        Op::AddItem {
            item: json!({"id": "c", "trackId": "main", "type": "video", "startTime": 0, "endTime": 1, "mediaItemId": "m1"}),
        },
    ];
    let doc = apply_batch(&title_doc(), &ops).unwrap().doc;
    assert_eq!(offline_media(&doc, None), vec!["Interview A.mov".to_string()]);

    let out = d.path().join("film.mp4");
    std::fs::write(&out, EARLIER).unwrap();
    let e = export(&doc, None, &out, &small(), &AtomicBool::new(false), |_| {}).unwrap_err();
    assert!(matches!(e, ExportError::Refused(_)), "{e}");
    assert!(e.to_string().contains("offline") && e.to_string().contains("“Interview A.mov”"), "{e}");
    assert_eq!(std::fs::read(&out).unwrap(), EARLIER);
    no_partials(d.path());

    // Back online: nothing is refused.
    std::fs::write(&gone, b"x").unwrap();
    assert!(offline_media(&doc, None).is_empty());
}

#[test]
fn only_fallback_encoders_warn() {
    for ok in ["h264_videotoolbox", "h264_nvenc", "h264_mf", "libopenh264", "libx264"] {
        assert_eq!(encoder_warning(ok), None, "{ok}");
    }
    assert!(encoder_warning("mpeg4").unwrap().contains("MPEG-4 Part 2"));
}
