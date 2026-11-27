//! Export tools (change extend-export-formats-and-queue): `export_video` with the export settings model — a preset,
//! a container, codecs, quality, size, frame rate and a range — shared by `ove export` and MCP.

#[cfg(feature = "media")]
use std::path::Path;

#[cfg(feature = "media")]
use ove_render::export::{
    AudioCodec, AudioSettings, Container, ExportError, ExportSettings, Fps, Quality, Size, VideoCodec, VideoSettings,
};

#[cfg(feature = "media")]
use crate::fail::Fail;
use crate::registry::PROJECT;
#[cfg(feature = "media")]
use crate::spec::Args;
use crate::spec::Kind::{Bool, Int, Json, Num, Str};
use crate::spec::{Arg, arg, req};

const RANGE: &str = r#"{"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2}"#;
const SETTINGS: &str = r#"{"type": "object"}"#;

pub const EXPORT_VIDEO_ABOUT: &str = "Render the timeline (or a range of it) to a video or audio file. Pick a `preset` (see `docs` topic `export`: \"YouTube 1080p\", \"ProRes 422 HQ master\", \"ProRes 4444 with alpha\", \"DNxHR HQ master\", \"Web VP9\", \"Audio WAV 24-bit\", …), or a `format` (mp4, mov, mkv, webm, m4a, wav, flac; default: the extension of `out`) with optional codec fields. HEVC and AV1 need a hardware encoder. Refused while placeholders are pending or media is offline, and for combinations that cannot be written (the error names the fields). Exports run one at a time in a queue shared with the app; the call returns a job id (use `job_status`, `cancel_job`), or the result with `wait`. The result names the encoder used.";

pub const EXPORT_VIDEO_ARGS: &[Arg] = &[
    PROJECT,
    req("out", Str, "Output file path; its extension must match the format"),
    arg("preset", Str, "Named export preset"),
    arg("format", Str, "Container: mp4, mov, mkv, webm, m4a, wav or flac"),
    arg("quality", Str, "very low, low, medium (default), high or very high"),
    arg("resolution", Str, "720p, 1080p (default), 4k, or match (the timeline's own size)"),
    arg("height", Int, "Output height in pixels (instead of resolution)"),
    arg("frameRate", Num, "Frames per second (default 30, or the preset's)"),
    arg("fps", Int, "Same as frameRate"),
    arg("range", Json(RANGE), "[start, end] in seconds: export only this part"),
    arg("videoCodec", Str, "h264, hevc, prores, dnxhr, vp9 or av1"),
    arg("profile", Str, "ProRes: proxy, lt, standard, hq, 4444; DNxHR: lb, sq, hq, hqx, 444; HEVC: main, main10"),
    arg("bitDepth", Int, "Video bit depth: 8, 10 or 12"),
    arg("alpha", Bool, "Keep transparency (ProRes 4444 in MOV, VP9 in WebM)"),
    arg("videoBitrate", Int, "Video bit rate in bits per second (H.264, HEVC, AV1), instead of quality"),
    arg("audioCodec", Str, "aac, pcm, flac, opus, or none"),
    arg("audioBitDepth", Int, "PCM and FLAC: 16 or 24"),
    arg("settings", Json(SETTINGS), "A whole export settings value (see `docs` topic `export`)"),
    arg(
        "wait",
        Bool,
        "Return when the export has finished (default: return the job id at once; follow it with job_status)",
    ),
];

#[cfg(feature = "media")]
fn invalid(message: impl Into<String>) -> Fail {
    Fail::new("export.invalid", message)
}

