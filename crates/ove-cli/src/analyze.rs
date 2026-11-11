//! Media and timeline analysis: `media grab|filmstrip|scenes`, `audio …`, `look`, `screenshot`. Needs the `media`
//! feature (FFmpeg).

use std::path::{Path, PathBuf};

use clap::Subcommand;
#[cfg(feature = "media")]
use serde_json::Value;
#[cfg(feature = "media")]
use serde_json::json;

use crate::common::{Ctx, Fail, Out};
#[cfg(feature = "media")]
use crate::media::media_or_file;

#[derive(Subcommand)]
pub enum AudioCmd {
    /// Peak and RMS level (dBFS) per bucket across the file.
    Levels {
        project: String,
        media: String,
        #[arg(long, default_value_t = 100)]
        points: usize,
    },
    /// The waveform as a PNG.
    Waveform {
        project: String,
        media: String,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 1200)]
        width: u32,
        #[arg(long, default_value_t = 200)]
        height: u32,
    },
    /// Ranges quieter than --threshold for at least --min seconds.
    Silence {
        project: String,
        media: String,
        #[arg(long, default_value_t = -40.0, allow_negative_numbers = true)]
        threshold: f64,
        #[arg(long, default_value_t = 0.5)]
        min: f64,
    },
    /// Integrated loudness (LUFS), loudness range, sample and true peak.
    Loudness { project: String, media: String },
}

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

#[cfg(feature = "media")]
fn fail(e: ove_analyze::Error) -> Fail {
    Fail::new(e.code(), e.to_string())
}

#[cfg(feature = "media")]
fn write_png(img: &ove_analyze::Image, out: &Path) -> Result<(), Fail> {
    if let Some(d) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d).map_err(|e| Fail::new("file.unwritable", format!("{}: {e}", d.display())))?;
    }
    std::fs::write(out, img.png()).map_err(|e| Fail::new("file.unwritable", format!("{}: {e}", out.display())))
}

#[cfg(feature = "media")]
fn fmt_db(v: Option<f64>) -> String {
    v.map(|x| format!("{x:.1}")).unwrap_or_else(|| "-inf".into())
}

#[cfg(feature = "media")]
pub fn grab(ctx: &Ctx, project: &str, media: &str, at: f64, width: Option<u32>, out: &Path) -> Out {
    let file = media_or_file(ctx, project, media)?;
    let img = ove_analyze::grab(&file, at, width).map_err(fail)?;
    write_png(&img, out)?;
    Ok((
        json!({"path": out, "width": img.width, "height": img.height, "at": at, "stats": ove_analyze::frame_stats(&img)}),
        format!("wrote {} ({}x{}, frame at {at:.3} s)", out.display(), img.width, img.height),
    ))
}

/// `media filmstrip` options.
pub struct StripOpts {
    pub count: u32,
    pub every: Option<f64>,
    pub from: f64,
    pub to: Option<f64>,
    pub columns: u32,
    pub tile_width: u32,
}

#[cfg(feature = "media")]
pub fn filmstrip(ctx: &Ctx, project: &str, media: &str, o: &StripOpts, out: &Path) -> Out {
    let opts = ove_analyze::FilmstripOpts {
        count: o.count.clamp(1, 200),
        every: o.every,
        from: o.from,
        to: o.to,
        columns: o.columns,
        tile_width: o.tile_width.clamp(16, 1920),
    };
    let file = media_or_file(ctx, project, media)?;
    let (img, times) = ove_analyze::filmstrip(&file, &opts).map_err(fail)?;
    write_png(&img, out)?;
    let list = times.iter().map(|t| format!("{t:.2}")).collect::<Vec<_>>().join(", ");
    Ok((
        json!({"path": out, "width": img.width, "height": img.height, "times": times}),
        format!("wrote {} ({}x{}) — frames at {list} s", out.display(), img.width, img.height),
    ))
}

#[cfg(feature = "media")]
pub fn scenes(ctx: &Ctx, project: &str, media: &str, threshold: f64) -> Out {
    let file = media_or_file(ctx, project, media)?;
    let s = ove_analyze::scenes(&file, threshold).map_err(fail)?;
    let mut text = format!("{} cuts in {:.2} s ({} frames)", s.cuts.len(), s.duration, s.frames);
    for c in &s.cuts {
        text.push_str(&format!("\n{:.3}\t{:.2}", c.time, c.score));
    }
    Ok((serde_json::to_value(&s).unwrap_or(Value::Null), text))
}

#[cfg(feature = "media")]
pub fn audio(ctx: &Ctx, cmd: &AudioCmd) -> Out {
    match cmd {
        AudioCmd::Levels { project, media, points } => {
            let file = media_or_file(ctx, project, media)?;
            let l = ove_analyze::levels(&file, (*points).clamp(1, 10_000)).map_err(fail)?;
            let peak = l.peaks_db.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            Ok((
                serde_json::to_value(&l).unwrap_or(Value::Null),
                format!("{} buckets of {:.2} s; loudest peak {peak:.1} dBFS", l.peaks_db.len(), l.bucket_seconds),
            ))
        }
        AudioCmd::Waveform { project, media, out, width, height } => {
            let file = media_or_file(ctx, project, media)?;
            let img =
                ove_analyze::waveform_png(&file, (*width).clamp(16, 8000), (*height).clamp(16, 2000)).map_err(fail)?;
            write_png(&img, out)?;
            Ok((json!({"path": out, "width": img.width, "height": img.height}), format!("wrote {}", out.display())))
        }
        AudioCmd::Silence { project, media, threshold, min } => {
            let file = media_or_file(ctx, project, media)?;
            let r = ove_analyze::silence(&file, *threshold, *min).map_err(fail)?;
            let total: f64 = r.iter().map(|x| x.end - x.start).sum();
            let mut text = format!("{} silent ranges ({total:.2} s) below {threshold} dB", r.len());
            for x in &r {
                text.push_str(&format!("\n{:.3}\t{:.3}", x.start, x.end));
            }
            Ok((json!({"ranges": r, "totalSeconds": total, "thresholdDb": threshold, "minSeconds": min}), text))
        }
        AudioCmd::Loudness { project, media } => {
            let file = media_or_file(ctx, project, media)?;
            let l = ove_analyze::loudness(&file).map_err(fail)?;
            let text = format!(
                "integrated {} LUFS · range {:.1} LU · sample peak {:.1} dBFS · true peak {:.1} dBTP",
                fmt_db(l.integrated_lufs),
                l.loudness_range_lu,
                l.sample_peak_dbfs,
                l.true_peak_dbtp
            );
            Ok((serde_json::to_value(&l).unwrap_or(Value::Null), text))
        }
    }
}

