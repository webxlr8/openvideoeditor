//! Colour readouts for agents: `inspect_color` measures the composited frame (or one item alone) at an exact frame
//! and, with a reference, reports the CIEDE2000 gap and a capped grade patch that moves the shot toward it. Needs the
//! `media` feature (FFmpeg).

use crate::ctx::ToolCtx;
use crate::fail::Fail;
use crate::receipt::Receipt;
use crate::spec::Args;

#[cfg(not(feature = "media"))]
pub fn inspect_color(_: &mut dyn ToolCtx, _: &Args) -> Result<Receipt, Fail> {
    Err(Fail::new("media.unavailable", "this build of ove cannot read media; build it with `--features media`"))
}

#[cfg(feature = "media")]
pub use imp::inspect_color;

#[cfg(feature = "media")]
mod imp {
    use std::path::Path;

    use ove_analyze::color_stats::{self, ColorStats, Gap};
    use ove_core::model::Document;
    use ove_core::ops::{Op, apply_batch};
    use ove_core::time::Micros;
    use ove_render::eval::{LayerKind, layers_at};
    use ove_render::{Renderer, Rgba};
    use serde_json::{Map, Value, json};

    use super::*;
    use crate::ctx::{read, snapshot};
    use crate::tools::media::{find_media, media_file};

    /// Default measuring height: the export default, so the numbers are those of the exported frame.
    const HEIGHT: u32 = 1080;

    /// A time on the frame grid of a whole-timeline export: (frame index, its start time).
    fn snap(doc: &Document, seconds: f64) -> (u64, Micros) {
        let fps = (doc.fps().round() as u32).clamp(1, 120) as f64;
        let frame = (seconds.max(0.0) * fps + 1e-6).floor() as u64;
        (frame, (frame as i128 * 1_000_000 / fps as i128) as Micros)
    }

    fn secs(us: Micros) -> f64 {
        (us as f64 / 1e3).round() / 1e3
    }

    /// The document with only `item` in it (an item rendered alone).
    fn alone(doc: &Document, item: &str) -> Document {
        let mut d = doc.clone();
        d.project.items =
            doc.project.items.iter().filter(|(id, _)| *id == item).map(|(k, v)| (k.clone(), v.clone())).collect();
        d
    }

    /// The frame at `t`: composited, or with `isolate` only that item (transparent elsewhere).
    fn render(r: &mut Renderer, doc: &Document, t: Micros, isolate: Option<&str>) -> Rgba {
        r.set_transparent(isolate.is_some());
        match isolate {
            Some(id) => r.render(&alone(doc, id), t),
            None => r.render(doc, t),
        }
    }

    /// The topmost video or image item visible at `t` (the shot a suggestion grades when no item is named).
    fn top_shot(doc: &Document, t: Micros) -> Option<String> {
        layers_at(doc, t)
            .into_iter()
            .rev()
            .find(|l| matches!(l.kind, LayerKind::Video { .. } | LayerKind::Image { .. }))
            .map(|l| l.item_id)
    }

    struct Reference {
        json: Value,
        stats: ColorStats,
        pixels: Rgba,
    }

    /// Measures the reference: an item rendered alone at a timeline time (default its middle), or a media frame at a
    /// source time (default 0).
    fn reference(r: &mut Renderer, doc: &Document, bundle: &Path, q: &str, at: Option<f64>) -> Result<Reference, Fail> {
        if let Some(item) = doc.item(q) {
            let seconds = at.unwrap_or((item.start + item.end) as f64 / 2e6);
            let (frame, t) = snap(doc, seconds);
            if t < item.start || t >= item.end {
                return Err(Fail::new(
                    "reference.outOfRange",
                    format!(
                        "reference item {q} is on the timeline from {:.3} s to {:.3} s, not at {seconds:.3} s",
                        item.start as f64 / 1e6,
                        item.end as f64 / 1e6
                    ),
                ));
            }
            let img = render(r, doc, t, Some(q));
            let stats = color_stats::color_stats(&img, true);
            if stats.pixels == 0 {
                return Err(Fail::new(
                    "reference.empty",
                    format!("reference item {q} shows nothing at {:.3} s", secs(t)),
                ));
            }
            return Ok(Reference {
                json: json!({"kind": "item", "id": q, "at": secs(t), "frame": frame}),
                pixels: color_stats::opaque_strip(&img, true),
                stats,
            });
        }
        let id = find_media(doc, bundle, q).map_err(|_| {
            Fail::new("reference.notFound", format!("reference \"{q}\" is not an item or media in this project"))
        })?;
        let file = media_file(doc, bundle, &id)?;
        let at = at.unwrap_or(0.0).max(0.0);
        let img = ove_analyze::grab(&file, at, None).map_err(|e| Fail::new(e.code(), e.to_string()))?;
        let img = Rgba { w: img.width, h: img.height, data: img.rgba };
        let stats = color_stats::color_stats(&img, false);
        Ok(Reference {
            json: json!({"kind": "media", "id": id, "at": at}),
            pixels: color_stats::opaque_strip(&img, false),
            stats,
        })
    }