/// The export settings a call asks for: `settings` (a whole settings value), else `preset`, else the container of
/// `format` or of the output file's extension; then the single fields given override it.
#[cfg(feature = "media")]
pub fn settings_from_args(a: &Args, out: &Path) -> Result<ExportSettings, Fail> {
    let from_ext = Container::from_path(out);
    let mut s = if let Some(v) = a.value("settings") {
        serde_json::from_value::<ExportSettings>(v.clone())
            .map_err(|e| invalid(format!("settings are not an export settings value: {e}")))?
            .with_defaults()
    } else if let Some(name) = a.str("preset") {
        let p = ove_render::export::preset(name).ok_or_else(|| {
            let names: Vec<&str> = ove_render::export::presets().iter().map(|p| p.name.as_str()).collect();
            Fail::new("export.preset.unknown", format!("there is no preset {name:?}; presets: {}", names.join(", ")))
        })?;
        p.settings.clone().with_defaults()
    } else {
        let c = match a.str("format") {
            Some(f) => Container::parse(f)
                .ok_or_else(|| invalid(format!("format {f:?} is not one of mp4, mov, mkv, webm, m4a, wav, flac")))?,
            None => from_ext.unwrap_or(Container::Mp4),
        };
        ExportSettings::for_container(c)
    };
    if (a.has("settings") || a.has("preset"))
        && let Some(f) = a.str("format")
    {
        s.container = Container::parse(f).ok_or_else(|| invalid(format!("format {f:?} is not a container")))?;
    }
    if let Some(c) = a.str("videoCodec") {
        let codec = VideoCodec::parse(c)
            .ok_or_else(|| invalid(format!("video codec {c:?} is not one of h264, hevc, prores, dnxhr, vp9, av1")))?;
        let v = s.video.get_or_insert_with(VideoSettings::default);
        if v.codec != codec {
            v.profile = None;
            v.bit_depth = None;
        }
        v.codec = codec;
    }
    let video_fields = ["profile", "bitDepth", "quality", "alpha", "videoBitrate"];
    if video_fields.iter().any(|k| a.has(k)) {
        let v = s.video.as_mut().ok_or_else(|| {
            invalid(format!(
                "{} is set, but the export has no video",
                video_fields.iter().find(|k| a.has(k)).unwrap_or(&"")
            ))
        })?;
        if let Some(p) = a.str("profile") {
            v.profile = Some(p.to_string());
        }
        if let Some(d) = a.i64("bitDepth") {
            v.bit_depth = Some(u8::try_from(d).map_err(|_| invalid(format!("bit depth {d} is not 8, 10 or 12")))?);
        }
        if let Some(q) = a.str("quality") {
            v.quality = Quality::parse(q).ok_or_else(|| {
                invalid(format!("quality {q:?} is not one of very low, low, medium, high, very high"))
            })?;
        }
        if a.has("alpha") {
            v.alpha = a.bool("alpha");
        }
        if let Some(b) = a.i64("videoBitrate") {
            v.bitrate = Some(u64::try_from(b).map_err(|_| invalid("video bit rate must be positive"))?);
        }
    }
    if let Some(c) = a.str("audioCodec") {
        if c.eq_ignore_ascii_case("none") {
            s.audio = None;
        } else {
            let codec = AudioCodec::parse(c)
                .ok_or_else(|| invalid(format!("audio codec {c:?} is not one of aac, pcm, flac, opus, none")))?;
            let au = s.audio.get_or_insert_with(AudioSettings::default);
            if au.codec != codec {
                au.bit_depth = None;
                au.bitrate = None;
            }
            au.codec = codec;
        }
    }
    if let Some(d) = a.i64("audioBitDepth") {
        let au = s.audio.as_mut().ok_or_else(|| invalid("audioBitDepth is set, but the export has no audio"))?;
        au.bit_depth = Some(u8::try_from(d).map_err(|_| invalid(format!("audio bit depth {d} is not 16 or 24")))?);
    }
    if let Some(h) = a.i64("height") {
        s.size = Size::Height(u32::try_from(h).map_err(|_| invalid(format!("height {h} is not a size")))?);
    }
    if let Some(r) = a.str("resolution") {
        s.size = Size::parse(r)
            .ok_or_else(|| invalid(format!("size {r:?} is not a height (720, 1080p, 4k) or \"match\"")))?;
    }
    if let Some(f) = a.f64("fps").or(a.f64("frameRate")) {
        if f.fract() != 0.0 || !(1.0..=120.0).contains(&f) {
            return Err(invalid(format!("frame rate {f} must be a whole number from 1 to 120")));
        }
        s.fps = Fps::Rate(f as u32);
    }
    if let Some(r) = a.value("range") {
        let pair: Vec<f64> = r.as_array().map(|v| v.iter().filter_map(|x| x.as_f64()).collect()).unwrap_or_default();
        let [start, end] = pair[..] else {
            return Err(invalid("range must be [start, end] in seconds"));
        };
        s.range = Some([start, end]);
    }
    // The file's extension must say what is inside.
    if let Some(ext) = from_ext
        && ext != s.container
    {
        return Err(invalid(format!(
            "{} has the extension of {} but the settings write {}; name the file .{}",
            out.display(),
            ext.label(),
            s.container.label(),
            s.container.extension()
        )));
    }
    Ok(s)
}

