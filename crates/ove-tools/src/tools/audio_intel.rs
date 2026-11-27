//! Audio intelligence tools (add-audio-intelligence): `sync_items` aligns recordings by timecode or sound, as one
//! revision or a dry run. They need the `media` feature (sound sync decodes audio).

use crate::ctx::ToolCtx;
use crate::fail::Fail;
use crate::receipt::Receipt;
use crate::spec::Args;

#[cfg(not(feature = "media"))]
const NO_MEDIA: &str = "this build of ove cannot read media; build it with `--features media`";

#[cfg(not(feature = "media"))]
pub fn sync_items(_: &mut dyn ToolCtx, _: &Args) -> Result<Receipt, Fail> {
    Err(Fail::new("media.unavailable", NO_MEDIA))
}

#[cfg(feature = "media")]
pub use imp::sync_items;

/// Item ids named directly or through a track (id or name, any case), from a JSON list or a comma-separated string.
pub fn item_names(doc: &ove_core::model::Document, a: &Args, k: &str) -> Result<Vec<String>, Fail> {
    let list = crate::tools::audio::names(a, k)?;
    let mut out = Vec::new();
    for n in list {
        if doc.item(&n).is_some() {
            out.push(n);
        } else if let Some(t) = doc.project.tracks.iter().find(|t| t.id == n || t.name.eq_ignore_ascii_case(&n)) {
            out.extend(doc.items_on_track(&t.id).iter().map(|i| i.id.clone()));
        } else {
            return Err(Fail::new("item.unknown", format!("no item or track {n}")));
        }
    }
    Ok(out)
}

#[cfg(feature = "media")]
mod imp {
    use std::collections::BTreeMap;

    use ove_analyze::sync::SyncParams;
    use ove_analyze::sync_place::{self, BundleAudio, Mode};
    use serde_json::json;

    use super::*;
    use crate::ctx::{edit, via};
    use crate::fail::plural;

    /// Media files of the document, with their content ids.
    pub(crate) fn bundle_audio(doc: &ove_core::model::Document, bundle: &std::path::Path) -> BundleAudio {
        let files = doc
            .registry
            .iter()
            .map(|(id, e)| {
                let path = ove_store::media::resolve_media_path(bundle, &e.native_path);
                let key = e.fields.get("contentId").and_then(|v| v.as_str()).map(str::to_string);
                (id.clone(), (path, key))
            })
            .collect::<BTreeMap<_, _>>();
        BundleAudio::new(bundle, files)
    }

    pub fn sync_items(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
        let reference = a.req_str("reference")?.to_string();
        let mode = Mode::parse(a.str("mode").unwrap_or("auto"))
            .ok_or_else(|| Fail::new("argument.invalid", "`mode` is auto, timecode or audio"))?;
        let min_confidence = a.f64("minConfidence").unwrap_or(SyncParams::default().min_confidence);
        if !(0.0..=1.0).contains(&min_confidence) {
            return Err(Fail::new("argument.invalid", format!("`minConfidence` must be 0–1, got {min_confidence}")));
        }
        let window_s = a.f64("window").filter(|w| *w > 0.0);
        let drift = match a.str("drift").unwrap_or("auto") {
            "auto" => true,
            "off" => false,
            d => return Err(Fail::new("argument.invalid", format!("`drift` is auto or off, not {d}"))),
        };
        let params = SyncParams { min_confidence, window_s, drift, ..SyncParams::default() };
        let dry = a.bool("dryRun");
        let bundle = ctx.bundle(a.project())?;
        edit(ctx, &bundle, |w| {
            let doc = w.doc().clone();
            if doc.item(&reference).is_none() {
                return Err(Fail::new("item.unknown", format!("no item {reference}")));
            }
            let targets: Vec<String> =
                item_names(&doc, a, "targets")?.into_iter().filter(|t| *t != reference).collect();
            if targets.is_empty() {
                return Err(Fail::new("argument.invalid", "`targets` names no item other than the reference"));
            }
            let mut media = bundle_audio(&doc, w.bundle());
            let plan = sync_place::plan(&doc, &reference, &targets, mode, &params, &mut media)
                .map_err(|e| Fail::new(e.code(), e.to_string()))?;
            let mut v = json!({
                "reference": plan.reference,
                "targets": plan.targets,
                "groupShift": plan.group_shift,
                "placed": plan.placed,
                "dryRun": dry,
            });
            let not_placed: Vec<String> = plan
                .targets
                .iter()
                .filter(|t| t.status != "placed")
                .map(|t| format!("{} ({})", t.item, t.reason.as_deref().unwrap_or(t.status)))
                .collect();
            if plan.placed == 0 {
                v["rev"] = w.rev().into();
                return Err(Fail::new(
                    "sync.unmatched",
                    format!("no target was synced to {reference}: {}", not_placed.join("; ")),
                )
                .details(v));
            }
            let what = format!(
                "{} to {reference}{}",
                plural(plan.placed, "item"),
                if not_placed.is_empty() { String::new() } else { format!("; not synced: {}", not_placed.join("; ")) }
            );
            if dry {
                v["rev"] = w.rev().into();
                return Ok(Receipt::new(v, format!("would sync {what}")));
            }
            if plan.ops.is_empty() {
                v["rev"] = w.rev().into();
                return Ok(Receipt::new(v, format!("already in sync: {what}")));
            }
            let rev = w.apply(plan.ops.clone(), format!("sync {} to {reference}", plural(plan.placed, "item")))?;
            v["rev"] = rev.into();
            Ok(Receipt::new(v, format!("rev {rev}: synced {what}{}", via(w))))
        })
    }
}
