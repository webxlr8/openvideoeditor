//! `layer_geometry` (add-viewer-interaction-and-text-fills D1): where each visible layer is on screen at a time, from
//! the same placement the compositor draws with. Agents use it to check framing and overlaps without guessing from
//! pixels. Needs the `media` feature (the renderer).

use crate::ctx::ToolCtx;
use crate::fail::Fail;
use crate::receipt::Receipt;
use crate::spec::Args;

pub const ABOUT: &str = "Where every visible layer is on screen at a time: for each layer (bottom first) its item id and type, its stacking index, the four corners of its source after transform (quad) and of the part its crop keeps (cropQuad), its rotation and opacity. Corners are canvas pixels from the frame's top-left corner, x right and y down, in the order top-left, top-right, bottom-right, bottom-left of the source. Use it to check framing, safe areas and overlaps.";

#[cfg(not(feature = "media"))]
pub fn layer_geometry(_: &mut dyn ToolCtx, _: &Args) -> Result<Receipt, Fail> {
    Err(Fail::new("media.unavailable", "this build of ove cannot read media; build it with `--features media`"))
}

#[cfg(feature = "media")]
pub fn layer_geometry(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    use ove_core::time::{micros_to_json, secs_to_micros};
    use ove_render::eval::reference_size;
    use ove_render::geometry::{layer_geometry_with, recorded_size};
    use serde_json::json;

    use crate::ctx::snapshot;

    let bundle = ctx.bundle(a.project())?;
    let (doc, bundle, rev) = snapshot(ctx, &bundle)?;
    let at = a.f64("at").ok_or_else(|| Fail::new("argument.missing", "give --at <seconds>"))?;
    let duration = doc.duration();
    let t = secs_to_micros(at)
        .filter(|t| *t >= 0)
        .ok_or_else(|| Fail::new("argument.invalid", format!("--at must be seconds from 0, not {at}")))?;
    if t >= duration.max(1) {
        return Err(Fail::new(
            "time.pastEnd",
            format!(
                "{at} s is past the end of the timeline, which is {} s long",
                micros_to_json(duration).as_f64().unwrap_or(0.0)
            ),
        ));
    }
    let (_, ref_h) = reference_size(&doc.project.aspect_ratio);
    let height = a.u32_or("height", ref_h).clamp(16, 4320);
    // Sizes from the media records; a record without them is decoded.
    let mut renderer = ove_render::Renderer::new(Some(&bundle), height);
    let g =
        layer_geometry_with(&doc, t, height, &mut |l| recorded_size(&doc, l).or_else(|| renderer.source_size(&doc, l)));
    let round = |p: &[[f64; 2]; 4]| p.map(|[x, y]| [(x * 100.0).round() / 100.0, (y * 100.0).round() / 100.0]);
    let layers: Vec<_> = g
        .layers
        .iter()
        .map(|l| {
            json!({"itemId": l.item_id, "itemType": l.item_type, "index": l.index, "quad": round(&l.quad),
                   "cropQuad": round(&l.crop_quad), "rotation": l.rotation, "opacity": l.opacity,
                   "selectable": l.selectable})
        })
        .collect();
    let text = if layers.is_empty() {
        format!("nothing is visible at {at} s")
    } else {
        g.layers
            .iter()
            .map(|l| {
                let q = round(&l.crop_quad);
                let (xs, ys) = (q.map(|p| p[0]), q.map(|p| p[1]));
                let min = |v: [f64; 4]| v.iter().copied().fold(f64::MAX, f64::min);
                let max = |v: [f64; 4]| v.iter().copied().fold(f64::MIN, f64::max);
                format!(
                    "{}\t{}\t{}\tx {}..{}\ty {}..{}\trotation {}",
                    l.index,
                    l.item_id,
                    l.item_type,
                    min(xs),
                    max(xs),
                    min(ys),
                    max(ys),
                    l.rotation
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let v = json!({"rev": rev, "at": at, "width": g.width, "height": g.height, "layers": layers});
    Ok(Receipt::new(v, text))
}

/// Adds the geometry lints to verify: `text.outsideTitleSafe` (needs the renderer; without the `media` feature
/// verify runs without it).
pub fn register_lints(registry: &mut ove_projection::verify::Registry) {
    #[cfg(feature = "media")]
    registry.register(Box::new(lint::OutsideTitleSafe));
    #[cfg(not(feature = "media"))]
    let _ = registry;
}

#[cfg(feature = "media")]
pub mod lint {
    //! `text.outsideTitleSafe` (add-viewer-interaction-and-text-fills, design D4): a text or caption item that
    //! extends outside the title-safe area (5 % in from each edge) at any sampled time of its duration, measured
    //! with the layer geometry the compositor draws with.

    use ove_core::model::{Document, Item};
    use ove_core::time::Micros;
    use ove_projection::verify::{Context, Finding, Level, Lint};
    use ove_render::eval::reference_size;
    use ove_render::geometry::layer_geometry;
    use serde_json::Value;

    pub const CODE: &str = "text.outsideTitleSafe";
    /// Title-safe inset from each edge (the same as the viewer and `look --overlay safe`).
    pub const INSET: f64 = 0.05;
    /// Sampling step through an item's duration, and the most samples per item.
    const STEP: Micros = 250_000;
    const MAX_SAMPLES: i64 = 40;

    pub struct OutsideTitleSafe;

    /// Times to look at: the item's start, then every 0.25 s (at most 40 times), plus the middle of each caption
    /// segment (a short segment between two samples is not missed).
    fn times(item: &Item) -> Vec<Micros> {
        let dur = (item.end - item.start).max(1);
        let n = ((dur + STEP - 1) / STEP).clamp(1, MAX_SAMPLES);
        let mut out: Vec<Micros> = (0..n).map(|i| item.start + i * dur / n).collect();
        if let Some(segs) = item.fields.get("captionSegments").and_then(Value::as_array) {
            for s in segs.iter().take(200) {
                let t = |k: &str| s.get(k).and_then(ove_core::time::json_to_micros);
                if let (Some(a), Some(b)) = (t("startTime"), t("endTime")) {
                    out.push(item.start + (a + b) / 2);
                }
            }
        }
        out.retain(|t| *t >= item.start && *t < item.end);
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Where the layer of `item` leaves the safe area at `t`, if it does: the sides and where they are.
    fn outside(doc: &Document, item: &Item, t: Micros) -> Option<String> {
        let (w, h) = reference_size(&doc.project.aspect_ratio);
        let g = layer_geometry(doc, t, h);
        let layer = g.layers.iter().find(|l| l.item_id == item.id)?;
        let xs = layer.crop_quad.map(|p| p[0]);
        let ys = layer.crop_quad.map(|p| p[1]);
        let min = |v: [f64; 4]| v.iter().copied().fold(f64::MAX, f64::min);
        let max = |v: [f64; 4]| v.iter().copied().fold(f64::MIN, f64::max);
        let (fw, fh) = (w as f64, h as f64);
        // Half a pixel of tolerance for rounding.
        let tol = 0.5;
        let pct = |v: f64| format!("{:.1} %", v * 100.0);
        let mut sides = Vec::new();
        if min(xs) < fw * INSET - tol {
            sides.push(format!("left edge at {} of the width", pct(min(xs) / fw)));
        }
        if max(xs) > fw * (1.0 - INSET) + tol {
            sides.push(format!("right edge at {} of the width", pct(max(xs) / fw)));
        }
        if min(ys) < fh * INSET - tol {
            sides.push(format!("top edge at {} of the height", pct(min(ys) / fh)));
        }
        if max(ys) > fh * (1.0 - INSET) + tol {
            sides.push(format!("bottom edge at {} of the height", pct(max(ys) / fh)));
        }
        (!sides.is_empty()).then(|| sides.join(", "))
    }

    impl Lint for OutsideTitleSafe {
        fn code(&self) -> &'static str {
            CODE
        }

        fn run(&self, ctx: &Context, out: &mut Vec<Finding>) {
            let doc = ctx.doc;
            for item in doc.items_in_canonical_order() {
                if !matches!(item.kind.as_str(), "text" | "caption")
                    || item.is_pending()
                    || item.fields.get("hidden").and_then(Value::as_bool) == Some(true)
                {
                    continue;
                }
                for t in times(item) {
                    if let Some(sides) = outside(doc, item, t) {
                        let what = if item.kind == "caption" { "caption" } else { "text" };
                        out.push(Finding {
                            lint: CODE,
                            severity: Level::Warning,
                            item_ids: vec![item.id.clone()],
                            message: format!(
                                "{what} {} is outside the title-safe area (90 % of the frame) at {} s: {sides}",
                                item.id,
                                ove_core::time::micros_to_json(t)
                            ),
                        });
                        break;
                    }
                }
            }
        }
    }
}