#[cfg(feature = "media")]
pub(crate) fn export_fail(e: ExportError) -> Fail {
    let code = match &e {
        ExportError::Refused(_) => "export.refused",
        ExportError::Invalid(_) => "export.invalid",
        ExportError::Cancelled => "export.cancelled",
        ExportError::Ffmpeg(_) => "export.failed",
    };
    let f = Fail::new(code, e.to_string());
    match e {
        ExportError::Invalid(s) => f.details(serde_json::json!({"fields": s.fields})),
        _ => f,
    }
}

/// One line about a finished export (and a warning line for a fallback encoder).
#[cfg(feature = "media")]
pub fn result_text(res: &ove_render::ExportResult) -> String {
    let what = match (&res.video_format, &res.encoder_label) {
        (Some(f), Some(e)) => format!("{}x{} {f}, {e}", res.width, res.height),
        _ => format!("audio only, {}", res.audio_encoder.as_deref().unwrap_or("")),
    };
    let mut text =
        format!("exported {} ({what}, {:.1} s) in {:.1} s", res.path, res.duration_seconds, res.seconds_taken);
    if let Some(w) = &res.encoder_warning {
        text.push_str(&format!("\nwarning: {w}"));
    }
    text
}

#[cfg(all(test, feature = "media"))]
mod tests {
    use super::*;
    use serde_json::{Map, Value, json};

    fn args(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(m) => m,
            _ => Map::new(),
        }
    }

    fn settings(v: Value, out: &str) -> Result<ExportSettings, Fail> {
        let m = args(v);
        settings_from_args(&Args(&m), Path::new(out))
    }

    #[test]
    fn container_follows_the_extension_and_fields_override() {
        let s = settings(json!({}), "a.mov").unwrap();
        assert_eq!(s.container, Container::Mov);
        let s =
            settings(json!({"videoCodec": "prores", "profile": "4444", "alpha": true, "resolution": "match"}), "a.mov")
                .unwrap();
        let v = s.video.as_ref().unwrap();
        assert_eq!(
            (v.codec, v.profile.as_deref(), v.alpha, s.size),
            (VideoCodec::Prores, Some("4444"), true, Size::MatchTimeline)
        );
        s.validate(None).unwrap();
        let s = settings(
            json!({"format": "webm", "quality": "very high", "resolution": "720p", "frameRate": 25}),
            "x.webm",
        )
        .unwrap();
        assert_eq!((s.size, s.fps), (Size::Height(720), Fps::Rate(25)));
        assert_eq!(s.video.unwrap().quality, Quality::VeryHigh);
    }

    #[test]
    fn presets_settings_and_refusals() {
        let s = settings(json!({"preset": "ProRes 422 HQ master", "range": [1, 3]}), "m.mov").unwrap();
        assert_eq!(s.range, Some([1.0, 3.0]));
        let e = settings(json!({"preset": "ProRes 422 HQ master"}), "m.mp4").unwrap_err();
        assert_eq!(e.code, "export.invalid");
        assert!(e.message.contains(".mov"), "{}", e.message);
        let e = settings(json!({"preset": "no such"}), "m.mov").unwrap_err();
        assert_eq!(e.code, "export.preset.unknown");
        let s = settings(json!({"settings": {"container": "wav"}}), "a.wav").unwrap();
        assert!(s.video.is_none() && s.audio.is_some());
        let e = settings(json!({"settings": {"container": "wav"}, "quality": "high"}), "a.wav").unwrap_err();
        assert!(e.message.contains("no video"), "{}", e.message);
        let e = settings(json!({"range": [3]}), "a.mp4").unwrap_err();
        assert!(e.message.contains("[start, end]"));
        let s = settings(json!({"audioCodec": "none"}), "a.mp4").unwrap();
        assert!(s.audio.is_none());
    }
}