    fn opt(v: Option<f64>) -> String {
        v.map(|x| format!("{x:.1}")).unwrap_or_else(|| "–".into())
    }

    /// The readable report.
    fn text(head: &str, s: &ColorStats, gap: Option<&Gap>) -> String {
        let zone = |z: &Option<color_stats::Zone>| match z {
            Some(z) => format!(
                "{:.0}% rgb({:.3}, {:.3}, {:.3}) Lab({:.1}, {:.1}, {:.1})",
                z.share, z.rgb[0], z.rgb[1], z.rgb[2], z.lab[0], z.lab[1], z.lab[2]
            ),
            None => "none".into(),
        };
        let hue: Vec<String> = s.hue.iter().enumerate().map(|(i, h)| format!("{}°:{h:.0}", i * 30)).collect();
        let skin = match s.skin.deviation {
            Some(d) => format!("{:.1}% of pixels, {d:+.1}° from the skin line", s.skin.share),
            None => format!("{:.1}% of pixels (too few to judge)", s.skin.share),
        };
        let mut out = format!(
            "{head}\nluma mean {:.1}% (p2 {:.1}, p98 {:.1}) · clipped low {:.2}% high {:.2}%\n\
             shadows {}\nmidtones {}\nhighlights {}\n\
             balance a* {:+.2} b* {:+.2} · saturation {:.3} · colourful {:.1}%\nhue {}\nskin {skin}",
            s.luma.mean,
            s.luma.p2,
            s.luma.p98,
            s.clipping.low,
            s.clipping.high,
            zone(&s.zones.shadows),
            zone(&s.zones.midtones),
            zone(&s.zones.highlights),
            s.balance.a,
            s.balance.b,
            s.saturation,
            s.colorful,
            hue.join(" ")
        );
        if let Some(g) = gap {
            out.push_str(&format!(
                "\nreference gap ΔE00 mean {:.2} (shadows {}, midtones {}, highlights {}) · luma {:+.1} · a* {:+.2} b* {:+.2}",
                g.mean,
                opt(g.shadows),
                opt(g.midtones),
                opt(g.highlights),
                g.luma,
                g.a,
                g.b
            ));
        }
        out
    }

