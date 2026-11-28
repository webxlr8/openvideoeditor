//! `duck_audio`: lower music under dialogue by writing volume keyframes on the music items (design D7), as one
//! revision, or return the planned keyframes with `dryRun`. Needs the `media` feature (the level gate mixes the
//! dialogue).

use crate::ctx::ToolCtx;
use crate::fail::Fail;
use crate::receipt::Receipt;
use crate::spec::Args;

/// Item or track names from a JSON list or a comma-separated string.
pub fn names(a: &Args, k: &str) -> Result<Vec<String>, Fail> {
    let v = a.value(k).ok_or_else(|| Fail::new("argument.missing", format!("duck_audio needs `{k}`")))?;
    let list: Vec<String> = match v {
        serde_json::Value::String(s) => s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
        serde_json::Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        _ => vec![],
    };
    if list.is_empty() {
        return Err(Fail::new("argument.invalid", format!("`{k}` names no items or tracks")));
    }
    Ok(list)
}

#[cfg(not(feature = "media"))]
pub fn duck_audio(_: &mut dyn ToolCtx, _: &Args) -> Result<Receipt, Fail> {
    Err(Fail::new("media.unavailable", "this build of ove cannot read media; build it with `--features media`"))
}

#[cfg(feature = "media")]
pub use imp::duck_audio;

#[cfg(not(feature = "media"))]
pub fn extract_audio(_: &mut dyn ToolCtx, _: &Args) -> Result<Receipt, Fail> {
    Err(Fail::new("media.unavailable", "this build of ove cannot read media; build it with `--features media`"))
}

#[cfg(feature = "media")]
pub use imp::extract_audio;

#[cfg(feature = "media")]
mod imp {
    use ove_analyze::duck::{self, DuckParams, Mode, SpeechSource};
    use serde_json::json;

    use super::*;
    use crate::ctx::{edit, via};
    use crate::fail::plural;

    fn secs(a: &Args, k: &str, default: f64) -> Result<i64, Fail> {
        let v = a.f64(k).unwrap_or(default);
        if !(0.0..=10.0).contains(&v) {
            return Err(Fail::new("argument.invalid", format!("`{k}` must be 0–10 seconds, got {v}")));
        }
        Ok((v * 1e6).round() as i64)
    }

