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

    /// A copy of the frame with the coordinate grid of a canvas of `reference` units, when given.
    fn with_grid(img: &ove_analyze::Image, reference: Option<(u32, u32)>) -> ove_analyze::Image {
        let mut out = img.clone();
        if let Some(r) = reference {
            ove_analyze::grid::overlay(&mut out, r);
        }
        out
    }

    /// What the grid means, for the receipt.
    fn grid_json(reference: (u32, u32)) -> Value {
        json!({
            "units": "transform units: reference pixels from the canvas centre, x right, y down",
            "reference": [reference.0, reference.1],
            "step": ove_analyze::grid::step(reference),
            "titleSafe": 0.8,
            "actionSafe": 0.9,
        })
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
        let raw = ove_analyze::grab(&file, at, width).map_err(fail)?;
        // A media frame's grid is in its own pixels from the centre.
        let img = with_grid(&raw, a.bool("grid").then_some((raw.width, raw.height)));
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
        if a.bool("storyboard") {
            let (img, tiles) = ove_analyze::storyboard::media(&file, opts.tile_width).map_err(fail)?;
            return storyboard_receipt(ctx, img, &tiles, out.as_deref(), None);
        }
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

    /// The loudness of project media from the import analysis cache (`cache/waveforms/<contentId>.json`), analysing
    /// it once when the cache has none; `None` for a plain file path or media without a content id.
    fn cached_loudness(ctx: &mut dyn ToolCtx, a: &Args, file: &Path) -> Option<ove_analyze::Loudness> {
        let q = a.req_str("media").ok()?.to_string();
        let bundle = ctx.bundle(a.project()).ok()?;
        let (b, cid) = crate::ctx::read(ctx, &bundle, |p| {
            let id = crate::tools::media::find_media(p.doc(), p.bundle(), &q)?;
            let cid = p.doc().registry.get(&id).and_then(|e| e.fields.get("contentId")?.as_str().map(str::to_string));
            Ok((p.bundle().to_path_buf(), cid))
        })
        .ok()?;
        ove_analyze::analysis::ensure(&b, &cid?, file).ok().map(|s| s.loudness)
    }

    /// `measure_loudness` with `timeline`: the mix of the whole timeline as it would export (gain off, true-peak
    /// limiter on).
    fn timeline_loudness(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
        let bundle = ctx.bundle(a.project())?;
        let (doc, bundle, _) = crate::ctx::snapshot(ctx, &bundle)?;
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let l = ove_render::export::measure_mix(&doc, Some(&bundle), 0, doc.duration(), &cancel)
            .map_err(|e| Fail::new("loudness.failed", e.to_string()))?;
        let text = match l.integrated_lufs {
            Some(i) => format!(
                "timeline: integrated {i:.1} LUFS · range {:.1} LU · true peak {:.1} dBTP · {:.1} s",
                l.loudness_range_lu, l.true_peak_dbtp, l.duration
            ),
            None => format!("timeline: silent (below the -70 LUFS gate) · {:.1} s", l.duration),
        };
        let mut v = serde_json::to_value(l).unwrap_or(Value::Null);
        v["timeline"] = json!(true);
        Ok(Receipt::new(v, text))
    }

    pub fn measure_loudness(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
        if a.bool("timeline") {
            return timeline_loudness(ctx, a);
        }
        let file = media_or_file(ctx, a)?;
        let l = match cached_loudness(ctx, a, &file) {
            Some(l) => l,
            None => ove_analyze::loudness(&file).map_err(fail)?,
        };
        let text = format!(
            "integrated {} LUFS · range {:.1} LU · sample peak {:.1} dBFS · true peak {:.1} dBTP",
            fmt_db(l.integrated_lufs),
            l.loudness_range_lu,
            l.sample_peak_dbfs,
            l.true_peak_dbtp
        );
        Ok(Receipt::new(serde_json::to_value(&l).unwrap_or(Value::Null), text))
    }

    /// A storyboard sheet: the image, and each tile's time and visible items.
    fn storyboard_receipt(
        ctx: &dyn ToolCtx,
        img: ove_analyze::Image,
        tiles: &[ove_analyze::storyboard::Tile],
        out: Option<&Path>,
        rev: Option<i64>,
    ) -> Result<Receipt, Fail> {
        let images = deliver(ctx, &img, out)?;
        let list: Vec<Value> = tiles
            .iter()
            .map(|t| json!({"at": (t.at * 1000.0).round() / 1000.0, "timecode": ove_analyze::storyboard::timecode(t.at), "items": t.items}))
            .collect();
        let mut v = json!({"storyboard": true, "width": img.width, "height": img.height, "columns": ove_analyze::storyboard::COLUMNS, "tiles": list});
        if let Some(r) = rev {
            v["rev"] = json!(r);
        }
        if let Some(o) = out {
            v["path"] = json!(o);
        }
        let text = format!("{} ({}x{}): storyboard of {} tiles", wrote(out), img.width, img.height, tiles.len());
        Ok(Receipt::new(v, text).with_images(images))
    }

    pub fn look(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
        let bundle = ctx.bundle(a.project())?;
        let (doc, bundle, rev) = snapshot(ctx, &bundle)?;
        if a.bool("storyboard") {
            let width = a.u32_or("width", 240).clamp(16, 1920);
            let (img, tiles) = ove_analyze::storyboard::timeline(&doc, Some(&bundle), width).map_err(fail)?;
            let sheet = a.str("sheet").map(PathBuf::from);
            return storyboard_receipt(ctx, img, &tiles, sheet.as_deref(), Some(rev));
        }
        let doc = match a.str("view") {
            None | Some("result") => doc,
            Some("matte") => ove_render::effects::key::matte_view(&doc),
            Some(v) => return Err(Fail::new("argument.invalid", format!("view is result or matte, not {v}"))),
        };
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
        let scope = match a.str("scope") {
            None => None,
            Some(n) => Some(ove_render::scopes::Scope::parse(n).ok_or_else(|| {
                Fail::new(
                    "argument.invalid",
                    format!("there is no scope {n}; scopes: {}", ove_render::scopes::NAMES.join(", ")),
                )
            })?),
        };
        if scope.is_some() && !agent && out.is_none() {
            return Err(Fail::new(
                "argument.missing",
                "give --out <file.png> (one time) or --out <dir> for the scope images",
            ));
        }
        if !agent && out.is_none() && sheet.is_none() && !stats && scope.is_none() {
            return Err(Fail::new("argument.missing", "give --out <dir>, --sheet <file> or --stats"));
        }
        let chosen = crate::tools::backend::choose(a)?;
        let backend = crate::tools::backend::json(&chosen);
        let images = ove_analyze::look_with(
            &doc,
            Some(&bundle),
            &times,
            a.u32_or("width", 640).clamp(16, 3840),
            Some(chosen.backend),
        )
        .map_err(fail)?;
        let reference = ove_render::eval::reference_size(&doc.project.aspect_ratio);
        let grid = a.bool("grid");
        // What is shown (with the grid when asked); statistics are of the frame itself.
        let shown: Vec<ove_analyze::Image> = images.iter().map(|i| with_grid(i, grid.then_some(reference))).collect();
        let mut frames = Vec::new();
        let mut pngs = Vec::new();
        if let Some(scope) = scope {
            return scope_receipt(agent, scope, &times, &images, out.as_deref(), rev, backend);
        }
        for ((t, img), seen) in times.iter().zip(&images).zip(&shown) {
            let mut f = json!({"at": t, "width": img.width, "height": img.height});
            if let Some(dir) = &out {
                let path = dir.join(format!("frame-{:08.3}.png", t));
                write_png(seen, &path)?;
                f["path"] = json!(path);
            }
            if stats {
                f["stats"] = serde_json::to_value(ove_analyze::frame_stats(img)).unwrap_or(Value::Null);
            }
            if agent {
                pngs.push(png(seen)?);
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
        let mut v = json!({"rev": rev, "frames": frames, "backend": backend});
        if grid {
            v["grid"] = grid_json(reference);
        }
        if let Some(sheet) = &sheet {
            let cols = match a.u32_or("columns", 0) {
                0 => (images.len() as f64).sqrt().ceil() as u32,
                c => c,
            };
            let img = ove_analyze::sheet(&shown, cols.max(1));
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

    /// `look` with `scope`: one scope image per time (the same binning as the colour panel), written to `out` (a
    /// .png file for one time, else a folder) and returned to agents.
    fn scope_receipt(
        agent: bool,
        scope: ove_render::scopes::Scope,
        times: &[f64],
        images: &[ove_analyze::Image],
        out: Option<&Path>,
        rev: i64,
        backend: Value,
    ) -> Result<Receipt, Fail> {
        let name = ove_render::scopes::NAMES[scope as usize];
        let one_file =
            times.len() == 1 && out.is_some_and(|o| o.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")));
        let (mut frames, mut pngs) = (Vec::new(), Vec::new());
        for (t, img) in times.iter().zip(images) {
            let frame = ove_render::Rgba { w: img.width, h: img.height, data: img.rgba.clone() };
            let d = ove_render::scopes::density(&ove_render::scopes::bin(&frame, scope));
            let s = ove_render::scopes::image(&d, scope, 2);
            let shown = ove_analyze::Image { width: s.w, height: s.h, rgba: s.data };
            let mut f = json!({"at": t, "width": shown.width, "height": shown.height});
            if let Some(o) = out {
                let path = if one_file { o.to_path_buf() } else { o.join(format!("{name}-{t:08.3}.png")) };
                write_png(&shown, &path)?;
                f["path"] = json!(path);
            }
            if agent {
                pngs.push(png(&shown)?);
            }
            frames.push(f);
        }
        let text = format!(
            "{name} at {} s{}",
            times.iter().map(|t| format!("{t:.2}")).collect::<Vec<_>>().join(", "),
            out.map(|o| format!(" → {}", o.display())).unwrap_or_default()
        );
        Ok(Receipt::new(json!({"rev": rev, "scope": name, "frames": frames, "backend": backend}), text)
            .with_images(pngs))
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
        let chosen = crate::tools::backend::choose(a)?;
        let mut imgs = ove_analyze::look_with(
            &doc,
            Some(&b),
            &[at],
            a.u32_or("width", 1280).clamp(16, 3840),
            Some(chosen.backend),
        )
        .map_err(fail)?;
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
