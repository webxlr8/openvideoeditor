//! Media search for agents and the command line (add-semantic-media-search D7): `search_media` (Moments, Spoken,
//! Timelines and Files, each Moment and Spoken hit with a clip fragment to paste into `timeline.tsx`),
//! `media_transcript` and `inspect_media` (with an overview sheet). They read the bundle only, so they work headless
//! on a closed project. Visual search needs the model; when it is missing the result says so with its size and
//! nothing is downloaded.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use ove_core::model::Document;
use ove_models::worker::WorkerClient;
use ove_vision::index::{Index, Lookup, lookup};
use ove_vision::model::{SiglipEncoder, installed, provenance_of};
use ove_vision::rank::{Moment, Scoring, Source, frame_query, rank};
use ove_vision::spoken::SpokenIndex;
use serde_json::{Value, json};

use crate::ctx::{ToolCtx, read};
use crate::fail::Fail;
use crate::receipt::Receipt;
use crate::spec::Kind::{Bool, Int, Json, Num, Str};
use crate::spec::{Arg, Args, arg, cli, req};
use crate::tools::media::find_media;
use crate::tools::transcript::any_transcript;

const PROJECT: Arg = crate::registry::PROJECT;
const SCOPE: &str = r#"{"type": "string", "enum": ["all", "moments", "spoken", "timelines", "files"]}"#;
const SIMILAR: &str = r#"{"type": "object", "properties": {"media": {"type": "string"}, "at": {"type": "number"}}, "required": ["media", "at"], "additionalProperties": false}"#;

pub const SEARCH_ABOUT: &str = "Search the project's media by what it shows and what is said, on this computer. Sections: moments (shots ranked by a local image–text model, each with its best frame, shot range, a suggested window of up to 4 s and a confidence 0–1), spoken (ranked phrase search over transcripts; quote a phrase to match it exactly; ranges padded by 150 ms), timelines (clips and titles whose name or text matches) and files (names, folders, tags). Each moment and spoken hit has `clip`: a `<clip …/>` line to paste into a track of timeline.tsx (set `start`/`end` where it goes, keeping the length). `similarTo` (media and time) finds shots that look like that frame. `index` reports what is searchable: visual not installed (with the download size; the person installs it in Settings > Storage, nothing is downloaded here), indexing (n of N files) or ready, and how many files have no transcript.";

pub const SEARCH_ARGS: &[Arg] = &[
    PROJECT,
    arg("query", Str, "What to find: words, or a quoted phrase for exact spoken matches"),
    arg("scope", Json(SCOPE), "Which sections to search (default all)"),
    arg("media", Str, "Only this media item (id, file name or timeline name)"),
    arg("limit", Int, "Results per section (default 20, at most 50)"),
    arg("similarTo", Json(SIMILAR), "A frame as the query: {media, at} in source seconds"),
];

pub const TRANSCRIPT_ABOUT: &str = "The transcript of one media file in source seconds: caption-sized lines with their times and the words with word times, from the transcripts already made on this computer (transcribe_media makes one). `from`/`to` limit it to a source range.";

pub const TRANSCRIPT_ARGS: &[Arg] = &[
    PROJECT,
    req("media", Str, "Media id, file name or timeline name"),
    arg("from", Num, "Source seconds to start at"),
    arg("to", Num, "Source seconds to end at"),
];

pub const INSPECT_ABOUT: &str = "Look at one media file: its probe facts, shots and index state; with `overview`, one sheet of at most 36 tiles with time labels, one per shot, built from the shot list (sampling seeks to each shot, it does not decode the whole file again).";

pub const INSPECT_ARGS: &[Arg] = &[
    PROJECT,
    req("media", Str, "Media id, file name or timeline name"),
    arg("overview", Bool, "Return the overview sheet"),
    arg("tileWidth", Int, "Tile width of the sheet in pixels (default 240)"),
    cli("out", Str, "Write the sheet to this PNG file"),
];

/// Results per section by default, and at most.
const LIMIT: usize = 20;
const MAX_LIMIT: usize = 50;
/// Length a still image gets on the timeline.
const IMAGE_SECONDS: f64 = 5.0;

static WORKER_PROGRAM: OnceLock<PathBuf> = OnceLock::new();

/// The program that serves `model-worker` (the desktop app sets it to the `ove` beside it). Set once, before the first
/// search; later calls are ignored.
pub fn set_worker_program(p: PathBuf) {
    let _ = WORKER_PROGRAM.set(p);
}

