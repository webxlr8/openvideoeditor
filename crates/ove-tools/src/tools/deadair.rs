//! Dead air and fillers for agents (add-transcript-editing D5, D7): `remove_silence`, the voice-activity mode of
//! `detect_silence` (`mode: "speech"`), and `find_fillers`. Speech masks and levels are cached in the bundle per media
//! content identity; computing them needs the inference runtime (installed with the person's consent) and media
//! reading.

use std::path::Path;

use ove_core::Document;
use ove_core::time::Micros;
use ove_transcribe::cuts::{CutRange, plan_cut};
use ove_transcribe::deadair::{DeadAirOptions, detect, fillers};
use ove_transcribe::timeline::SourceClip;
use ove_transcribe::vad::SpeechMask;
use serde_json::{Value, json};

use crate::ctx::{ToolCtx, edit};
use crate::fail::Fail;
use crate::receipt::Receipt;
use crate::spec::Args;
use crate::tools::transcript::{applies, content_id, cut_error, envelope, finish_cut, words_of};

fn options(a: &Args) -> DeadAirOptions {
    let d = DeadAirOptions::default();
    let us = |k: &str, def: i64| a.f64(k).map(|s| (s * 1e6) as i64).unwrap_or(def);
    DeadAirOptions {
        relative_db: a.f64("relativeDb").map(|v| v as f32).unwrap_or(d.relative_db),
        min_pause_us: us("minPause", d.min_pause_us),
        padding_us: us("padding", d.padding_us),
    }
    .clamped()
}

/// The speech mask of a media file: cached in the bundle, else computed with the voice-activity model through the
/// shared runtime (and cached). Without the runtime: `model.runtimeMissing` with how to install it.
pub fn speech_mask(bundle: &Path, doc: &Document, path: &Path, media: &str) -> Result<SpeechMask, Fail> {
    let key = content_id(doc, media);
    if let Some(m) = key.as_deref().and_then(|k| ove_transcribe::vad::load(bundle, k)) {
        return Ok(m);
    }
    let mask = compute_mask(path)?;
    if let Some(k) = key {
        ove_transcribe::vad::store(bundle, &k, &mask);
    }
    Ok(mask)
}

#[cfg(feature = "media")]
fn compute_mask(path: &Path) -> Result<SpeechMask, Fail> {
    use ove_models::infer::Backend;
    let store = crate::tools::models::open_store()?;
    let lib = ove_models::runtime::library_path(&store)?;
    // The voice-activity model is 2.2 MB: it downloads on first use, with progress, from its pinned location.
    let paths = match store.ensure_installed(
        ove_models::manifest::VAD_ID,
        ove_models::Origin::Feature,
        &mut |_| {},
        &std::sync::atomic::AtomicBool::new(false),
    )? {
        ove_models::Ensure::Ready(p) => p,
        ove_models::Ensure::NeedsConsent(n) => return Err(Fail::new("model.consent", n.message)),
    };
    let model = paths.first().ok_or_else(|| Fail::new("model.unknown", "the voice-activity model has no file"))?;
    let backend = ove_models::onnx::OnnxBackend::load(&lib)?.with_threads(1);
    let mut session = backend.load(model)?;
    let mut vad = ove_transcribe::vad::OnnxVad::default();
    let mut err = None;
    ove_transcribe::levels::decode(path, &mut |chunk| {
        if err.is_none()
            && let Err(e) = vad.push(session.as_mut(), chunk)
        {
            err = Some(e);
        }
    })
    .map_err(|e| Fail::new(e.code(), e.to_string()))?;
    if let Some(e) = err {
        return Err(Fail::new(e.code(), e.to_string()));
    }
    vad.finish(session.as_mut()).map_err(|e| Fail::new(e.code(), e.to_string()))
}

#[cfg(not(feature = "media"))]
fn compute_mask(_: &Path) -> Result<SpeechMask, Fail> {
    Err(Fail::new("media.unavailable", "this build of ove cannot read media; build it with `--features media`"))
}

