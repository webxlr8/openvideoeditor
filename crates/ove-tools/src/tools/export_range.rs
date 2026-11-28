//! Save a timeline range as media, and render in place (change extend-export-formats-and-queue, design D8).
//!
//! The range is rendered at the project's own frame size and rate into `media/rendered/` in the bundle (DNxHR HQ, or
//! ProRes 422 HQ where DNxHR is not available; on macOS ProRes 422 HQ; H.264 in MP4 with `small`), then added to the
//! library as media the project owns, in one revision. With `replace`, the same revision removes what the range
//! covers on the chosen tracks and places one clip of the new media there; one undo restores the items. Only the
//! chosen tracks are rendered then, so nothing shows twice; when other visual tracks have content in the range the
//! render keeps its transparency (ProRes 4444) so they still show through.
//!
//! The file exists before the revision is committed: a failed render leaves nothing, and a failed commit deletes the
//! file.

use crate::ctx::ToolCtx;
use crate::fail::Fail;
use crate::receipt::Receipt;
use crate::spec::Kind::{Bool, List, Num, Str};
use crate::spec::{Arg, Args, arg, req};

pub const SAVE_RANGE_ABOUT: &str = "Render a range of the timeline into a new media file owned by the project (one revision). With `replace`, the same revision replaces what the range covers on `tracks` (default: every track except audio tracks) with one clip of the new media: a render in place, undone in one step. The file is DNxHR HQ or ProRes 422 HQ in MOV at the project's size (`small`: H.264 MP4).";

pub const SAVE_RANGE_ARGS: &[Arg] = &[
    crate::registry::PROJECT,
    req("start", Num, "Range start in seconds"),
    req("end", Num, "Range end in seconds"),
    arg("replace", Bool, "Also replace the covered items with one clip of the new media"),
    arg("tracks", List, "Track ids or names to render and replace (default: all but audio tracks)"),
    arg("name", Str, "Name of the new media"),
    arg("small", Bool, "H.264 in MP4 instead of an intermediate codec (smaller, lossy)"),
];

#[cfg(feature = "media")]
mod imp {
    use std::path::{Path, PathBuf};

    use ove_core::model::Document;
    use ove_core::ops::{Op, apply_batch};
    use ove_core::time::{Micros, micros_to_json};
    use ove_render::export::{
        AudioCodec, AudioSettings, Container, ExportSettings, Fps, Inventory, Size, VideoCodec, VideoSettings,
    };
    use serde_json::{Value, json};

    use super::*;
    use crate::ctx::{edit, snapshot, via};
    use crate::fail::{now_ms, secs};

    /// The intermediate the range is written in on this computer, by preference.
    pub fn intermediate(alpha: bool, small: bool) -> ExportSettings {
        let video = |codec, profile: &str| {
            Some(VideoSettings { codec, profile: Some(profile.into()), alpha, ..VideoSettings::default() })
        };
        let pcm = Some(AudioSettings { codec: AudioCodec::Pcm, bit_depth: Some(24), ..AudioSettings::default() });
        let base = ExportSettings { size: Size::MatchTimeline, fps: Fps::MatchTimeline, ..ExportSettings::default() };
        if small && !alpha {
            return ExportSettings {
                container: Container::Mp4,
                video: Some(VideoSettings { quality: ove_render::export::Quality::High, ..VideoSettings::default() }),
                audio: Some(AudioSettings::default()),
                ..base
            };
        }
        let prores = ExportSettings {
            container: Container::Mov,
            video: video(VideoCodec::Prores, if alpha { "4444" } else { "hq" }),
            audio: pcm.clone(),
            ..base.clone()
        };
        if alpha || cfg!(target_os = "macos") || !Inventory::current().has_encoder("dnxhd") {
            return prores;
        }
        ExportSettings { container: Container::Mov, video: video(VideoCodec::Dnxhr, "hq"), audio: pcm, ..base }
    }

    /// The tracks a call names (ids or names), or every track that is not an audio track.
    pub fn chosen_tracks(doc: &Document, names: &[String]) -> Result<Vec<String>, Fail> {
        if names.is_empty() {
            return Ok(doc.project.tracks.iter().filter(|t| t.kind != "audio").map(|t| t.id.clone()).collect());
        }
        names
            .iter()
            .map(|n| {
                doc.project
                    .tracks
                    .iter()
                    .find(|t| t.id == *n || t.name.eq_ignore_ascii_case(n))
                    .map(|t| t.id.clone())
                    .ok_or_else(|| Fail::new("track.unknown", format!("there is no track {n:?}")))
            })
            .collect()
    }

