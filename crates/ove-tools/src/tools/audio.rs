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