/// Dead air of every clip, as timeline cuts. Returns the cuts and, per media, the source ranges and levels.
fn plan_dead_air(
    bundle: &Path,
    doc: &Document,
    clips: &[SourceClip],
    opts: DeadAirOptions,
) -> Result<(Vec<CutRange>, Vec<Value>, bool), Fail> {
    let mut cuts = Vec::new();
    let mut per_media: Vec<Value> = Vec::new();
    let mut any_speech = false;
    let mut seen: Vec<String> = Vec::new();
    for clip in clips {
        let mask = speech_mask(bundle, doc, &clip.path, &clip.media_id)?;
        let env = envelope(bundle, doc, clips, &clip.media_id)
            .ok_or_else(|| Fail::new("media.unavailable", format!("no audio levels for {}", clip.media_id)))?;
        let duration = env.db.len() as Micros * env.cell_us;
        let dead = detect(&mask, &env, duration, opts);
        any_speech |= dead.speech_found;
        if !seen.contains(&clip.media_id) {
            seen.push(clip.media_id.clone());
            per_media.push(json!({
                "media": clip.media_id,
                "speechFound": dead.speech_found,
                "speechDb": dead.speech_db,
                "ranges": dead.ranges.iter().map(|(a, b)| json!({"start": *a as f64 / 1e6, "end": *b as f64 / 1e6})).collect::<Vec<_>>(),
            }));
        }
        let (lo, hi) = clip.source_range();
        for (a, b) in dead.ranges {
            // Inside the clip's source range; padding is not kept at the clip's own edges.
            let (mut a, mut b) = (a.max(lo), b.min(hi));
            if b <= a {
                continue;
            }
            if a - lo <= opts.padding_us {
                a = lo;
            }
            if hi - b <= opts.padding_us {
                b = hi;
            }
            let (s, e) = (clip.to_timeline(a), clip.to_timeline(b));
            cuts.push(CutRange { clip_id: clip.item_id.clone(), start: s.min(e), end: s.max(e), word_ids: vec![] });
        }
    }
    cuts.sort_by_key(|c| c.start);
    Ok((cuts, per_media, any_speech))
}

/// `remove_silence`: cuts dead air (pauses well below the speaker's level) from the timeline as one revision.
pub fn remove_silence(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let opts = options(a);
    let apply = applies(a);
    let bundle = ctx.bundle(a.project())?;
    edit(ctx, &bundle, |w| {
        let (bundle, doc) = (w.bundle().to_path_buf(), w.doc().clone());
        let (_, _, clips) = words_of(&bundle, &doc, a)?;
        if clips.is_empty() {
            return Err(Fail::new("transcript.noClips", "There is no clip with sound."));
        }
        let (cuts, media, speech) = plan_dead_air(&bundle, &doc, &clips, opts)?;
        let settings = json!({"minPause": opts.min_pause_us as f64 / 1e6, "padding": opts.padding_us as f64 / 1e6, "relativeDb": opts.relative_db});
        if !speech || cuts.is_empty() {
            let why = if speech { "no dead air found" } else { "no speech was found, so nothing was removed" };
            let v = json!({"rev": w.rev(), "changed": false, "speechFound": speech, "media": media, "settings": settings, "message": why});
            return Ok(Receipt::new(v, why.to_string()));
        }
        let total: Micros = cuts.iter().map(|c| c.end - c.start).sum();
        let mut cut =
            plan_cut(&doc, cuts, vec![], &mut crate::tools::transcript::cut_ids(w.rev())).map_err(cut_error)?;
        cut.plan.summary = format!("remove dead air ({} cuts, {:.1} s)", cut.ranges.len(), total as f64 / 1e6);
        finish_cut(
            w,
            cut,
            apply,
            &doc,
            json!({"speechFound": true, "media": media, "settings": settings, "changed": apply}),
        )
    })
}

