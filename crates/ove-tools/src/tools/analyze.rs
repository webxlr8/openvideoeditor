//! Media and timeline analysis: `grab_frame`, `filmstrip`, `detect_scenes`, `audio_levels`, `waveform_image`,
//! `detect_silence`, `measure_loudness`, `look`, `screenshot`. Needs the `media` feature (FFmpeg). Agents receive
//! the pictures as images; the command line writes them to the paths it is given.

use crate::ctx::ToolCtx;
use crate::fail::Fail;
use crate::receipt::Receipt;
use crate::spec::Args;

/// Parses `2,5.5,10` into seconds.
pub fn parse_times(s: &str) -> Result<Vec<f64>, Fail> {
    s.split(',')
        .filter(|x| !x.trim().is_empty())
        .map(|x| x.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| Fail::new("argument.invalid", format!("times must be seconds separated by commas, not {s}")))
}

#[cfg(not(feature = "media"))]
fn unavailable() -> Fail {
    Fail::new("media.unavailable", "this build of ove cannot read media; build it with `--features media`")
}

#[cfg(not(feature = "media"))]
mod imp {
    use super::*;
    pub fn grab_frame(_: &mut dyn ToolCtx, _: &Args) -> Result<Receipt, Fail> {
        Err(unavailable())
    }
    pub use grab_frame as filmstrip;
    pub use grab_frame as detect_scenes;
    pub use grab_frame as audio_levels;
    pub use grab_frame as waveform_image;
    pub use grab_frame as detect_silence;
    pub use grab_frame as measure_loudness;
    pub use grab_frame as look;
    pub use grab_frame as screenshot;
}

#[cfg(feature = "media")]
mod imp {
    use std::path::{Path, PathBuf};

    use serde_json::{Value, json};

    use super::*;
    use crate::ctx::snapshot;
    use crate::receipt::Png;
    use crate::tools::media::media_or_file;

    fn fail(e: ove_analyze::Error) -> Fail {
        Fail::new(e.code(), e.to_string())
    }

    fn png(img: &ove_analyze::Image) -> Result<Png, Fail> {
        let bytes = img.png().map_err(|e| Fail::new("image.encode", e.to_string()))?;
        Ok(Png { bytes, width: img.width, height: img.height })
    }