fn default_program() -> PathBuf {
    let name = if cfg!(windows) { "ove.exe" } else { "ove" };
    let exe = std::env::current_exe().ok();
    if let Some(e) = &exe
        && e.file_stem().is_some_and(|s| s == "ove")
    {
        return e.clone();
    }
    exe.and_then(|e| e.parent().map(|d| d.join(name))).filter(|p| p.is_file()).unwrap_or_else(|| PathBuf::from(name))
}

/// The model worker this process shares: started on the first model call, it exits by itself when idle.
pub fn shared_worker() -> Arc<WorkerClient> {
    static WORKER: OnceLock<Arc<WorkerClient>> = OnceLock::new();
    WORKER
        .get_or_init(|| {
            let program = WORKER_PROGRAM.get().cloned().unwrap_or_else(default_program);
            Arc::new(WorkerClient::new(program, &["model-worker"]))
        })
        .clone()
}

/// The model store: `OVE_MODELS_DIR`, else the app's.
fn model_store() -> Option<ove_models::Store> {
    ove_models::default_root().map(ove_models::Store::open)
}

/// What the tools need to know about one media item.
#[derive(Debug, Clone)]
struct Row {
    id: String,
    /// The projection name (`src`).
    name: String,
    kind: String,
    content_id: Option<String>,
    file_name: String,
    folder: Option<String>,
    tags: Vec<String>,
    duration: Option<f64>,
}