/// `detect_silence` with `mode: "speech"`: dead air per media in source time; the document is not changed.
pub fn detect_dead_air(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let opts = options(a);
    let bundle = ctx.bundle(a.project())?;
    let media = a.str("media").map(str::to_string);
    let mut out = None;
    ctx.read(&bundle, &mut |p| {
        let (_, _, clips) = words_of(p.bundle(), p.doc(), a)?;
        let clips: Vec<SourceClip> = clips
            .into_iter()
            .filter(|c| media.as_deref().is_none_or(|m| c.media_id == m || c.path.file_name().is_some_and(|f| f == m)))
            .collect();
        if clips.is_empty() {
            return Err(Fail::new("media.notFound", "no clip with sound uses that media"));
        }
        let (_, per_media, speech) = plan_dead_air(p.bundle(), p.doc(), &clips, opts)?;
        let n: usize = per_media.iter().map(|m| m["ranges"].as_array().map_or(0, Vec::len)).sum();
        let text = if speech { format!("{n} dead-air ranges") } else { "no speech was found".to_string() };
        out = Some(Receipt::new(json!({"mode": "speech", "speechFound": speech, "media": per_media}), text));
        Ok(())
    })?;
    out.ok_or_else(|| Fail::new("internal", "no result"))
}

/// `find_fillers`: filler candidates on the timeline (listed filler words, context words flagged but not selected,
/// voiced sounds no word covers), with word ids for `remove_words`. Nothing is cut.
pub fn find_fillers(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let bundle = ctx.bundle(a.project())?;
    let mut out = None;
    ctx.read(&bundle, &mut |p| {
        let (words, untranscribed, clips) = words_of(p.bundle(), p.doc(), a)?;
        let mut list = Vec::new();
        for w in &words {
            if w.filler || w.context_filler {
                list.push(json!({"kind": "word", "id": w.id, "text": w.text, "start": w.start, "end": w.end,
                    "selected": w.filler && !w.context_filler, "context": w.context_filler}));
            }
        }
        // Voiced gaps need a cached speech mask (made by dead-air detection); none are guessed without one.
        for clip in &clips {
            let Some(key) = content_id(p.doc(), &clip.media_id) else { continue };
            let Some(mask) = ove_transcribe::vad::load(p.bundle(), &key) else { continue };
            let t = ove_transcribe::Transcript {
                language: "en".into(),
                model: String::new(),
                words: words
                    .iter()
                    .filter(|w| w.media_id == clip.media_id)
                    .map(|w| ove_transcribe::Word {
                        text: w.text.clone(),
                        start_us: (w.source_start * 1e6) as i64,
                        end_us: (w.source_end * 1e6) as i64,
                        confidence: 1.0,
                    })
                    .collect(),
                duration_us: 0,
            };
            let (lo, hi) = clip.source_range();
            for f in fillers(&t, Some(&mask))
                .into_iter()
                .filter(|f| f.kind == "voiced" && f.start_us >= lo && f.end_us <= hi)
            {
                let (s, e) = (clip.to_timeline(f.start_us), clip.to_timeline(f.end_us));
                list.push(json!({"kind": "voiced", "clipId": clip.item_id, "start": s.min(e) as f64 / 1e6,
                    "end": s.max(e) as f64 / 1e6, "selected": true, "context": false}));
            }
        }
        let selected = list.iter().filter(|f| f["selected"] == true).count();
        let text = format!("{} filler candidates, {selected} selected by default", list.len());
        out = Some(Receipt::new(json!({"rev": p.rev(), "fillers": list, "untranscribed": untranscribed}), text));
        Ok(())
    })?;
    out.ok_or_else(|| Fail::new("internal", "no result"))
}

/// `detect_silence`: level mode (quieter than a threshold, the default) or `mode: "speech"` (dead air judged by voice
/// activity against the speaker's level).
pub fn detect_silence(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    match a.str("mode").unwrap_or("level") {
        "speech" => detect_dead_air(ctx, a),
        "level" => crate::tools::analyze::detect_silence(ctx, a),
        other => Err(Fail::new("argument.invalid", format!("`mode` is level or speech, not {other}"))),
    }
}