    pub fn inspect_color(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
        let bundle = ctx.bundle(a.project())?;
        let (doc, bundle, rev) = snapshot(ctx, &bundle)?;
        let height = a.u32_or("height", HEIGHT).clamp(16, 2160);
        let item = a.str("item");
        let isolate = a.bool("isolate");
        if isolate && item.is_none() {
            return Err(Fail::new("argument.missing", "isolate measures one item: give its id with item"));
        }
        let shot = match item {
            Some(id) => Some(
                doc.item(id)
                    .cloned()
                    .ok_or_else(|| Fail::new("item.notFound", format!("no item \"{id}\" in this project")))?,
            ),
            None => None,
        };
        let seconds = match (a.f64("at"), &shot) {
            (Some(s), _) => s,
            (None, Some(i)) => (i.start + i.end) as f64 / 2e6,
            (None, None) => return Err(Fail::new("argument.missing", "give a time (at) or an item")),
        };
        if !seconds.is_finite() || seconds < 0.0 {
            return Err(Fail::new("argument.invalid", "at must be seconds from the timeline start"));
        }
        let duration = doc.duration();
        let (frame, t) = snap(&doc, seconds);
        if t >= duration {
            return Err(Fail::new(
                "time.outOfRange",
                format!("{seconds:.3} s is past the end of the timeline, which is {:.3} s long", duration as f64 / 1e6),
            )
            .details(json!({"durationSeconds": duration as f64 / 1e6})));
        }
        if let Some(i) = &shot
            && isolate
            && (t < i.start || t >= i.end)
        {
            return Err(Fail::new(
                "item.notAtTime",
                format!(
                    "item {} is on the timeline from {:.3} s to {:.3} s, not at {:.3} s",
                    i.id,
                    i.start as f64 / 1e6,
                    i.end as f64 / 1e6,
                    secs(t)
                ),
            ));
        }
        // Check the reference before rendering anything.
        let reference_q = a.str("reference");
        if let Some(q) = reference_q
            && doc.item(q).is_none()
            && find_media(&doc, &bundle, q).is_err()
        {
            return Err(Fail::new(
                "reference.notFound",
                format!("reference \"{q}\" is not an item or media in this project"),
            ));
        }
        let isolated = isolate.then_some(item).flatten();
        let chosen = crate::tools::backend::choose(a)?;
        let backend = crate::tools::backend::json(&chosen);
        ove_media::init();
        let mut renderer = Renderer::with_backend(Some(&bundle), height, chosen.backend);
        let img = render(&mut renderer, &doc, t, isolated);
        let stats = color_stats::color_stats(&img, isolated.is_some());
        if stats.pixels == 0 {
            return Err(Fail::new(
                "item.empty",
                format!("item {} shows nothing at {:.3} s", item.unwrap_or("?"), secs(t)),
            ));
        }
        let mut v = json!({
            "rev": rev,
            "at": secs(t),
            "frame": frame,
            "fps": (doc.fps().round() as u32).clamp(1, 120),
            "width": img.w,
            "height": img.h,
            "item": item,
            "isolate": isolate,
            "backend": backend,
            "stats": stats,
        });
        let head = format!(
            "frame {frame} at {:.3} s ({}x{}, rev {rev}){}",
            secs(t),
            img.w,
            img.h,
            match (item, isolate) {
                (Some(i), true) => format!(", item {i} alone"),
                _ => String::new(),
            }
        );
        let Some(q) = reference_q else {
            let text = text(&head, &stats, None);
            return Ok(Receipt::new(v, text));
        };
        let r = reference(&mut renderer, &doc, &bundle, q, a.f64("referenceAt"))?;
        let gap = color_stats::gap(&stats, &r.stats);
        v["reference"] = r.json.clone();
        v["reference"]["stats"] = serde_json::to_value(&r.stats).unwrap_or(Value::Null);
        v["gap"] = serde_json::to_value(&gap).unwrap_or(Value::Null);
        let mut text = text(&head, &stats, Some(&gap));
        // The suggestion grades the named item, or the topmost video or image at that time.
        let target = item.map(str::to_string).or_else(|| top_shot(&doc, t));
        let target =
            target.and_then(|id| doc.item(&id).cloned()).filter(|i| matches!(i.kind.as_str(), "video" | "image"));
        match target {
            Some(target) => {
                // Fit against what the shot looks like now: alone when isolated, else the composited frame.
                let shot_px = color_stats::opaque_strip(&img, isolated.is_some());
                let strength = a.f64("strength").unwrap_or(1.0);
                let grade = color_stats::suggest_grade(&r.pixels, &shot_px, target.fields.get("colorGrade"), strength);
                let set: Map<String, Value> = [("colorGrade".to_string(), grade)].into_iter().collect();
                let ops = vec![Op::UpdateItem { id: target.id.clone(), set }];
                v["suggestedOps"] = serde_json::to_value(&ops).unwrap_or(Value::Null);
                if let Some(edit) = suggested_edit(ctx, &bundle, &doc, rev, &target.id, &ops) {
                    v["suggestedEdit"] = edit;
                }
                text.push_str(&format!(
                    "\nsuggested colorGrade for {}: {}",
                    target.id, v["suggestedOps"][0]["set"]["colorGrade"]
                ));
            }
            None => text.push_str("\nno video or image item to grade at this time: no suggestion"),
        }
        Ok(Receipt::new(v, text))
    }

    /// The suggestion as an `apply_timeline` edit: the item's printed element before and after, against `rev`.
    fn suggested_edit(
        ctx: &mut dyn ToolCtx,
        bundle: &Path,
        doc: &Document,
        rev: i64,
        id: &str,
        ops: &[Op],
    ) -> Option<Value> {
        let after = apply_batch(doc, ops).ok()?.doc;
        let (old, new) = read(ctx, bundle, |p| {
            if p.rev() != rev {
                return Ok((None, None));
            }
            let before = crate::edits::elements(&crate::edits::printed_at(p, rev)?);
            let printed = crate::edits::elements(&ove_projection::print(&after, &p.print_context()));
            Ok((before.get(id).cloned(), printed.get(id).cloned()))
        })
        .ok()?;
        let (old, new) = (old?, new?);
        (old != new).then(|| json!({"rev": rev, "edits": [{"oldText": old, "newText": new}]}))
    }
}
