//! `get_transcript` (add-transcript-editing D1, D2): the timeline transcript of a project revision from the cached
//! transcripts of its media, with word ids anchored to the source. Read-only: media without a transcript are listed
//! as untranscribed (`transcribe_media` makes them).

use std::collections::HashMap;
use std::path::Path;

use ove_core::Document;
use ove_transcribe::Transcript;
use ove_transcribe::timeline::{self, NoClips};
use serde_json::{Value, json};

use crate::ctx::ToolCtx;
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

/// The timeline transcript of a project (all tracks, or the given clips).
pub fn transcript_of(
    bundle: &Path,
    doc: &Document,
    rev: i64,
    a: &Args,
) -> Result<(ove_transcribe::timeline::TimelineTranscript, Vec<timeline::SourceClip>), Fail> {
    let ids: Vec<String> = a.strings("clips");
    let resolve =
        |m: &str| doc.registry.get(m).map(|e| ove_store::media::resolve_media_path(bundle, e.native_path.as_str()));
    let clips = if ids.is_empty() {
        timeline::all_clips(doc, &resolve)
    } else {
        timeline::source_clips(doc, &ids, &resolve).map_err(|e| match e {
            NoClips::Selection => Fail::new("transcript.noClips", e.to_string()),
            NoClips::Timeline => Fail::new("transcript.noClips", e.to_string()),
        })?
    };
    let transcripts = cached(bundle, doc, &clips, a.str("model"), a.str("language"));
    let (words, untranscribed) = timeline::words(&clips, &transcripts);
    let page = timeline::page(
        rev,
        words,
        untranscribed,
        a.i64("offset").unwrap_or(0).max(0) as usize,
        a.i64("limit").unwrap_or(0).max(0) as usize,
    );
    Ok((page, clips))
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