/// What `look` writes.
pub struct LookOpts<'a> {
    pub at: Option<&'a str>,
    pub every: Option<f64>,
    pub count: u32,
    pub width: u32,
    pub out: Option<&'a Path>,
    pub sheet: Option<&'a Path>,
    pub columns: u32,
    pub stats: bool,
}

#[cfg(feature = "media")]
pub fn look(ctx: &Ctx, project: &str, o: &LookOpts) -> Out {
    let p = ctx.read(project)?;
    let times = match o.at {
        Some(s) => parse_times(s)?,
        None => ove_analyze::timeline_times(p.doc(), o.count.max(1), o.every),
    };
    if times.is_empty() {
        return Err(Fail::new("argument.invalid", "no times to render"));
    }
    if times.len() > 200 {
        return Err(Fail::new("argument.invalid", "at most 200 frames per look"));
    }
    if o.out.is_none() && o.sheet.is_none() && !o.stats {
        return Err(Fail::new("argument.missing", "give --out <dir>, --sheet <file> or --stats"));
    }
    let images = ove_analyze::look(p.doc(), Some(p.bundle()), &times, o.width.clamp(16, 3840)).map_err(fail)?;
    let mut frames = Vec::new();
    for (t, img) in times.iter().zip(&images) {
        let mut f = json!({"at": t, "width": img.width, "height": img.height});
        if let Some(dir) = o.out {
            let path = dir.join(format!("frame-{:08.3}.png", t));
            write_png(img, &path)?;
            f["path"] = json!(path);
        }
        if o.stats {
            f["stats"] = serde_json::to_value(ove_analyze::frame_stats(img)).unwrap_or(Value::Null);
        }
        frames.push(f);
    }
    let mut text = format!(
        "{} frames at {} s",
        frames.len(),
        times.iter().map(|t| format!("{t:.2}")).collect::<Vec<_>>().join(", ")
    );
    if let Some(dir) = o.out {
        text.push_str(&format!(" → {}", dir.display()));
    }
    let mut v = json!({"rev": p.rev(), "frames": frames});
    if let Some(sheet) = o.sheet {
        let cols = if o.columns == 0 { (images.len() as f64).sqrt().ceil() as u32 } else { o.columns };
        let img = ove_analyze::sheet(&images, cols.max(1));
        write_png(&img, sheet)?;
        v["sheet"] = json!({"path": sheet, "width": img.width, "height": img.height});
        text.push_str(&format!("; sheet {}", sheet.display()));
    }
    if o.stats {
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
    Ok((v, text))
}

/// The frame under the app's playhead.
#[cfg(feature = "media")]
pub fn screenshot(ctx: &Ctx, out: &Path, width: u32) -> Out {
    let c = ctx.app("context", json!({}))?;
    let bundle = c
        .pointer("/project/bundle")
        .and_then(Value::as_str)
        .ok_or_else(|| Fail::new("app.noProject", "the app has no project open"))?
        .to_string();
    let at = c.pointer("/playback/seconds").and_then(Value::as_f64).unwrap_or(0.0);
    let p = ctx.read(&bundle)?;
    let img = ove_analyze::look(p.doc(), Some(p.bundle()), &[at], width.clamp(16, 3840)).map_err(fail)?.remove(0);
    write_png(&img, out)?;
    Ok((
        json!({"path": out, "at": at, "width": img.width, "height": img.height, "project": bundle, "rev": p.rev()}),
        format!("wrote {} (frame at {at:.3} s)", out.display()),
    ))
}

#[cfg(not(feature = "media"))]
pub fn grab(_: &Ctx, _: &str, _: &str, _: f64, _: Option<u32>, _: &Path) -> Out {
    Err(unavailable())
}
#[cfg(not(feature = "media"))]
pub fn filmstrip(_: &Ctx, _: &str, _: &str, _: &StripOpts, _: &Path) -> Out {
    Err(unavailable())
}
#[cfg(not(feature = "media"))]
pub fn scenes(_: &Ctx, _: &str, _: &str, _: f64) -> Out {
    Err(unavailable())
}
#[cfg(not(feature = "media"))]
pub fn audio(_: &Ctx, _: &AudioCmd) -> Out {
    Err(unavailable())
}
#[cfg(not(feature = "media"))]
pub fn look(_: &Ctx, _: &str, _: &LookOpts) -> Out {
    Err(unavailable())
}
#[cfg(not(feature = "media"))]
pub fn screenshot(_: &Ctx, _: &Path, _: u32) -> Out {
    Err(unavailable())
}
