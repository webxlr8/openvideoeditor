//! Leaving the editor: `export_video`, `export_timeline_file`, `import_timeline_file`.

use std::path::PathBuf;

use serde_json::json;

use crate::ctx::{ToolCtx, edit, read, via};
use crate::fail::{Fail, now_ms};
use crate::receipt::Receipt;
use crate::spec::Args;

fn interchange_format(file: &std::path::Path, format: Option<&str>) -> Result<ove_interchange::Format, Fail> {
    match format {
        Some(f) => ove_interchange::Format::parse(f),
        None => ove_interchange::Format::from_path(file),
    }
    .map_err(|e| Fail::new("format.unknown", e.to_string()))
}

pub fn export_timeline_file(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let file = PathBuf::from(a.req_str("file")?);
    let f = interchange_format(&file, a.str("format"))?;
    let fps = a.f64("fps").unwrap_or(30.0);
    let bundle = ctx.bundle(a.project())?;
    let out = read(ctx, &bundle, |p| {
        Ok(ove_interchange::export_with_report(p.doc(), Some(p.bundle()), f, ove_interchange::Rate::from_fps(fps)))
    })?;
    std::fs::write(&file, &out.text)
        .map_err(|e| Fail::new("file.unwritable", format!("could not write {}: {e}", file.display())))?;
    let mut text = format!("wrote {} ({})", file.display(), f.label());
    for line in out.report.lines() {
        text.push_str(&format!("\n  {line}"));
    }
    Ok(Receipt::new(
        json!({"path": file, "format": f.extension(), "bytes": out.text.len(), "report": out.report.to_json()}),
        text,
    ))
}

pub fn import_timeline_file(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let file = PathBuf::from(a.req_str("file")?);
    let f = interchange_format(&file, a.str("format"))?;
    let fps = a.f64("fps").unwrap_or(30.0);
    let text = std::fs::read_to_string(&file)
        .map_err(|e| Fail::new("file.unreadable", format!("could not read {}: {e}", file.display())))?;
    let tl = ove_interchange::read(&text, f, ove_interchange::Rate::from_fps(fps))
        .map_err(|e| Fail::new("import.invalid", format!("{}: {e}", file.display())))?;
    let bundle = ctx.bundle(a.project())?;
    edit(ctx, &bundle, |w| {
        let (ops, summary) = ove_interchange::to_ops(w.doc(), &tl, now_ms());
        if ops.is_empty() {
            return Err(Fail::new("import.empty", format!("{} has no clips", file.display())));
        }
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let rev = w.apply(ops, format!("import {name} ({} clips)", summary.clips))?;
        let v = json!({"rev": rev, "tracks": summary.tracks, "clips": summary.clips, "mediaAdded": summary.media_added, "missing": summary.missing});
        let text = format!(
            "rev {rev}: imported {} clips on {} tracks ({} new media{}){}",
            summary.clips,
            summary.tracks,
            summary.media_added,
            if summary.missing.is_empty() {
                String::new()
            } else {
                format!("; missing: {}", summary.missing.join(", "))
            },
            via(w)
        );
        Ok(Receipt::new(v, text))
    })
}

#[cfg(feature = "media")]
pub fn export_video(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let out = PathBuf::from(a.req_str("out")?);
    let settings =
        ove_render::ExportSettings { height: a.u32_or("height", 1080), fps: a.u32_or("fps", 30), ..Default::default() };
    let bundle = ctx.bundle(a.project())?;
    let (doc, bundle, _) = crate::ctx::snapshot(ctx, &bundle)?;
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mut last = -1i64;
    let res = ove_render::export(&doc, Some(&bundle), &out, &settings, &cancel, |pr| {
        let pct = (pr.fraction * 100.0) as i64;
        if pct != last {
            last = pct;
            ctx.progress(pct, &serde_json::to_value(pr).unwrap_or_default());
        }
    })
    .map_err(|e| {
        Fail::new(
            match e {
                ove_render::ExportError::Refused(_) => "export.refused",
                ove_render::ExportError::Cancelled => "export.cancelled",
                ove_render::ExportError::Ffmpeg(_) => "export.failed",
            },
            e.to_string(),
        )
    })?;
    ctx.progress_done();
    let mut text = format!(
        "exported {} ({}x{}, {:.1} s, {}) in {:.1} s",
        res.path, res.width, res.height, res.duration_seconds, res.video_encoder, res.seconds_taken
    );
    if let Some(w) = &res.encoder_warning {
        text.push_str(&format!("\nwarning: {w}"));
    }
    Ok(Receipt::new(serde_json::to_value(&res).unwrap_or_default(), text))
}

#[cfg(not(feature = "media"))]
pub fn export_video(_: &mut dyn ToolCtx, _: &Args) -> Result<Receipt, Fail> {
    Err(Fail::new("export.unavailable", "this build of ove has no export; build it with `--features media`"))
}