    /// The document with only the items of `tracks` (what render in place renders).
    fn only_tracks(doc: &Document, tracks: &[String]) -> Result<Document, Fail> {
        let others: Vec<String> =
            doc.project.items.values().filter(|i| !tracks.contains(&i.track_id)).map(|i| i.id.clone()).collect();
        if others.is_empty() {
            return Ok(doc.clone());
        }
        apply_batch(doc, &[Op::RemoveItems { ids: others }])
            .map(|c| c.doc)
            .map_err(|e| Fail::new("internal", format!("could not isolate the tracks: {e}")))
    }

    /// True when a visual track outside `tracks` has content in the range.
    fn others_show(doc: &Document, tracks: &[String], a: Micros, b: Micros) -> bool {
        doc.project.items.values().any(|i| {
            !tracks.contains(&i.track_id)
                && i.end > a
                && i.start < b
                && doc.track(&i.track_id).is_some_and(|t| t.kind != "audio")
        })
    }

    /// A file name for the new media: the given name or "Range <a>–<b> s", made safe and unique in `dir`.
    fn file_for(dir: &Path, name: &str, ext: &str) -> PathBuf {
        let safe: String = name
            .chars()
            .map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') { c } else { '_' })
            .collect::<String>()
            .trim()
            .trim_matches('.')
            .chars()
            .take(80)
            .collect();
        let safe = if safe.is_empty() { "Range".to_string() } else { safe };
        let mut p = dir.join(format!("{safe}.{ext}"));
        let mut n = 2;
        while p.exists() || ove_render::export::partial_path(&p).exists() {
            p = dir.join(format!("{safe} {n}.{ext}"));
            n += 1;
        }
        p
    }

    fn label(us: Micros) -> String {
        let s = us as f64 / 1e6;
        if (s - s.round()).abs() < 1e-6 { format!("{s:.0}") } else { format!("{s:.2}") }
    }