    fn write_png(img: &ove_analyze::Image, out: &Path) -> Result<(), Fail> {
        if let Some(d) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(d).map_err(|e| Fail::new("file.unwritable", format!("{}: {e}", d.display())))?;
        }
        img.png()
            .and_then(|png| std::fs::write(out, png))
            .map_err(|e| Fail::new("file.unwritable", format!("{}: {e}", out.display())))
    }

    /// Writes the picture to `out` (command line) and/or returns it as an image (agents).
    fn deliver(ctx: &dyn ToolCtx, img: &ove_analyze::Image, out: Option<&Path>) -> Result<Vec<Png>, Fail> {
        if let Some(o) = out {
            write_png(img, o)?;
        }
        if ctx.surface().is_agent() { Ok(vec![png(img)?]) } else { Ok(vec![]) }
    }

    fn fmt_db(v: Option<f64>) -> String {
        v.map(|x| format!("{x:.1}")).unwrap_or_else(|| "-inf".into())
    }

    fn out(a: &Args) -> Option<PathBuf> {
        a.str("out").map(PathBuf::from)
    }

    /// The written path for the text: `wrote <path> (…)`, or `frame (…)` when only returned.
    fn wrote(out: Option<&Path>) -> String {
        match out {
            Some(o) => format!("wrote {}", o.display()),
            None => "image".into(),
        }
    }

    pub fn grab_frame(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
        let file = media_or_file(ctx, a)?;
        let at = a.f64("at").unwrap_or(0.0);
        let width = a.i64("width").map(|w| w.clamp(2, 7680) as u32);
        let out = out(a);
        let img = ove_analyze::grab(&file, at, width).map_err(fail)?;
        let images = deliver(ctx, &img, out.as_deref())?;
        let mut v =
            json!({"width": img.width, "height": img.height, "at": at, "stats": ove_analyze::frame_stats(&img)});
        if let Some(o) = &out {
            v.as_object_mut().map(|m| m.insert("path".into(), json!(o)));
        }
        let text = format!("{} ({}x{}, frame at {at:.3} s)", wrote(out.as_deref()), img.width, img.height);
        Ok(Receipt::new(sort_path_first(v), text).with_images(images))
    }

    /// Puts `path` first, as the command line always printed it.
    fn sort_path_first(v: Value) -> Value {
        match v {
            Value::Object(m) if m.contains_key("path") => {
                let mut out = serde_json::Map::new();
                if let Some(p) = m.get("path") {
                    out.insert("path".into(), p.clone());
                }
                for (k, x) in m {
                    if k != "path" {
                        out.insert(k, x);
                    }
                }
                Value::Object(out)
            }
            other => other,
        }
    }

    pub fn filmstrip(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
        let opts = ove_analyze::FilmstripOpts {
            count: a.u32_or("count", 8).clamp(1, 200),
            every: a.f64("every"),
            from: a.f64("from").unwrap_or(0.0),
            to: a.f64("to"),
            columns: a.u32_or("columns", 0),
            tile_width: a.u32_or("tileWidth", 240).clamp(16, 1920),
        };
        let file = media_or_file(ctx, a)?;
        let out = out(a);
        let (img, times) = ove_analyze::filmstrip(&file, &opts).map_err(fail)?;
        let images = deliver(ctx, &img, out.as_deref())?;
        let list = times.iter().map(|t| format!("{t:.2}")).collect::<Vec<_>>().join(", ");
        let mut v = json!({"width": img.width, "height": img.height, "times": times});
        if let Some(o) = &out {
            v.as_object_mut().map(|m| m.insert("path".into(), json!(o)));
        }
        Ok(Receipt::new(
            sort_path_first(v),
            format!("{} ({}x{}) — frames at {list} s", wrote(out.as_deref()), img.width, img.height),
        )
        .with_images(images))
    }

    pub fn detect_scenes(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
        let file = media_or_file(ctx, a)?;
        let s = ove_analyze::scenes(&file, a.f64("threshold").unwrap_or(0.3)).map_err(fail)?;
        let mut text = format!("{} cuts in {:.2} s ({} frames)", s.cuts.len(), s.duration, s.frames);
        for c in &s.cuts {
            text.push_str(&format!("\n{:.3}\t{:.2}", c.time, c.score));
        }
        Ok(Receipt::new(serde_json::to_value(&s).unwrap_or(Value::Null), text))
    }

    pub fn audio_levels(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
        let file = media_or_file(ctx, a)?;
        let points = a.i64("points").unwrap_or(100).clamp(1, 10_000) as usize;
        let l = ove_analyze::levels(&file, points).map_err(fail)?;
        let peak = l.peaks_db.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        Ok(Receipt::new(
            serde_json::to_value(&l).unwrap_or(Value::Null),
            format!("{} buckets of {:.2} s; loudest peak {peak:.1} dBFS", l.peaks_db.len(), l.bucket_seconds),
        ))
    }

    pub fn waveform_image(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
        let file = media_or_file(ctx, a)?;
        let out = out(a);
        let img = ove_analyze::waveform_png(
            &file,
            a.u32_or("width", 1200).clamp(16, 8000),
            a.u32_or("height", 200).clamp(16, 2000),
        )
        .map_err(fail)?;
        let images = deliver(ctx, &img, out.as_deref())?;
        let mut v = json!({"width": img.width, "height": img.height});
        if let Some(o) = &out {
            v.as_object_mut().map(|m| m.insert("path".into(), json!(o)));
        }
        Ok(Receipt::new(sort_path_first(v), wrote(out.as_deref())).with_images(images))
    }

    pub fn detect_silence(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
        let file = media_or_file(ctx, a)?;
        let threshold = a.f64("thresholdDb").unwrap_or(-40.0);
        let min = a.f64("minDuration").unwrap_or(0.5);
        let r = ove_analyze::silence(&file, threshold, min).map_err(fail)?;
        let total: f64 = r.iter().map(|x| x.end - x.start).sum();
        let mut text = format!("{} silent ranges ({total:.2} s) below {threshold} dB", r.len());
        for x in &r {
            text.push_str(&format!("\n{:.3}\t{:.3}", x.start, x.end));
        }
        Ok(Receipt::new(json!({"ranges": r, "totalSeconds": total, "thresholdDb": threshold, "minSeconds": min}), text))
    }

    pub fn measure_loudness(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
        let file = media_or_file(ctx, a)?;
        let l = ove_analyze::loudness(&file).map_err(fail)?;
        let text = format!(
            "integrated {} LUFS · range {:.1} LU · sample peak {:.1} dBFS · true peak {:.1} dBTP",
            fmt_db(l.integrated_lufs),
            l.loudness_range_lu,
            l.sample_peak_dbfs,
            l.true_peak_dbtp
        );
        Ok(Receipt::new(serde_json::to_value(&l).unwrap_or(Value::Null), text))
    }

    pub fn look(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
        let bundle = ctx.bundle(a.project())?;
        let (doc, bundle, rev) = snapshot(ctx, &bundle)?;
        let times = match a.str("at") {
            Some(s) => parse_times(s)?,
            None => ove_analyze::timeline_times(&doc, a.u32_or("count", 6).max(1), a.f64("every")),
        };
        if times.is_empty() {
            return Err(Fail::new("argument.invalid", "no times to render"));
        }
        if times.len() > 200 {
            return Err(Fail::new("argument.invalid", "at most 200 frames per look"));
        }
        let (out, sheet, stats) = (a.str("out").map(PathBuf::from), a.str("sheet").map(PathBuf::from), a.bool("stats"));
        let agent = ctx.surface().is_agent();
        if !agent && out.is_none() && sheet.is_none() && !stats {
            return Err(Fail::new("argument.missing", "give --out <dir>, --sheet <file> or --stats"));
        }
        let images =
            ove_analyze::look(&doc, Some(&bundle), &times, a.u32_or("width", 640).clamp(16, 3840)).map_err(fail)?;
        let mut frames = Vec::new();
        let mut pngs = Vec::new();
        for (t, img) in times.iter().zip(&images) {
            let mut f = json!({"at": t, "width": img.width, "height": img.height});
            if let Some(dir) = &out {
                let path = dir.join(format!("frame-{:08.3}.png", t));
                write_png(img, &path)?;
                f["path"] = json!(path);
            }
            if stats {
                f["stats"] = serde_json::to_value(ove_analyze::frame_stats(img)).unwrap_or(Value::Null);
            }
            if agent {
                pngs.push(png(img)?);
            }
            frames.push(f);
        }
        let mut text = format!(
            "{} frames at {} s",
            frames.len(),
            times.iter().map(|t| format!("{t:.2}")).collect::<Vec<_>>().join(", ")
        );
        if let Some(dir) = &out {
            text.push_str(&format!(" → {}", dir.display()));
        }
        let mut v = json!({"rev": rev, "frames": frames});
        if let Some(sheet) = &sheet {
            let cols = match a.u32_or("columns", 0) {
                0 => (images.len() as f64).sqrt().ceil() as u32,
                c => c,
            };
            let img = ove_analyze::sheet(&images, cols.max(1));
            write_png(&img, sheet)?;
            v["sheet"] = json!({"path": sheet, "width": img.width, "height": img.height});
            text.push_str(&format!("; sheet {}", sheet.display()));
        }
        if stats {
            for f in &frames {
                let s = &f["stats"];
                text.push_str(&format!(
                    "\n{:.2}s\tluma {:.2}\tblack {:.0}%\tclipped {:.0}%",
                    f["at"].as_f64().unwrap_or(0.0),
                    s["meanLuma"].as_f64().unwrap_or(0.0),
                    s["blackFraction"].as_f64().unwrap_or(0.0) * 100.0,
                    s["clippedFraction"].as_f64().unwrap_or(0.0) * 100.0
                ));
            }
        }
        Ok(Receipt::new(v, text).with_images(pngs))
    }

    /// The frame under the app's playhead.
    pub fn screenshot(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
        let c = ctx.app("context", json!({}))?;
        let bundle = c
            .pointer("/project/bundle")
            .and_then(Value::as_str)
            .ok_or_else(|| Fail::new("app.noProject", "the app has no project open"))?
            .to_string();
        let at = c.pointer("/playback/seconds").and_then(Value::as_f64).unwrap_or(0.0);
        let (doc, b, rev) = snapshot(ctx, Path::new(&bundle))?;
        let mut imgs =
            ove_analyze::look(&doc, Some(&b), &[at], a.u32_or("width", 1280).clamp(16, 3840)).map_err(fail)?;
        let img = imgs.pop().ok_or_else(|| Fail::new("render.failed", "no frame was rendered"))?;
        let out = out(a);
        let images = deliver(ctx, &img, out.as_deref())?;
        let mut v = json!({"at": at, "width": img.width, "height": img.height, "project": bundle, "rev": rev});
        if let Some(o) = &out {
            v.as_object_mut().map(|m| m.insert("path".into(), json!(o)));
        }
        Ok(Receipt::new(sort_path_first(v), format!("{} (frame at {at:.3} s)", wrote(out.as_deref())))
            .with_images(images))
    }
}

pub use imp::*;