fn rows(doc: &Document) -> Vec<Row> {
    let names = ove_projection::print::media_names(doc);
    doc.project
        .media
        .iter()
        .map(|(id, m)| {
            let d = m.data();
            let get = |k: &str| d.and_then(|d| d.get(k));
            Row {
                id: id.clone(),
                name: names.get(id).cloned().unwrap_or_else(|| id.clone()),
                kind: get("type").and_then(Value::as_str).unwrap_or("").to_string(),
                content_id: doc
                    .registry
                    .get(id)
                    .and_then(|e| e.fields.get("contentId"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                file_name: m.file_name().unwrap_or("").to_string(),
                folder: m.fields.get("folder").and_then(Value::as_str).map(str::to_string),
                tags: m
                    .fields
                    .get("tags")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                    .unwrap_or_default(),
                duration: get("duration").and_then(ove_core::time::json_to_micros).map(|us| us as f64 / 1e6),
            }
        })
        .collect()
}

/// Seconds as the projection prints them: at most three decimals, no trailing zeros.
fn secs(x: f64) -> String {
    let r = (x * 1000.0).round() / 1000.0;
    let s = format!("{r:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

/// A clip element for `timeline.tsx` that plays `[from, to)` of the media: placed at 0 s; the agent sets `start`
/// and `end` (keeping `end − start`).
pub fn clip_fragment(kind: &str, name: &str, from: f64, to: f64) -> String {
    let src = serde_json::to_string(name).unwrap_or_else(|_| format!("\"{name}\""));
    match kind {
        "image" => format!("<image start={{0}} end={{{}}} src={src} />", secs(IMAGE_SECONDS)),
        "audio" => format!("<audio start={{0}} end={{{}}} in={{{}}} src={src} />", secs(to - from), secs(from)),
        _ => format!("<clip start={{0}} end={{{}}} in={{{}}} src={src} />", secs(to - from), secs(from)),
    }
}

/// Folds text for name and timeline matching (as transcript search folds words).
fn folded(s: &str) -> String {
    s.split_whitespace().map(ove_transcribe::search::fold).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// The visual section and its health. Without the model, text queries find nothing (and nothing is downloaded);
/// `similarTo` still works on existing indexes made by one model, because it compares stored frames only.
fn visual(
    bundle: &Path,
    rows: &[Row],
    query: Option<&str>,
    similar: Option<(String, f64)>,
    limit: usize,
) -> Result<(Vec<Moment>, Value), Fail> {
    let visual_rows: Vec<&Row> = rows.iter().filter(|r| matches!(r.kind.as_str(), "video" | "image")).collect();
    let total = visual_rows.len();
    let (files, mut health) = match model_store().map(|s| installed(&s)) {
        Some(Ok(f)) => (Some(f), json!({})),
        Some(Err(n)) => (
            None,
            json!({"visual": "notInstalled", "model": n.id, "downloadBytes": n.download_bytes, "host": n.host,
                "message": n.message, "install": n.install}),
        ),
        None => (None, json!({"visual": "notInstalled", "message": "no model folder on this computer"})),
    };
    if files.is_none() && similar.is_none() {
        health["total"] = total.into();
        health["indexed"] = 0.into();
        return Ok((Vec::new(), health));
    }
    let prov = files.as_ref().map(provenance_of);
    let mut indexes: Vec<(String, Index)> = Vec::new();
    for r in &visual_rows {
        let Some(cid) = &r.content_id else { continue };
        if let Lookup::Ready(i) = lookup(bundle, cid, prov.as_ref()) {
            indexes.push((r.id.clone(), i));
        }
    }
    if let Some((media, _)) = &similar {
        // Frames compare only with frames of the same model.
        let model = indexes.iter().find(|(id, _)| id == media).map(|(_, i)| i.provenance.clone());
        indexes.retain(|(_, i)| model.as_ref().is_none_or(|m| i.provenance == *m));
    }
    let indexed = indexes.len();
    if files.is_some() {
        health["visual"] = if indexed < total { "indexing" } else { "ready" }.into();
    }
    health["indexed"] = indexed.into();
    health["total"] = total.into();
    if indexes.is_empty() {
        return Ok((Vec::new(), health));
    }
    let (q, scoring) = match (similar, query, files) {
        (Some((media, at)), _, _) => {
            let Some((_, i)) = indexes.iter().find(|(id, _)| *id == media) else {
                return Err(Fail::new("index.missing", format!("media {media} is not indexed yet")));
            };
            let q = frame_query(i, at).ok_or_else(|| Fail::new("index.empty", "that media has no sampled frames"))?;
            (q, Scoring::SIMILAR)
        }
        (None, Some(text), Some(files)) => {
            let mut enc = SiglipEncoder::new(shared_worker(), files).map_err(vision_fail)?;
            (ove_vision::query::embed(bundle, &mut enc, text).map_err(vision_fail)?, Scoring::TEXT)
        }
        _ => return Ok((Vec::new(), health)),
    };
    let sources: Vec<Source> = indexes.iter().map(|(id, index)| Source { media_id: id, index }).collect();
    Ok((rank(&sources, &q, scoring, limit), health))
}

fn vision_fail(e: ove_vision::VisionError) -> Fail {
    Fail::new(e.code(), e.to_string())
}

/// `search_media`.
pub fn search_media(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let query = a.str("query").map(str::trim).filter(|q| !q.is_empty()).map(str::to_string);
    let similar = a.value("similarTo").cloned();
    if query.is_none() && similar.is_none() {
        return Err(Fail::new("argument.missing", "give `query`, `similarTo` or both"));
    }
    let scope = a.str("scope").unwrap_or("all").to_string();
    let limit = a.i64("limit").map(|l| l.clamp(1, MAX_LIMIT as i64) as usize).unwrap_or(LIMIT);
    let want = |s: &str| scope == "all" || scope == s;
    let bundle = ctx.bundle(a.project())?;
    let (doc, bundle, rev) = read(ctx, &bundle, |p| Ok((p.doc().clone(), p.bundle().to_path_buf(), p.rev())))?;
    let mut all = rows(&doc);
    if let Some(m) = a.str("media") {
        let id = find_media(&doc, &bundle, m)?;
        all.retain(|r| r.id == id);
    }
    let similar = match &similar {
        Some(s) => {
            let m = s.get("media").and_then(Value::as_str).unwrap_or("");
            let at =
                s.get("at").and_then(Value::as_f64).filter(|t| t.is_finite() && *t >= 0.0).ok_or_else(|| {
                    Fail::new("argument.invalid", "`similarTo.at` must be source seconds (0 or more)")
                })?;
            Some((find_media(&doc, &bundle, m)?, at))
        }
        None => None,
    };
    let everything = rows(&doc);
    let name_of = |id: &str| -> (String, String) {
        everything
            .iter()
            .find(|r| r.id == id)
            .map(|r| (r.name.clone(), r.kind.clone()))
            .unwrap_or_else(|| (id.to_string(), String::new()))
    };
    let mut text = Vec::new();
    let mut out = serde_json::Map::new();

    // Moments.
    let mut index_health = Value::Null;
    if want("moments") {
        let (moments, health) = visual(&bundle, &all, query.as_deref(), similar.clone(), limit)?;
        index_health = health;
        let hits: Vec<Value> = moments
            .iter()
            .map(|m| {
                let (name, kind) = name_of(&m.media_id);
                json!({
                    "media": m.media_id, "name": name, "shot": [m.shot_start, m.shot_end], "best": m.best,
                    "in": m.window_start, "out": m.window_end, "confidence": (m.confidence * 1000.0).round() / 1000.0,
                    "clip": clip_fragment(&kind, &name, m.window_start, m.window_end),
                })
            })
            .collect();
        match index_health["visual"].as_str() {
            Some("notInstalled") => text.push(format!(
                "moments: model not installed ({}); nothing was downloaded",
                index_health["message"].as_str().unwrap_or("visual search model")
            )),
            _ if hits.is_empty() => text.push(format!(
                "moments: no visual matches ({} of {} files indexed)",
                index_health["indexed"], index_health["total"]
            )),
            _ => {
                text.push(format!(
                    "moments: {} (indexed {} of {})",
                    hits.len(),
                    index_health["indexed"],
                    index_health["total"]
                ));
                for h in &hits {
                    text.push(format!(
                        "  {} {}–{} s best {} ({:.2})  {}",
                        h["name"].as_str().unwrap_or(""),
                        secs(h["in"].as_f64().unwrap_or(0.0)),
                        secs(h["out"].as_f64().unwrap_or(0.0)),
                        secs(h["best"].as_f64().unwrap_or(0.0)),
                        h["confidence"].as_f64().unwrap_or(0.0),
                        h["clip"].as_str().unwrap_or("")
                    ));
                }
            }
        }
        out.insert("moments".into(), json!(hits));
    }

    // Spoken.
    let mut spoken_health = json!({});
    if want("spoken")
        && let Some(q) = &query
    {
        let mut have = Vec::new();
        let mut untranscribed = Vec::new();
        let speech: Vec<&Row> = all.iter().filter(|r| matches!(r.kind.as_str(), "video" | "audio")).collect();
        for r in &speech {
            match any_transcript(&bundle, &doc, &r.id) {
                Some(t) => have.push((r.id.clone(), t)),
                None => untranscribed.push(r.id.clone()),
            }
        }
        let index = SpokenIndex::new(have);
        let hits: Vec<Value> = index
            .search(q, limit)
            .into_iter()
            .map(|h| {
                let (name, kind) = name_of(&h.media_id);
                json!({
                    "media": h.media_id, "name": name, "text": h.text, "context": h.context,
                    "in": h.start, "out": h.end, "score": h.score,
                    "clip": clip_fragment(&kind, &name, h.start, h.end),
                })
            })
            .collect();
        text.push(format!("spoken: {} in {} words", hits.len(), index.words()));
        for h in &hits {
            text.push(format!(
                "  {} {}–{} s \"{}\"  {}",
                h["name"].as_str().unwrap_or(""),
                secs(h["in"].as_f64().unwrap_or(0.0)),
                secs(h["out"].as_f64().unwrap_or(0.0)),
                h["text"].as_str().unwrap_or(""),
                h["clip"].as_str().unwrap_or("")
            ));
        }
        if !untranscribed.is_empty() {
            text.push(format!("  {} files not transcribed (transcribe_media makes transcripts)", untranscribed.len()));
        }
        spoken_health = json!({"transcribed": speech.len() - untranscribed.len(), "untranscribed": untranscribed.len(),
            "untranscribedMedia": untranscribed});
        out.insert("spoken".into(), json!(hits));
    }

    // Timelines: clips and titles whose name or text matches.
    if want("timelines")
        && let Some(q) = &query
    {
        let fq = folded(q.trim_matches('"'));
        let hits: Vec<Value> = doc
            .project
            .items
            .values()
            .filter_map(|i| {
                let text_of = |k: &str| i.fields.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let (name, body) = (text_of("name"), text_of("textContent"));
                let hay = folded(&format!("{name} {body}"));
                (!fq.is_empty() && hay.contains(&fq)).then(|| {
                    json!({"item": i.id, "type": i.kind, "track": i.track_id, "start": i.start as f64 / 1e6,
                        "end": i.end as f64 / 1e6, "name": name, "text": body})
                })
            })
            .take(limit)
            .collect();
        text.push(format!("timelines: {}", hits.len()));
        for h in &hits {
            text.push(format!(
                "  {} {} {}–{} s",
                h["item"].as_str().unwrap_or(""),
                h["type"].as_str().unwrap_or(""),
                secs(h["start"].as_f64().unwrap_or(0.0)),
                secs(h["end"].as_f64().unwrap_or(0.0))
            ));
        }
        out.insert("timelines".into(), json!(hits));
    }

    // Files: names, file names, folders and tags.
    if want("files")
        && let Some(q) = &query
    {
        let fq = folded(q.trim_matches('"'));
        let hits: Vec<Value> = all
            .iter()
            .filter(|r| {
                let hay = folded(&format!(
                    "{} {} {} {}",
                    r.name,
                    r.file_name,
                    r.folder.as_deref().unwrap_or(""),
                    r.tags.join(" ")
                ));
                !fq.is_empty() && hay.contains(&fq)
            })
            .take(limit)
            .map(|r| {
                json!({"media": r.id, "name": r.name, "type": r.kind, "folder": r.folder, "tags": r.tags,
                    "duration": r.duration})
            })
            .collect();
        text.push(format!("files: {}", hits.len()));
        for h in &hits {
            text.push(format!("  {} ({})", h["name"].as_str().unwrap_or(""), h["media"].as_str().unwrap_or("")));
        }
        out.insert("files".into(), json!(hits));
    }

    let mut index = index_health;
    if let (Some(o), Some(s)) = (index.as_object_mut(), spoken_health.as_object()) {
        o.extend(s.clone());
    } else if index.is_null() {
        index = spoken_health;
    }
    out.insert("index".into(), index);
    out.insert("rev".into(), rev.into());
    out.insert("query".into(), json!(query));
    Ok(Receipt::new(Value::Object(out), text.join("\n")))
}

/// `media_transcript`.
pub fn media_transcript(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let q = a.req_str("media")?.to_string();
    let (from, to) = (a.f64("from").unwrap_or(0.0), a.f64("to").unwrap_or(f64::INFINITY));
    if !(from >= 0.0 && to > from) {
        return Err(Fail::new("argument.invalid", "`to` must be after `from` (source seconds)"));
    }
    let bundle = ctx.bundle(a.project())?;
    read(ctx, &bundle, |p| {
        let (doc, bundle) = (p.doc(), p.bundle());
        let id = find_media(doc, bundle, &q)?;
        let t = any_transcript(bundle, doc, &id).ok_or_else(|| {
            Fail::new("transcript.missing", format!("media {id} has no transcript yet; transcribe_media makes one"))
        })?;
        let words: Vec<_> = t
            .words
            .iter()
            .filter(|w| w.end_us as f64 / 1e6 > from && (w.start_us as f64 / 1e6) < to)
            .cloned()
            .collect();
        let segments = ove_transcribe::segment(&words, &Default::default());
        let lines: Vec<Value> = segments
            .iter()
            .map(|s| json!({"start": s.start_us as f64 / 1e6, "end": s.end_us as f64 / 1e6, "text": s.text}))
            .collect();
        let text = segments
            .iter()
            .map(|s| format!("{}–{} {}", secs(s.start_us as f64 / 1e6), secs(s.end_us as f64 / 1e6), s.text))
            .collect::<Vec<_>>()
            .join("\n");
        let words: Vec<Value> = words
            .iter()
            .map(|w| json!({"text": w.text, "start": w.start_us as f64 / 1e6, "end": w.end_us as f64 / 1e6}))
            .collect();
        Ok(Receipt::new(
            json!({"media": id, "language": t.language, "model": t.model, "lines": lines, "words": words}),
            if text.is_empty() { "no speech in that range".to_string() } else { text },
        ))
    })
}

#[cfg(not(feature = "media"))]
pub fn inspect_media(_: &mut dyn ToolCtx, _: &Args) -> Result<Receipt, Fail> {
    Err(Fail::new("media.unavailable", "this build of ove cannot read media; build it with `--features media`"))
}

/// Up to `max` sample times of an index for an overview: each shot's first sample, then the other samples spread
/// evenly; with more shots than tiles, shots spread evenly.
pub fn overview_times(i: &Index, max: usize) -> Vec<f64> {
    let firsts: Vec<usize> = (0..i.shots.len()).filter_map(|s| (0..i.rows()).find(|&r| i.shot_of(r) == s)).collect();
    let pick = |from: &[usize], n: usize| -> Vec<usize> {
        if from.len() <= n { from.to_vec() } else { (0..n).map(|k| from[k * from.len() / n]).collect() }
    };
    let mut rows = pick(&firsts, max);
    let rest: Vec<usize> = (0..i.rows()).filter(|r| !rows.contains(r)).collect();
    let room = max.saturating_sub(rows.len());
    rows.extend(pick(&rest, room));
    rows.sort_unstable();
    rows.into_iter().map(|r| i.time(r)).collect()
}

/// `inspect_media`.
#[cfg(feature = "media")]
pub fn inspect_media(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    use ove_analyze::storyboard::{MAX_TILES, Tile, render};
    let q = a.req_str("media")?.to_string();
    let bundle = ctx.bundle(a.project())?;
    let (doc, bundle) = read(ctx, &bundle, |p| Ok((p.doc().clone(), p.bundle().to_path_buf())))?;
    let id = find_media(&doc, &bundle, &q)?;
    let file = crate::tools::media::media_file(&doc, &bundle, &id)?;
    let row = rows(&doc).into_iter().find(|r| r.id == id).ok_or_else(|| Fail::new("media.notFound", q.clone()))?;
    let index = row.content_id.as_deref().and_then(|c| Index::open(&ove_vision::index::index_path(&bundle, c)).ok());
    let shots: Vec<(f64, f64)> = index.as_ref().map(|i| i.shots.clone()).unwrap_or_default();
    let mut v = json!({
        "media": id, "name": row.name, "type": row.kind, "duration": row.duration, "folder": row.folder,
        "tags": row.tags, "indexed": index.is_some(), "shots": shots.len(),
    });
    let mut text = format!(
        "{} ({}, {}){}",
        row.name,
        row.kind,
        row.duration.map(|d| format!("{} s", secs(d))).unwrap_or_else(|| "-".into()),
        if index.is_some() { format!(", {} shots indexed", shots.len()) } else { ", not indexed".into() }
    );
    if !a.bool("overview") {
        return Ok(Receipt::new(v, text));
    }
    let tile_width = a.u32_or("tileWidth", 240).clamp(64, 640);
    let fail = |e: &dyn std::fmt::Display| Fail::new("media.unreadable", format!("{}: {e}", file.display()));
    let (img, tiles) = match &index {
        // Tiles from the shot list: every shot once (its first sampled frame), the rest of the 36 filled evenly from
        // the other samples; more than 36 shots are spread evenly.
        Some(i) if i.rows() > 0 => {
            let times = overview_times(i, MAX_TILES);
            let frames = ove_vision::sampler::decode_at(&file, &times).map_err(|e| fail(&e))?;
            let tiles: Vec<Tile> = frames
                .into_iter()
                .zip(&times)
                .map(|(f, &at)| Tile { at, image: f.resize_to_width(tile_width), items: vec![] })
                .collect();
            (render(&tiles), tiles)
        }
        _ => ove_analyze::storyboard::media(&file, tile_width).map_err(|e| fail(&e))?,
    };
    let bytes = img.png().map_err(|e| Fail::new("image.encode", e.to_string()))?;
    if let Some(o) = a.str("out") {
        std::fs::write(o, &bytes).map_err(|e| Fail::new("file.write", format!("{o}: {e}")))?;
        v["path"] = json!(o);
        text.push_str(&format!("\nwrote {o}"));
    }
    let times: Vec<f64> = tiles.iter().map(|t| t.at).collect();
    v["sheet"] = json!({"width": img.width, "height": img.height, "tiles": tiles.len(), "times": times,
        "from": if index.is_some() { "shots" } else { "samples" }});
    text.push_str(&format!("\noverview: {} tiles ({}x{})", tiles.len(), img.width, img.height));
    let png = crate::receipt::Png { bytes, width: img.width, height: img.height };
    Ok(Receipt::new(v, text).with_images(vec![png]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_fragments_are_projection_elements() {
        assert_eq!(
            clip_fragment("video", "harbour", 12.4, 16.4),
            r#"<clip start={0} end={4} in={12.4} src="harbour" />"#
        );
        assert_eq!(
            clip_fragment("audio", "voice \"one\"", 1.0 / 3.0, 2.0),
            r#"<audio start={0} end={1.667} in={0.333} src="voice \"one\"" />"#
        );
        assert_eq!(clip_fragment("image", "still", 0.0, 0.0), r#"<image start={0} end={5} src="still" />"#);
        assert_eq!(secs(2.0), "2");
        assert_eq!(secs(0.1 + 0.2), "0.3");
    }
}