    pub fn save_range_as_media(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
        let start = secs(a.f64("start").unwrap_or(-1.0), "start")?;
        let end = secs(a.f64("end").unwrap_or(-1.0), "end")?;
        let replace = a.bool("replace");
        let bundle = ctx.bundle(a.project())?;
        let (doc, bundle, rev) = snapshot(ctx, &bundle)?;
        let tracks = chosen_tracks(&doc, &a.strings("tracks"))?;
        let alpha = replace && others_show(&doc, &tracks, start, end);
        let mut settings = intermediate(alpha, a.bool("small"));
        settings.range = Some([start as f64 / 1e6, end as f64 / 1e6]);
        let render_doc = if replace { only_tracks(&doc, &tracks)? } else { doc.clone() };
        let name =
            a.str("name").map(str::to_string).unwrap_or_else(|| format!("Range {}–{} s", label(start), label(end)));
        let dir = bundle.join("media").join("rendered");
        std::fs::create_dir_all(&dir)
            .map_err(|e| Fail::new("file.unwritable", format!("could not create {}: {e}", dir.display())))?;
        let out = file_for(&dir, &name, settings.container.extension());
        // Refusals (bad range, offline media, pending placeholders) come before anything is written. The rendered
        // document keeps the whole project's settings, so check the real one too.
        ove_render::export::prepare(&doc, Some(&bundle), &settings).map_err(super::super::export::export_fail)?;
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let mut last = -1i64;
        let res = ove_render::export(&render_doc, Some(&bundle), &out, &settings, &cancel, |pr| {
            let pct = (pr.fraction * 100.0) as i64;
            if pct != last {
                last = pct;
                ctx.progress(pct, &serde_json::to_value(&pr).unwrap_or_default());
            }
        })
        .map_err(super::super::export::export_fail)?;
        ctx.progress_done();
        let rel = format!("media/rendered/{}", out.file_name().map(|n| n.to_string_lossy()).unwrap_or_default());
        let media_id = format!("m_r{:x}", now_ms());
        let len = end - start;
        // The file's identity (content id, size, time) goes in with it, as for an import: no later "media
        // identities" revision lands on top of this one, so one undo still undoes the render.
        let mut entry = json!({"mediaId": media_id, "nativePath": rel, "ownedByBundle": true, "probeStatus": "probed"});
        if let Some(e) = entry.as_object_mut() {
            e.extend(ove_store::media::identity_fields(&out));
        }
        let committed = edit(ctx, &bundle, |w| {
            let current = w.project().doc();
            if current.project.rev != rev {
                return Err(Fail::new(
                    "rev.changed",
                    "the project changed while the range was rendering; save the range again",
                ));
            }
            let now = now_ms();
            let file_name = out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let add = Op::AddMedia {
                media: json!({"id": media_id, "createdAt": now, "addedAt": now, "data": {
                    "type": "video", "fileName": file_name, "duration": micros_to_json(len),
                    "width": res.width, "height": res.height, "hasAudio": res.audio_encoder.is_some(),
                    "rendered": {"start": micros_to_json(start), "end": micros_to_json(end), "rev": rev}}}),
                entry: Some(entry.clone()),
            };
            let mut ops = vec![add.clone()];
            if replace {
                let with_media = apply_batch(current, &[add])
                    .map_err(|e| Fail::new("internal", format!("could not add the media: {e}")))?
                    .doc;
                let lift = ove_edit::plan(
                    &with_media,
                    &ove_edit::Request::Lift { tracks: tracks.clone(), range: ove_edit::Range { start, end } },
                )
                .map_err(|r| Fail::new(r.code.clone(), r.message.clone()))?;
                let top = tracks
                    .iter()
                    .filter_map(|t| current.track(t))
                    .min_by_key(|t| t.order)
                    .map(|t| t.id.clone())
                    .ok_or_else(|| Fail::new("track.unknown", "no track to place the rendered clip on"))?;
                let item = json!({"type": "video", "trackId": top, "startTime": 0, "endTime": micros_to_json(len),
                    "mediaItemId": media_id, "name": name});
                let place = ove_edit::plan(
                    &lift.result,
                    &ove_edit::Request::Overwrite { track: top.clone(), at: start, items: vec![item] },
                )
                .map_err(|r| Fail::new(r.code.clone(), r.message.clone()))?;
                ops.extend(lift.ops);
                ops.extend(place.ops);
            }
            let summary = if replace {
                format!("render {}–{} s in place", label(start), label(end))
            } else {
                format!("save {}–{} s as media", label(start), label(end))
            };
            let new_rev = w.apply(ops, summary)?;
            // The clip of the new media, as committed.
            let item_id = w
                .project()
                .doc()
                .project
                .items
                .values()
                .find(|i| i.media_id() == Some(media_id.as_str()))
                .map(|i| i.id.clone());
            Ok((new_rev, item_id, via(w)))
        });
        let (new_rev, item_id, via_text) = match committed {
            Ok(c) => c,
            Err(e) => {
                // The media never made it into the project: its file goes too.
                let _ = std::fs::remove_file(&out);
                return Err(e);
            }
        };
        let v: Value = json!({
            "rev": new_rev, "mediaId": media_id, "path": out, "file": rel, "itemId": item_id,
            "replaced": replace, "tracks": tracks, "start": micros_to_json(start), "end": micros_to_json(end),
            "videoFormat": res.video_format, "encoderLabel": res.encoder_label, "alpha": alpha,
        });
        let text = format!(
            "rev {new_rev}: {} {}–{} s as {} ({}){}{}",
            if replace { "rendered in place" } else { "saved" },
            label(start),
            label(end),
            rel,
            res.video_format.as_deref().unwrap_or(""),
            item_id.map(|i| format!("; clip {i}")).unwrap_or_default(),
            via_text
        );
        Ok(Receipt::new(v, text))
    }
}

#[cfg(feature = "media")]
pub use imp::{chosen_tracks, intermediate, save_range_as_media};

#[cfg(not(feature = "media"))]
pub fn save_range_as_media(_: &mut dyn ToolCtx, _: &Args) -> Result<Receipt, Fail> {
    Err(Fail::new("export.unavailable", "this build of ove has no export; build it with `--features media`"))
}
