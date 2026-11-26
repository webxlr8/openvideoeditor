//! The built-in assistant's tools for seeing the open project (add-local-agents-and-onboarding 3.3a): composited
//! timeline frames, a media item's frames, its sound levels, silences and loudness. The same things the pinned MCP
//! server gives a local agent; media is named within the project only, never by a path.

use std::path::{Path, PathBuf};

use ove_agent::tools::{MAX_IMAGE, SeeRequest, ToolOut};
use ove_core::Document;
use serde_json::json;

/// The file behind a media item of the project, named by id or file name (case-insensitive).
fn media_file(doc: &Document, bundle: &Path, q: &str) -> Result<PathBuf, String> {
    let id = if doc.project.media.contains_key(q) {
        q.to_string()
    } else {
        let ql = q.to_lowercase();
        let named: Vec<&String> = doc
            .project
            .media
            .values()
            .filter(|m| m.file_name().is_some_and(|n| n.to_lowercase() == ql))
            .map(|m| &m.id)
            .collect();
        match named.as_slice() {
            [one] => (*one).clone(),
            [] => return Err(format!("No media \"{q}\" in this project (see list_media).")),
            _ => return Err(format!("{} media items are named {q}; use the id.", named.len())),
        }
    };
    let entry = doc.registry.get(&id).ok_or_else(|| format!("Media {id} has no file."))?;
    let path = ove_store::media::resolve_media_path(bundle, &entry.native_path);
    if !path.is_file() {
        return Err(format!("The file of media {id} is offline."));
    }
    Ok(path)
}

fn png(img: &ove_analyze::Image) -> Result<Vec<u8>, String> {
    img.fit_within(MAX_IMAGE).png().map_err(|e| e.to_string())
}

fn err(e: ove_analyze::Error) -> String {
    e.to_string()
}

/// Answers one seeing request over the project `doc` in `bundle`.
pub fn see(doc: &Document, bundle: &Path, req: &SeeRequest) -> Result<ToolOut, String> {
    match req {
        SeeRequest::Look { times, count, width, stats } => {
            let times = if times.is_empty() { ove_analyze::timeline_times(doc, *count, None) } else { times.clone() };
            if times.is_empty() {
                return Err("The timeline is empty: nothing to look at yet.".into());
            }
            let imgs = ove_analyze::look(doc, Some(bundle), &times, (*width).min(MAX_IMAGE)).map_err(err)?;
            let frames: Vec<serde_json::Value> = times
                .iter()
                .zip(&imgs)
                .map(|(t, i)| {
                    let mut f = json!({"at": t, "width": i.width, "height": i.height});
                    if *stats {
                        f["stats"] = json!(ove_analyze::frame_stats(i));
                    }
                    f
                })
                .collect();
            let images = imgs.iter().map(png).collect::<Result<_, _>>()?;
            Ok(ToolOut { text: json!({"frames": frames}).to_string(), images })
        }
        SeeRequest::Grab { media, at, width } => {
            let file = media_file(doc, bundle, media)?;
            let img = ove_analyze::grab(&file, *at, Some((*width).min(MAX_IMAGE))).map_err(err)?;
            let text = json!({"media": media, "at": at, "width": img.width, "height": img.height, "stats": ove_analyze::frame_stats(&img)});
            Ok(ToolOut { text: text.to_string(), images: vec![png(&img)?] })
        }
        SeeRequest::Filmstrip { media, count } => {
            let file = media_file(doc, bundle, media)?;
            let opts = ove_analyze::FilmstripOpts { count: *count, ..Default::default() };
            let (img, times) = ove_analyze::filmstrip(&file, &opts).map_err(err)?;
            let text = json!({"media": media, "times": times, "width": img.width, "height": img.height});
            Ok(ToolOut { text: text.to_string(), images: vec![png(&img)?] })
        }
        SeeRequest::Levels { media, points } => {
            let file = media_file(doc, bundle, media)?;
            let l = ove_analyze::levels(&file, *points as usize).map_err(err)?;
            Ok(ToolOut { text: json!(l).to_string(), images: vec![] })
        }
        SeeRequest::Silence { media, threshold_db, min_seconds } => {
            let file = media_file(doc, bundle, media)?;
            let ranges = ove_analyze::silence(&file, *threshold_db, *min_seconds).map_err(err)?;
            Ok(ToolOut { text: json!({"media": media, "silences": ranges}).to_string(), images: vec![] })
        }
        SeeRequest::Loudness { media } => {
            let file = media_file(doc, bundle, media)?;
            let l = ove_analyze::loudness(&file).map_err(err)?;
            Ok(ToolOut { text: json!(l).to_string(), images: vec![] })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ove_core::ops::Op;
    use ove_core::revision::Source;
    use std::sync::Arc;

    fn project() -> (tempfile::TempDir, ove_store::Project) {
        let d = tempfile::tempdir().unwrap();
        let lib = ove_store::Library::open(d.path(), Arc::new(ove_core::SystemClock)).unwrap();
        let mut p = lib.create("See").unwrap();
        p.apply(
            vec![
                Op::AddTrack { track: json!({"id": "t", "name": "Text", "order": 0, "type": "video"}) },
                Op::AddItem {
                    item: json!({"id": "x", "trackId": "t", "type": "text", "startTime": 0, "endTime": 3, "textContent": "Hello", "name": "Title"}),
                },
            ],
            Source::Gui,
            None,
        )
        .unwrap();
        (d, p)
    }

    #[test]
    fn look_returns_capped_pictures_with_stats() {
        let (_d, p) = project();
        let out = see(p.doc(), p.bundle(), &SeeRequest::Look { times: vec![1.0], count: 6, width: 4000, stats: true })
            .unwrap();
        assert_eq!(out.images.len(), 1);
        let img = ove_analyze::Image::from_png(&out.images[0]).unwrap();
        assert!(img.width.max(img.height) <= MAX_IMAGE, "{}x{}", img.width, img.height);
        let v: serde_json::Value = serde_json::from_str(&out.text).unwrap();
        assert!(v["frames"][0]["stats"].is_object(), "{v}");
    }

    #[test]
    fn media_is_named_within_the_project_only() {
        let (d, p) = project();
        let outside = d.path().join("secret.wav");
        std::fs::write(&outside, b"RIFF").unwrap();
        for q in [outside.display().to_string(), "../secret.wav".into(), "nothing".into()] {
            let e = see(p.doc(), p.bundle(), &SeeRequest::Loudness { media: q.clone() }).unwrap_err();
            assert!(e.contains("No media"), "{q}: {e}");
        }
    }
}
