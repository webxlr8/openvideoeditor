//! `get_transcript` (add-transcript-editing D1, D2): the timeline transcript of a project revision from the cached
//! transcripts of its media, with word ids anchored to the source. Read-only: media without a transcript are listed
//! as untranscribed (`transcribe_media` makes them).

use std::collections::HashMap;
use std::path::Path;

use ove_core::Document;
use ove_transcribe::Transcript;
use ove_transcribe::cuts::{CutError, Envelope, Pause, Selection, cut_ranges, plan_cut, select};
use ove_transcribe::timeline::{self, TimelineWord};
use serde_json::{Value, json};

use crate::ctx::{ToolCtx, edit, via};
use crate::fail::Fail;
use crate::receipt::Receipt;
use crate::spec::Args;

/// The cached transcript of each media id used by `clips`: a given model (and language) when named, else the most
/// recent transcript of that media.
fn cached(
    bundle: &Path,
    doc: &Document,
    clips: &[timeline::SourceClip],
    model: Option<&str>,
    language: Option<&str>,
) -> HashMap<String, Transcript> {
    let mut out = HashMap::new();
    for c in clips {
        if out.contains_key(&c.media_id) {
            continue;
        }
        let cid = doc
            .registry
            .get(&c.media_id)
            .and_then(|e| e.fields.get("contentId"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let t = match (model, &cid) {
            (Some(m), _) => ove_transcribe::cache::lookup(bundle, &c.path, cid.as_deref(), m, language, false),
            (None, Some(cid)) => ove_transcribe::cache::any_for(bundle, cid).into_iter().next(),
            (None, None) => None,
        }
        .or_else(|| {
            // Projects from before content ids: any model under the path's legacy key.
            ove_transcribe::models::Model::ALL
                .iter()
                .find_map(|m| ove_transcribe::cache::lookup(bundle, &c.path, cid.as_deref(), m.id(), language, false))
        });
        if let Some(t) = t {
            out.insert(c.media_id.clone(), t);
        }
    }
    out
}

/// Words on the timeline, media without a transcript, and the clips with sound.
pub type TimelineWords = (Vec<TimelineWord>, Vec<String>, Vec<timeline::SourceClip>);

/// The words of the timeline (all tracks, or the given clips), the media without a transcript, and the clips.
pub fn words_of(bundle: &Path, doc: &Document, a: &Args) -> Result<TimelineWords, Fail> {
    let ids: Vec<String> = a.strings("clips");
    let resolve =
        |m: &str| doc.registry.get(m).map(|e| ove_store::media::resolve_media_path(bundle, e.native_path.as_str()));
    let clips = if ids.is_empty() {
        timeline::all_clips(doc, &resolve)
    } else {
        timeline::source_clips(doc, &ids, &resolve).map_err(|e| Fail::new("transcript.noClips", e.to_string()))?
    };
    let transcripts = cached(bundle, doc, &clips, a.str("model"), a.str("language"));
    let (words, untranscribed) = timeline::words(&clips, &transcripts);
    Ok((words, untranscribed, clips))
}

/// The timeline transcript of a project (all tracks, or the given clips), one page.
pub fn transcript_of(
    bundle: &Path,
    doc: &Document,
    rev: i64,
    a: &Args,
) -> Result<(ove_transcribe::timeline::TimelineTranscript, Vec<timeline::SourceClip>), Fail> {
    let (words, untranscribed, clips) = words_of(bundle, doc, a)?;
    let page = timeline::page(
        rev,
        words,
        untranscribed,
        a.i64("offset").unwrap_or(0).max(0) as usize,
        a.i64("limit").unwrap_or(0).max(0) as usize,
    );
    Ok((page, clips))
}

pub fn content_id(doc: &Document, media: &str) -> Option<String> {
    doc.registry.get(media).and_then(|e| e.fields.get("contentId")).and_then(Value::as_str).map(str::to_string)
}

/// The audio levels of a clip's media: cached in the bundle, else measured (builds with media reading) and cached.
pub fn envelope(bundle: &Path, doc: &Document, clips: &[timeline::SourceClip], media: &str) -> Option<Envelope> {
    let key = content_id(doc, media)?;
    if let Some(e) = ove_transcribe::levels::load(bundle, &key) {
        return Some(e);
    }
    #[cfg(feature = "media")]
    {
        let path = clips.iter().find(|c| c.media_id == media)?.path.clone();
        let e = ove_transcribe::levels::measure(&path).ok()?;
        ove_transcribe::levels::store(bundle, &key, &e);
        Some(e)
    }
    #[cfg(not(feature = "media"))]
    {
        let _ = clips;
        None
    }
}

pub fn cut_error(e: CutError) -> Fail {
    Fail::new(e.code.clone(), e.message.clone()).details(json!({"culprits": e.culprits}))
}

fn selection(a: &Args) -> Result<Selection, Fail> {
    let ranges = match a.value("ranges") {
        Some(v) => serde_json::from_value(v.clone())
            .map_err(|e| Fail::new("argument.invalid", format!("`ranges` must be a list of word-id ranges: {e}")))?,
        None => vec![],
    };
    Ok(Selection { ids: a.strings("ids"), ranges, matches: a.strings("match") })
}

fn pause(a: &Args) -> Result<Pause, Fail> {
    match a.str("pause").unwrap_or("balanced") {
        "tight" => Ok(Pause::Tight),
        "balanced" => Ok(Pause::Balanced),
        "loose" => Ok(Pause::Loose),
        other => Err(Fail::new("argument.invalid", format!("`pause` is tight, balanced or loose, not {other}"))),
    }
}

/// `apply: false` makes a dry run.
pub fn applies(a: &Args) -> bool {
    a.value("apply").and_then(Value::as_bool).unwrap_or(true)
}

/// `remove_words`: cuts the selected words from the timeline as one revision (or, with `apply: false`, returns the
/// plan and the `timeline.tsx` diff without applying it).
pub fn remove_words(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let sel = selection(a)?;
    let pause = pause(a)?;
    let apply = applies(a);
    let bundle = ctx.bundle(a.project())?;
    edit(ctx, &bundle, |w| {
        let (bundle, doc) = (w.bundle().to_path_buf(), w.doc().clone());
        let (words, _, clips) = words_of(&bundle, &doc, a)?;
        let chosen = select(&words, &sel).map_err(cut_error)?;
        let ranges = cut_ranges(&words, &chosen, &clips, pause, &|m| envelope(&bundle, &doc, &clips, m));
        let removed: Vec<String> = chosen.iter().map(|&i| words[i].id.clone()).collect();
        let cut = plan_cut(&doc, ranges, removed, &mut cut_ids(w.rev())).map_err(cut_error)?;
        finish_cut(w, cut, apply, &doc, json!({}))
    })
}

/// Applies a planned cut (or reports it as a dry run) and builds the receipt; `extra` adds fields.
pub fn finish_cut(
    w: &mut dyn crate::ctx::Editor,
    cut: ove_transcribe::cuts::CutPlan,
    apply: bool,
    before: &Document,
    extra: Value,
) -> Result<Receipt, Fail> {
    let mut v = json!({
        "summary": cut.plan.summary,
        "ranges": cut.ranges,
        "removedWords": cut.removed_words,
        "captionsChanged": cut.captions_changed,
        "captionsRemoved": cut.captions_removed,
        "timeMap": cut.plan.time_map,
        "dryRun": !apply,
        "diff": tsx_diff(before, &cut.plan.result),
    });
    if let (Value::Object(m), Value::Object(e)) = (&mut v, extra) {
        m.extend(e);
    }
    if !apply {
        v["rev"] = w.rev().into();
        return Ok(Receipt::new(v, format!("would apply: {}", cut.plan.summary)));
    }
    let new = w.apply_mapped(cut.plan.ops.clone(), cut.plan.summary.clone(), cut.plan.time_map.clone())?;
    v["rev"] = new.into();
    crate::edits::remember_doc(w.bundle(), new, w.doc());
    Ok(Receipt::new(v, format!("rev {new}: {}{}", cut.plan.summary, via(w))))
}

/// The change to `timeline.tsx` between two documents, as a unified diff.
pub fn tsx_diff(before: &Document, after: &Document) -> String {
    let a = ove_projection::print(before, &Default::default());
    let b = ove_projection::print(after, &Default::default());
    unified_diff(&a, &b)
}

/// A unified diff of two texts (whole lines, 2 lines of context).
pub fn unified_diff(a: &str, b: &str) -> String {
    let (x, y): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    let (n, m) = (x.len(), y.len());
    let mut pre = 0;
    while pre < n && pre < m && x[pre] == y[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < n - pre && suf < m - pre && x[n - 1 - suf] == y[m - 1 - suf] {
        suf += 1;
    }
    let (xs, ys) = (&x[pre..n - suf], &y[pre..m - suf]);
    // Longest common subsequence over the changed middle.
    let mut lcs = vec![vec![0u32; ys.len() + 1]; xs.len() + 1];
    for i in (0..xs.len()).rev() {
        for j in (0..ys.len()).rev() {
            lcs[i][j] = if xs[i] == ys[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
        }
    }
    let mut ops: Vec<(char, &str)> = x[..pre].iter().map(|l| (' ', *l)).collect();
    let (mut i, mut j) = (0, 0);
    while i < xs.len() || j < ys.len() {
        if i < xs.len() && j < ys.len() && xs[i] == ys[j] {
            ops.push((' ', xs[i]));
            i += 1;
            j += 1;
        } else if j < ys.len() && (i == xs.len() || lcs[i][j + 1] >= lcs[i + 1][j]) {
            ops.push(('+', ys[j]));
            j += 1;
        } else {
            ops.push(('-', xs[i]));
            i += 1;
        }
    }
    ops.extend(x[n - suf..].iter().map(|l| (' ', *l)));
    hunks(&ops, 2)
}

fn hunks(ops: &[(char, &str)], context: usize) -> String {
    let changed: Vec<usize> = ops.iter().enumerate().filter(|(_, o)| o.0 != ' ').map(|(i, _)| i).collect();
    if changed.is_empty() {
        return String::new();
    }
    let mut out = String::from("--- timeline.tsx\n+++ timeline.tsx\n");
    let mut k = 0;
    while k < changed.len() {
        let start = changed[k].saturating_sub(context);
        let mut end = changed[k] + context;
        while k + 1 < changed.len() && changed[k + 1] <= end + context {
            k += 1;
            end = changed[k] + context;
        }
        let end = end.min(ops.len() - 1);
        let a_start = ops[..start].iter().filter(|o| o.0 != '+').count() + 1;
        let b_start = ops[..start].iter().filter(|o| o.0 != '-').count() + 1;
        let a_len = ops[start..=end].iter().filter(|o| o.0 != '+').count();
        let b_len = ops[start..=end].iter().filter(|o| o.0 != '-').count();
        out.push_str(&format!("@@ -{a_start},{a_len} +{b_start},{b_len} @@\n"));
        for (c, l) in &ops[start..=end] {
            out.push(*c);
            out.push_str(l);
            out.push('\n');
        }
        k += 1;
    }
    out
}

/// Applies a unified diff made by [`unified_diff`] to `a` (tests: the preview equals the applied change).
pub fn apply_diff(a: &str, diff: &str) -> String {
    let lines: Vec<&str> = a.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut at = 0usize;
    for l in diff.lines().skip(2) {
        if let Some(h) = l.strip_prefix("@@ -") {
            let start: usize = h.split(',').next().and_then(|s| s.parse().ok()).unwrap_or(1);
            while at + 1 < start {
                out.push(lines[at].to_string());
                at += 1;
            }
        } else if let Some(t) = l.strip_prefix('+') {
            out.push(t.to_string());
        } else if l.starts_with('-') {
            at += 1;
        } else if let Some(t) = l.strip_prefix(' ') {
            out.push(t.to_string());
            at += 1;
        }
    }
    out.extend(lines[at.min(lines.len())..].iter().map(|s| s.to_string()));
    out.join("\n")
}

pub fn get_transcript(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let bundle = ctx.bundle(a.project())?;
    let segments_mode = a.bool("segments");
    let mut out: Option<Receipt> = None;
    ctx.read(&bundle, &mut |p| {
        let (page, _) = transcript_of(p.bundle(), p.doc(), p.rev(), a)?;
        let mut v = serde_json::to_value(&page).unwrap_or(Value::Null);
        let mut text = format!(
            "rev {} · {} words{}",
            page.rev,
            page.total_words,
            if page.untranscribed.is_empty() {
                String::new()
            } else {
                format!(" · not transcribed yet: {}", page.untranscribed.join(", "))
            }
        );
        if segments_mode {
            let segs = timeline::segments(&page.words);
            for s in &segs {
                text.push_str(&format!("\n{:.2}\t{:.2}\t{}", s.start, s.end, s.text));
            }
            if let Value::Object(m) = &mut v {
                m.remove("words");
                m.insert("segments".into(), json!(segs));
            }
        } else {
            for w in &page.words {
                text.push_str(&format!("\n{}\t{:.2}\t{:.2}\t{}", w.id, w.start, w.end, w.text));
            }
        }
        if let Some(next) = page.next_offset {
            text.push_str(&format!("\nmore: offset {next}"));
        }
        out = Some(Receipt::new(v, text));
        Ok(())
    })?;
    out.ok_or_else(|| Fail::new("internal", "no transcript"))
}

/// Ids for the pieces a cut creates, derived from the revision it is planned on, so a dry run and the apply that
/// follows it (on the same revision) produce the same items and the same diff.
pub fn cut_ids(rev: i64) -> impl FnMut() -> String {
    let mut n = 0u32;
    move || {
        n += 1;
        format!("i_cut{rev:x}_{n}")
    }
}

/// The cached transcript of one media file (any model; the newest), with the legacy path key as a fallback.
fn any_transcript(bundle: &Path, doc: &Document, media: &str) -> Option<Transcript> {
    if let Some(t) = content_id(doc, media).and_then(|c| ove_transcribe::cache::any_for(bundle, &c).into_iter().next())
    {
        return Some(t);
    }
    let path = doc.registry.get(media).map(|e| ove_store::media::resolve_media_path(bundle, e.native_path.as_str()))?;
    ove_transcribe::models::Model::ALL
        .iter()
        .find_map(|m| ove_transcribe::cache::lookup(bundle, &path, None, m.id(), None, false))
}

/// `search_transcript`: a phrase across every transcribed media of the project, ignoring case, accents and
/// punctuation; word-accurate hits with their timeline uses, and how many media have no transcript yet.
pub fn search_transcript(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let phrase = a.req_str("phrase")?.to_string();
    let bundle = ctx.bundle(a.project())?;
    let mut out = None;
    ctx.read(&bundle, &mut |p| {
        let (bundle, doc) = (p.bundle(), p.doc());
        let mut have = Vec::new();
        let mut untranscribed = Vec::new();
        for (id, m) in doc.project.media.iter() {
            let data = m.data();
            let kind = data.and_then(|d| d.get("type")).and_then(Value::as_str).unwrap_or("");
            if kind == "image" || data.and_then(|d| d.get("hasAudio")).and_then(Value::as_bool) == Some(false) {
                continue;
            }
            match any_transcript(bundle, doc, id) {
                Some(t) => have.push((id.clone(), t)),
                None => untranscribed.push(id.clone()),
            }
        }
        let resolve =
            |m: &str| doc.registry.get(m).map(|e| ove_store::media::resolve_media_path(bundle, e.native_path.as_str()));
        let clips = timeline::all_clips(doc, &resolve);
        let index = ove_transcribe::search::Index::new(have);
        let hits = index.find(&phrase, &clips);
        let mut text = format!("{} hits for \"{phrase}\" in {} words", hits.len(), index.words());
        for h in &hits {
            let uses: Vec<String> = h.uses.iter().map(|u| format!("{} at {:.2}", u.clip_id, u.start)).collect();
            text.push_str(&format!(
                "\n{} {:.2}–{:.2} \"{}\"{}",
                h.media_id,
                h.source_start,
                h.source_end,
                h.text,
                if uses.is_empty() { " (not on the timeline)".to_string() } else { format!(" · {}", uses.join(", ")) }
            ));
        }
        if !untranscribed.is_empty() {
            text.push_str(&format!("\nnot transcribed yet: {}", untranscribed.len()));
        }
        out = Some(Receipt::new(
            json!({"rev": p.rev(), "phrase": phrase, "hits": hits, "searchedWords": index.words(), "untranscribed": untranscribed.len(), "untranscribedMedia": untranscribed}),
            text,
        ));
        Ok(())
    })?;
    out.ok_or_else(|| Fail::new("internal", "no result"))
}

/// `transcribe_media`: recognises a media file's speech on this computer and caches the transcript in the project
/// (by content identity, model, language and the verbatim flag). A cached transcript is returned unless `force`.
/// The speech model must be installed: models over 50 MB need the person's consent, which no agent call gives.
pub fn transcribe_media(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let media_q = a.req_str("media")?.to_string();
    let format = a.str("format").unwrap_or("json");
    if !matches!(format, "text" | "words" | "json") {
        return Err(Fail::new(
            "format.unsupported",
            format!(
                "format {format} is not available here; use text, words or json (subtitle files come with caption tools)"
            ),
        ));
    }
    let model = match a.str("model") {
        Some(m) => ove_transcribe::Model::parse(m)
            .ok_or_else(|| Fail::new("argument.invalid", format!("`model` is tiny, base or small, not {m}")))?,
        None => ove_transcribe::Model::Base,
    };
    let (language, verbatim, force) = (a.str("language").map(str::to_string), a.bool("verbatim"), a.bool("force"));
    let bundle = ctx.bundle(a.project())?;
    let mut found: Option<(String, std::path::PathBuf, Option<String>)> = None;
    ctx.read(&bundle, &mut |p| {
        let doc = p.doc();
        let id = doc
            .registry
            .iter()
            .find(|(id, e)| {
                **id == media_q
                    || Path::new(e.native_path.as_str()).file_name().is_some_and(|f| f.to_string_lossy() == media_q)
            })
            .map(|(id, _)| id.clone())
            .ok_or_else(|| Fail::new("media.notFound", format!("no media {media_q} in the project")))?;
        let e = doc.registry.get(&id).ok_or_else(|| Fail::new("media.notFound", media_q.clone()))?;
        found = Some((
            id.clone(),
            ove_store::media::resolve_media_path(p.bundle(), e.native_path.as_str()),
            content_id(doc, &id),
        ));
        Ok(())
    })?;
    let (media, path, cid) = found.ok_or_else(|| Fail::new("media.notFound", media_q.clone()))?;
    let cached = if force {
        None
    } else {
        ove_transcribe::cache::lookup(&bundle, &path, cid.as_deref(), model.id(), language.as_deref(), verbatim)
    };
    let was_cached = cached.is_some();
    let t = match cached {
        Some(t) => t,
        None => recognise(&bundle, &path, cid.as_deref(), model, language.clone(), verbatim)?,
    };
    let mut v = json!({"media": media, "language": t.language, "model": t.model, "verbatim": verbatim,
        "cached": was_cached, "wordCount": t.words.len(), "durationSeconds": t.duration_us as f64 / 1e6});
    match format {
        "text" => v["text"] = json!(t.text()),
        _ => {
            v["text"] = json!(t.text());
            v["words"] = json!(
                t.words
                    .iter()
                    .map(|w| json!({"id": timeline::word_id(&media, w.start_us), "text": w.text,
                "start": w.start_us as f64 / 1e6, "end": w.end_us as f64 / 1e6}))
                    .collect::<Vec<_>>()
            );
        }
    }
    let text =
        format!("{} words ({}{})\n{}", t.words.len(), t.model, if was_cached { ", cached" } else { "" }, t.text());
    Ok(Receipt::new(v, text))
}

#[cfg(feature = "speech")]
fn recognise(
    bundle: &Path,
    path: &Path,
    cid: Option<&str>,
    model: ove_transcribe::Model,
    language: Option<String>,
    verbatim: bool,
) -> Result<Transcript, Fail> {
    let store = crate::tools::models::open_store()?;
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let files = match store.ensure_installed(model.store_id(), ove_models::Origin::Agent, &mut |_| {}, &cancel)? {
        ove_models::Ensure::Ready(p) => p,
        ove_models::Ensure::NeedsConsent(n) => {
            return Err(
                Fail::new("model.consent", n.message.clone()).details(serde_json::to_value(&n).unwrap_or(Value::Null))
            );
        }
    };
    let model_path = files.first().ok_or_else(|| Fail::new("model.unknown", "the speech model has no file"))?;
    let _lease = store.lease(model.store_id());
    let opts = ove_transcribe::Options {
        language: language.clone(),
        model_name: model.id().into(),
        verbatim,
        ..Default::default()
    };
    let t = ove_transcribe::transcribe_file(model_path, path, &opts, |_| {}, std::sync::Arc::new(cancel))
        .map_err(|e| Fail::new(e.code(), e.to_string()))?;
    let key = match cid {
        Some(c) => ove_transcribe::cache::transcript_key(c, model.id(), language.as_deref(), verbatim),
        None => {
            let k = ove_transcribe::cache::key(path, model.id(), language.as_deref());
            if verbatim { format!("{k}-verbatim") } else { k }
        }
    };
    ove_transcribe::cache::store(bundle, &key, &t);
    Ok(t)
}

#[cfg(not(feature = "speech"))]
fn recognise(
    _: &Path,
    _: &Path,
    _: Option<&str>,
    _: ove_transcribe::Model,
    _: Option<String>,
    _: bool,
) -> Result<Transcript, Fail> {
    Err(Fail::new("speech.unavailable", "this build of ove cannot recognise speech; build it with `--features speech`"))
}