    /// `extract_audio`: a media file's sound as a new WAV (24-bit) or FLAC file in the bundle's `media/` folder,
    /// added to the library in one revision.
    pub fn extract_audio(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
        use ove_render::export::settings::Container;
        let q = a.req_str("media")?.to_string();
        let container = match a.str("format").unwrap_or("wav") {
            "wav" => Container::Wav,
            "flac" => Container::Flac,
            f => return Err(Fail::new("argument.invalid", format!("`format` is wav or flac, not {f}"))),
        };
        let bundle = ctx.bundle(a.project())?;
        let (doc, bundle, _) = crate::ctx::snapshot(ctx, &bundle)?;
        let id = crate::tools::media::find_media(&doc, &bundle, &q)?;
        let src = crate::tools::media::media_file(&doc, &bundle, &id)?;
        let name = doc.project.media.get(&id).and_then(|m| m.file_name()).unwrap_or(&id).to_string();
        if !src.is_file() {
            return Err(Fail::new("media.offline", format!("“{name}” is offline; reconnect or relink it first")));
        }
        let info = ove_media::probe(&src).map_err(|e| Fail::new("media.unreadable", format!("{name}: {e}")))?;
        let Some(audio) = info.audio.as_ref() else {
            return Err(Fail::new("media.noAudio", format!("“{name}” has no audio, so there is nothing to extract")));
        };
        let dur = audio.duration_us.or(info.duration_us).filter(|d| *d > 0).unwrap_or(1_000_000);
        // The file's sound alone, as a one-item timeline, through the export writers (partial files, offline checks).
        let one = json!([
            {"op": "addTrack", "track": {"id": "t", "name": "Audio", "order": 0, "type": "audio"}},
            {"op": "addMedia", "media": {"id": "m", "data": {"type": "audio", "fileName": name, "duration": dur as f64 / 1e6, "hasAudio": true}},
                "entry": {"mediaId": "m", "nativePath": src.display().to_string(), "ownedByBundle": false}},
            {"op": "addItem", "item": {"id": "a", "trackId": "t", "type": "audio", "startTime": 0, "endTime": dur as f64 / 1e6, "mediaItemId": "m"}}
        ]);
        let ops: Vec<ove_core::Op> = serde_json::from_value(one).map_err(|e| Fail::new("internal", e.to_string()))?;
        let solo = ove_core::ops::apply_batch(&ove_core::model::Document::new("x", "extract", 1), &ops)
            .map_err(|e| Fail::new("internal", format!("{e:?}")))?
            .doc;
        let stem =
            std::path::Path::new(&name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(id.clone());
        let dir = bundle.join("media");
        let mut file = dir.join(format!("{stem} (audio).{}", container.extension()));
        let mut n = 2;
        while file.exists() {
            file = dir.join(format!("{stem} (audio {n}).{}", container.extension()));
            n += 1;
        }
        let settings = ove_render::ExportSettings::for_container(container);
        let cancel = std::sync::atomic::AtomicBool::new(false);
        ove_render::export(&solo, None, &file, &settings, &cancel, |_| {})
            .map_err(|e| Fail::new("extract.failed", format!("could not extract the sound of “{name}”: {e}")))?;
        let now = crate::fail::now_ms();
        let new_id = format!("m_{now:x}_audio");
        let (media, mut entry, _, _) = crate::tools::media::describe(&file, &new_id, now, 5_000_000)?;
        let rel = format!("media/{}", file.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default());
        entry["nativePath"] = json!(rel);
        entry["ownedByBundle"] = json!(true);
        let out = file.clone();
        let rev = edit(ctx, &bundle, |w| {
            let r = w.apply(
                vec![ove_core::Op::AddMedia { media: media.clone(), entry: Some(entry.clone()) }],
                format!("extract audio from {name}"),
            );
            if r.is_err() {
                let _ = std::fs::remove_file(&out);
            }
            r
        })?;
        let v = json!({"rev": rev, "media": new_id, "path": file.display().to_string(), "from": id,
            "format": container.name(), "duration": dur as f64 / 1e6});
        Ok(Receipt::new(
            v,
            format!("rev {rev}: extracted the sound of “{name}” to {} as media {new_id}", file.display()),
        ))
    }
    pub fn duck_audio(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
        let music = names(a, "music")?;
        let under = names(a, "under")?;
        let depth = a.f64("depth").unwrap_or(-12.0);
        if !(-60.0..=0.0).contains(&depth) {
            return Err(Fail::new("argument.invalid", format!("`depth` must be -60 to 0 dB, got {depth}")));
        }
        let p = DuckParams {
            depth_db: depth,
            attack_us: secs(a, "attack", 0.2)?,
            release_us: secs(a, "release", 0.4)?,
            hold_us: secs(a, "hold", 0.5)?,
        };
        let source = match a.str("source").unwrap_or("auto") {
            "auto" => SpeechSource::Auto,
            "transcript" => SpeechSource::Transcript,
            "level" => SpeechSource::Level,
            s => return Err(Fail::new("argument.invalid", format!("`source` is transcript, level or auto, not {s}"))),
        };
        let mode = match a.str("mode").unwrap_or("replace") {
            "replace" => Mode::Replace,
            "merge" => Mode::Merge,
            s => return Err(Fail::new("argument.invalid", format!("`mode` is replace or merge, not {s}"))),
        };
        let dry = a.bool("dryRun");
        let bundle = ctx.bundle(a.project())?;
        edit(ctx, &bundle, |w| {
            let plan = duck::plan(w.doc(), Some(w.bundle()), &music, &under, &p, source, mode)
                .map_err(|e| Fail::new(e.code(), e.to_string()))?;
            let keys: usize = plan.items.iter().map(|i| i.keyframes.len()).sum();
            let mut v = json!({
                "source": plan.source,
                "speech": plan.speech,
                "items": plan.items,
                "depth": depth,
                "dryRun": dry,
            });
            let what = format!(
                "{} under {} of speech ({} from the {}), {}",
                plural(plan.items.len(), "music item"),
                plural(plan.speech.len(), "span"),
                format_args!("{depth} dB"),
                plan.source,
                plural(keys, "keyframe")
            );
            if dry {
                v["rev"] = w.rev().into();
                return Ok(Receipt::new(v, format!("would duck {what}")));
            }
            let rev = w.apply(plan.ops.clone(), format!("duck music {depth} dB under dialogue"))?;
            v["rev"] = rev.into();
            let text = format!("rev {rev}: ducked {what}{}", via(w));
            Ok(Receipt::new(v, text))
        })
    }
}
