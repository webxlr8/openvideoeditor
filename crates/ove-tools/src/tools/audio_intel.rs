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

#[cfg(not(feature = "media"))]
pub fn detect_beats(_: &mut dyn ToolCtx, _: &Args) -> Result<Receipt, Fail> {
    Err(Fail::new("media.unavailable", NO_MEDIA))
}

#[cfg(feature = "media")]
pub use beats_imp::detect_beats;

#[cfg(feature = "media")]
mod beats_imp {
    use std::path::{Path, PathBuf};

    use ove_analyze::beats::{self, Activations, BeatAnalysis, MELS, MODEL_ID, PINS};
    use ove_models::infer::{Backend, Tensor};
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};

    use super::*;

    /// The model files in the store, when both are present with their pinned size and hash.
    fn model_files(root: &Path) -> Option<Vec<PathBuf>> {
        PINS.iter()
            .map(|pin| {
                let p = root.join(MODEL_ID).join(pin.name);
                let bytes = std::fs::read(&p).ok()?;
                let ok = bytes.len() as u64 == pin.bytes && format!("{:x}", Sha256::digest(&bytes)) == pin.sha256;
                ok.then_some(p)
            })
            .collect()
    }

    fn output(out: &[Tensor], index: usize, what: &str) -> Result<Vec<f32>, Fail> {
        out.iter()
            .find(|t| t.name == what)
            .or_else(|| out.get(index))
            .and_then(|t| t.as_f32())
            .map(<[f32]>::to_vec)
            .ok_or_else(|| Fail::new("model.inference", format!("the beat model gave no {what}")))
    }

    fn infer(path: &Path, files: &[PathBuf]) -> Result<BeatAnalysis, Fail> {
        use ove_media::{AudioDecoder, AudioOutputSpec, SampleFormat, Samples};
        let store = crate::tools::models::open_store()?;
        let lib = ove_models::runtime::library_path(&store)?;
        let backend = ove_models::onnx::OnnxBackend::load(&lib)?;
        let mut front = backend.load(&files[0])?;
        let mut net = backend.load(&files[1])?;
        ove_media::init();
        let spec = AudioOutputSpec { sample_rate: beats::MODEL_RATE, channels: 1, format: SampleFormat::F32 };
        let unreadable = |e: ove_media::MediaError| Fail::new("media.unreadable", e.to_string());
        let mut dec = AudioDecoder::open(path, spec).map_err(unreadable)?;
        let mut samples = Vec::new();
        while let Some(c) = dec.next_chunk().map_err(unreadable)? {
            if let Samples::F32(s) = c.samples {
                samples.extend_from_slice(&s);
            }
        }
        let mut run_front = |seg: &[f32]| -> Result<Vec<f32>, Fail> {
            let out = front.run(&[Tensor::f32("samples", &[1, seg.len()], seg.to_vec())])?;
            output(&out, 0, "spect")
        };
        let mut run_net = |chunk: &[f32], frames: usize| -> Result<Activations, Fail> {
            let out = net.run(&[Tensor::f32("spect", &[1, frames, MELS], chunk.to_vec())])?;
            Ok(Activations { beat: output(&out, 0, "beat")?, downbeat: output(&out, 1, "downbeat")? })
        };
        let act = beats::run_pipeline(&samples, &mut run_front, &mut run_net)?;
        let (b, d) = beats::pick(&act);
        Ok(beats::analyse(&b, &d, MODEL_ID))
    }

    fn within(v: &[f64], from: Option<f64>, to: Option<f64>) -> Vec<f64> {
        v.iter().copied().filter(|t| from.is_none_or(|f| *t >= f) && to.is_none_or(|e| *t <= e)).collect()
    }

    fn not_installed(root: &Path) -> Receipt {
        let bytes: u64 = PINS.iter().map(|p| p.bytes).sum();
        let folder = root.join(MODEL_ID);
        let files: Vec<Value> =
            PINS.iter().map(|p| json!({"name": p.name, "bytes": p.bytes, "sha256": p.sha256})).collect();
        let v = json!({"status": "modelNotInstalled", "model": MODEL_ID, "bytes": bytes,
            "folder": folder.display().to_string(), "files": files});
        let text = format!(
            "the beat model is not installed ({:.1} MB); nothing was downloaded. Its files go in {} (see tools/notices/beat-this.md)",
            bytes as f64 / 1e6,
            folder.display()
        );
        Receipt::new(v, text)
    }

    /// `detect_beats`: beats, downbeats, bars, meter and tempo of a media file or an item, from the cache or the
    /// local beat model. Never downloads.
    pub fn detect_beats(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
        let q = a.req_str("target")?.to_string();
        let (from, to) = (a.f64("from"), a.f64("to"));
        let bundle = ctx.bundle(a.project())?;
        let (doc, bundle, _) = crate::ctx::snapshot(ctx, &bundle)?;
        let item = doc.item(&q).cloned();
        let media = match &item {
            Some(i) => i
                .media_id()
                .filter(|_| matches!(i.kind.as_str(), "audio" | "video"))
                .map(str::to_string)
                .ok_or_else(|| Fail::new("item.noSound", format!("item {q} has no sound")))?,
            None => crate::tools::media::find_media(&doc, &bundle, &q)?,
        };
        let path = crate::tools::media::media_file(&doc, &bundle, &media)?;
        let key = crate::tools::transcript::content_id(&doc, &media).or_else(|| {
            path.is_file()
                .then(|| ove_store::media::identity_fields(&path))
                .and_then(|f| f.get("contentId")?.as_str().map(str::to_string))
        });
        let cached = key.as_deref().and_then(|k| beats::load(&bundle, k));
        let was_cached = cached.is_some();
        let analysis = match cached {
            Some(c) => c,
            None => {
                let root = ove_models::default_root().ok_or_else(|| {
                    Fail::new("model.store", "could not find the app data folder (set OVE_MODELS_DIR)")
                })?;
                let Some(files) = model_files(&root) else { return Ok(not_installed(&root)) };
                if !path.is_file() {
                    return Err(Fail::new("media.offline", format!("{} is offline", path.display())));
                }
                let r = infer(&path, &files)?;
                if let Some(k) = &key {
                    let _ = beats::store(&bundle, k, &r);
                }
                r
            }
        };
        let (sf, st) = if item.is_some() { (None, None) } else { (from, to) };
        let mut v = json!({
            "status": "ready",
            "media": media,
            "model": analysis.model,
            "cached": was_cached,
            "meter": analysis.meter,
            "bpm": analysis.bpm,
            "beats": within(&analysis.beats, sf, st),
            "downbeats": within(&analysis.downbeats, sf, st),
            "bars": analysis.bars,
            "tempo": analysis.tempo,
        });
        let mut text = format!(
            "{} beats, {} downbeats{}{}",
            analysis.beats.len(),
            analysis.downbeats.len(),
            analysis.meter.map(|m| format!(", {m} beats per bar")).unwrap_or_default(),
            analysis.bpm.map(|b| format!(", {b} BPM")).unwrap_or_default()
        );
        if let Some(i) = &item {
            let speed = i.fields.get("playbackSpeed").and_then(Value::as_f64).unwrap_or(1.0).max(0.01);
            let tb = within(&beats::to_timeline(i, &analysis.beats), from, to);
            let td = within(&beats::to_timeline(i, &analysis.downbeats), from, to);
            v["item"] = json!(i.id);
            v["timelineBeats"] = json!(tb);
            v["timelineDownbeats"] = json!(td);
            v["timelineBpm"] = json!(analysis.bpm.map(|b| (b * speed * 100.0).round() / 100.0));
            text = format!("{text}; {} beats on the timeline for item {}", tb.len(), i.id);
        }
        Ok(Receipt::new(v, text))
    }
}

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
