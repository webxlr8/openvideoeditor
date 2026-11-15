//! The sample project: a short clip made for the app (generated shapes and a synthesised voice, see
//! `sample/make-sample.sh`), opened as a project that already has a cut, a title, captions and a transition, so a
//! first look at the editor is never an empty timeline.
//!
//! The clip is part of the program (`include_bytes!`), not a resource file: packaging puts resources in a different
//! place on each platform, and a file inside the binary cannot be edited or go missing. "Try the sample" writes a
//! copy into a new project bundle, which owns it; asking again makes another project with its own copy.

use std::path::{Path, PathBuf};

use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_core::time::{Micros, micros_to_json};
use ove_store::Library;
use ove_store::project::Project;
use ove_transcribe::{Transcript, Word};
use serde_json::{Map, Value, json};
use tauri::AppHandle;

use crate::captions::{SourceClip, caption_ops, timeline_words};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::projects::{
    ProjectView, TEMPLATE_SUMMARY, default_tracks, make_thumbnail, media_records, open_bundle, serial, thumbs_dir,
    with_library,
};

/// The clip, as shipped.
const CLIP: &[u8] = include_bytes!("../sample/sample.mp4");
/// Where a project keeps its copy, relative to the bundle (forward slashes, as for generated media).
const CLIP_PATH: &str = "media/sample-clip.mp4";
const MEDIA_ID: &str = "m_sample";
/// Name of the project (the library adds a number to the bundle of a second copy, and so does this).
pub const NAME: &str = "Sample project";

// Times in the clip, in µs (they follow sample/make-sample.sh). The clip has two scenes with a spoken line each and
// a pause between them; the sample's cut takes the pause out.
/// The first timeline clip plays the source up to here.
const CUT_OUT: Micros = 2_800_000;
/// The second timeline clip starts here in the source (the second scene's first frame).
const CUT_IN: Micros = 3_400_000;
const CLIP_END: Micros = 7_000_000;
const DISSOLVE: Micros = 500_000;
const TITLE: &str = "My first video";
const TITLE_END: Micros = 2_600_000;

/// The words spoken in the clip and when (source time, µs), as an offline transcription would report them.
const WORDS: [(&str, Micros, Micros); 10] = [
    ("Welcome", 500_000, 1_020_000),
    ("to", 1_020_000, 1_200_000),
    ("OpenVideoEditor.", 1_200_000, 2_440_000),
    ("Cut", 3_655_000, 3_920_000),
    ("it,", 3_920_000, 4_145_000),
    ("add", 4_339_000, 4_620_000),
    ("a", 4_620_000, 4_720_000),
    ("title,", 4_720_000, 5_142_000),
    ("then", 5_349_000, 5_620_000),
    ("export.", 5_620_000, 6_239_000),
];

fn failed(what: &str, e: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::Io, format!("The sample project could not be made: {what}: {e}."))
}

/// The two timeline clips: (item id, timeline start, timeline end, start in the source).
fn clips() -> [(&'static str, Micros, Micros, Micros); 2] {
    let second = CUT_OUT + (CLIP_END - CUT_IN);
    [("i_sample_a", 0, CUT_OUT, 0), ("i_sample_b", CUT_OUT, second, CUT_IN)]
}

/// The clips, the title and the transition, on the starter tracks.
fn content_ops(media: Value, entry: Value) -> Vec<Op> {
    let mut ops = vec![Op::AddMedia { media, entry: Some(entry) }];
    for (n, (id, start, end, trim)) in clips().into_iter().enumerate() {
        let mut item = json!({
            "id": id, "trackId": "t_main", "type": "video", "name": "Sample clip", "mediaItemId": MEDIA_ID,
            "startTime": micros_to_json(start), "endTime": micros_to_json(end), "trimStart": micros_to_json(trim),
        });
        if n == 1 {
            // As the Transitions panel's "between" does: the transition sits on the incoming clip.
            item["transitionIn"] =
                json!({"type": "dissolve", "duration": micros_to_json(DISSOLVE), "easing": "ease-in-out"});
        }
        ops.push(Op::AddItem { item });
    }
    ops.push(Op::AddItem {
        item: json!({
            "id": "i_sample_title", "trackId": "t_text", "type": "text", "name": "Title",
            "startTime": micros_to_json(200_000), "endTime": micros_to_json(TITLE_END),
            "textContent": TITLE, "fontFamily": "Inter", "fontSize": 120, "fontWeight": 800, "color": "#ffffff",
            "textAlign": "center", "shadowColor": "#000000", "shadowBlur": 12,
            // Above the middle, clear of the captions at the bottom.
            "transform": {"y": -300},
        }),
    });
    ops
}

/// The clip's words in timeline time: what auto-captions would get from transcribing the two timeline clips.
fn spoken_words(path: &Path) -> Vec<Word> {
    let transcript = Transcript {
        language: "en".into(),
        model: "sample".into(),
        words: WORDS
            .iter()
            .map(|(text, start, end)| Word { text: (*text).into(), start_us: *start, end_us: *end, confidence: 1.0 })
            .collect(),
        duration_us: CLIP_END,
    };
    clips()
        .into_iter()
        .flat_map(|(id, start, end, trim)| {
            let clip = SourceClip {
                item_id: id.into(),
                media_id: MEDIA_ID.into(),
                path: path.to_path_buf(),
                start,
                end,
                trim,
                speed: 1.0,
                reverse: false,
            };
            timeline_words(&clip, &transcript)
        })
        .collect()
}

/// Fills a new, empty project: the starter tracks (as every new project), then the sample's content.
fn fill(p: &mut Project) -> AppResult<()> {
    let bundle = p.bundle().to_path_buf();
    let path = bundle.join(CLIP_PATH);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| failed("its media folder", e))?;
    }
    std::fs::write(&path, CLIP).map_err(|e| failed("its clip", e))?;
    let info = ove_media::probe(&path).map_err(|e| failed("its clip could not be read", e))?;
    let now = ove_core::Clock::now_ms(&ove_core::SystemClock);
    let (media, mut entry, duration_us) = media_records(&path, &info, MEDIA_ID, now);
    // The bundle owns its copy: a path inside the bundle, which moves with it.
    entry["nativePath"] = CLIP_PATH.into();
    entry["ownedByBundle"] = true.into();
    if make_thumbnail(&path, &thumbs_dir(&bundle).join(format!("{MEDIA_ID}.png")), duration_us).is_none() {
        tracing::warn!("no thumbnail for the sample clip");
    }

    let summary = format!("{TEMPLATE_SUMMARY}“{NAME}”");
    let mut tracks = default_tracks();
    // A second copy says so in its name, as its bundle folder does.
    if let Some(stem) = bundle.file_stem().map(|s| s.to_string_lossy().into_owned()).filter(|s| s != NAME) {
        let mut set = Map::new();
        set.insert("name".into(), stem.into());
        tracks.push(Op::UpdateProject { set, unset: vec![] });
    }
    p.apply(tracks, Source::Gui, Some(summary.clone()))?;
    // Captions through the auto-captions path, from the words instead of a transcription: the same items, on the
    // Captions track the starter tracks just made.
    let (captions, _) = caption_ops(p.doc(), &spoken_words(&path), None, false);
    let mut ops = content_ops(media, entry);
    ops.extend(captions);
    p.apply(ops, Source::Gui, Some(format!("{summary}: clips, title and captions")))?;
    Ok(())
}

/// Makes a new copy of the sample project in the library and returns its bundle. The project is closed again, so
/// opening it starts with an empty undo history: Undo cannot take the sample apart by accident.
pub fn create(lib: &Library) -> AppResult<PathBuf> {
    let mut p = lib.create(NAME)?;
    let bundle = p.bundle().to_path_buf();
    let made = match fill(&mut p) {
        Ok(()) => p.close().map_err(AppError::from),
        Err(e) => {
            // Dropping the project releases its lock, so the folder can be removed.
            drop(p);
            Err(e)
        }
    };
    match made {
        Ok(()) => {
            tracing::info!(bundle = %bundle.display(), "sample project created");
            Ok(bundle)
        }
        Err(e) => {
            // No half-made project stays in the library.
            if let Err(io) = std::fs::remove_dir_all(&bundle) {
                tracing::warn!(bundle = %bundle.display(), error = %io, "could not remove the unfinished sample");
            }
            Err(e)
        }
    }
}

/// "Try the sample": copies the sample project into the library and opens it in the editor.
#[tauri::command]
#[specta::specta]
pub async fn create_sample_project(app: AppHandle) -> AppResult<ProjectView> {
    serial(&app, "create_sample_project", |app, state| {
        let bundle = with_library(&state.projects, create)?;
        open_bundle(app, state, &bundle.display().to_string())
    })
    .await
}

#[cfg(test)]
mod tests {
    use ove_store::project::Mode;

    use super::*;
    use crate::projects::{clock, view};

    fn open(bundle: &Path) -> Project {
        Project::open(bundle, Mode::Write, clock()).unwrap()
    }

    #[test]
    fn the_sample_has_a_cut_a_title_captions_and_a_transition() {
        let d = tempfile::tempdir().unwrap();
        let lib = Library::open(d.path(), clock()).unwrap();
        let bundle = create(&lib).unwrap();
        let p = open(&bundle);
        ove_core::migrate::validate(p.doc()).unwrap();
        let v = view(&p);
        assert_eq!(v.name, NAME);
        assert!(!v.can_undo, "the sample opens with an empty undo history");

        // The clip is in the bundle, byte for byte, and reads as the video it is.
        let copy = bundle.join(CLIP_PATH);
        assert_eq!(std::fs::read(&copy).unwrap(), CLIP);
        assert_eq!(v.media.len(), 1);
        let m = &v.media[0];
        assert_eq!((m.kind.as_str(), m.status.as_str()), ("video", "online"));
        assert_eq!((m.width, m.height), (Some(960), Some(540)));
        assert!((m.duration_seconds.unwrap() - 7.0).abs() < 0.1, "{:?}", m.duration_seconds);
        assert!(p.doc().registry[MEDIA_ID].owned_by_bundle);
        // A frame of it decoded with the app's own FFmpeg build.
        assert!(m.thumbnail.as_deref().is_some_and(|t| t.len() > 200), "a thumbnail of the clip");

        // A cut: two clips of the same file that meet on the timeline with part of the source left out.
        let clips: Vec<_> = v.items.iter().filter(|i| i.track_id == "t_main").collect();
        assert_eq!(clips.len(), 2);
        assert!(clips.iter().all(|c| c.kind == "video" && c.media_id.as_deref() == Some(MEDIA_ID)));
        assert_eq!(clips[0].end, clips[1].start);
        assert!(clips[1].trim_start > clips[0].trim_start + (clips[0].end - clips[0].start));
        // A transition at the cut.
        assert_eq!(clips[1].transition_in, 0.5);
        let second = p.doc().item(&clips[1].id).unwrap().to_json();
        assert_eq!(second["transitionIn"]["type"], "dissolve");
        // The outgoing clip keeps playing under the incoming one: the source has that much left after its end.
        let handle = (CUT_IN - CUT_OUT) as f64 / 1e6;
        assert!(clips[1].transition_in <= handle, "the dissolve fits in the part that was cut out");

        // A title.
        let titles: Vec<_> = v.items.iter().filter(|i| i.kind == "text").collect();
        assert_eq!(titles.len(), 1);
        assert_eq!(titles[0].text.as_deref(), Some(TITLE));
        assert_eq!(titles[0].track_id, "t_text");

        // Captions: one per spoken line, on the Captions track, inside the timeline, with word timings.
        let captions: Vec<_> = v.items.iter().filter(|i| i.kind == "caption").collect();
        assert_eq!(captions.len(), 2, "{captions:?}");
        assert_eq!(captions[0].text.as_deref(), Some("Welcome to OpenVideoEditor."));
        assert_eq!(captions[1].text.as_deref(), Some("Cut it, add a title, then export."));
        assert!(captions.iter().all(|c| c.track_id == "t_captions" && c.end <= v.duration_seconds + 1e-6));
        assert!(captions[0].end <= captions[1].start);
        // The second line is said after the cut, so its caption moved earlier with the clip.
        let shift = (CUT_IN - CUT_OUT) as f64 / 1e6;
        assert!((captions[1].start - (3.655 - shift)).abs() < 1e-6, "{}", captions[1].start);
        let words = p.doc().item(&captions[1].id).unwrap().to_json()["captionSegments"][0]["wordTimings"].clone();
        assert_eq!(words.as_array().map(Vec::len), Some(7), "{words}");

        assert_eq!(v.tracks.len(), 5, "the starter tracks, nothing more");
        assert!((v.duration_seconds - 6.4).abs() < 1e-6);
        let who: Vec<String> = crate::projects::history_rows(&p, 10).unwrap().into_iter().map(|r| r.who).collect();
        assert_eq!(who, ["Template", "Template"]);
    }

    #[test]
    fn opening_the_sample_twice_makes_two_projects_and_the_first_keeps_its_edits() {
        let d = tempfile::tempdir().unwrap();
        let lib = Library::open(d.path(), clock()).unwrap();
        let first = create(&lib).unwrap();
        // An edit in the first copy: its title is removed.
        let mut p = open(&first);
        p.apply(vec![Op::RemoveItems { ids: vec!["i_sample_title".into()] }], Source::Gui, None).unwrap();
        p.close().unwrap();

        let second = create(&lib).unwrap();
        assert_ne!(first, second);
        assert_eq!(lib.list().unwrap().len(), 2);
        let (a, b) = (open(&first), open(&second));
        assert_ne!(a.doc().project.id, b.doc().project.id);
        assert_eq!(a.doc().project.name, NAME);
        assert_eq!(b.doc().project.name, format!("{NAME} 2"));
        assert!(a.doc().item("i_sample_title").is_none(), "the first copy keeps its edit");
        assert!(b.doc().item("i_sample_title").is_some(), "the second copy is the sample as shipped");
        // Each project has its own copy of the clip.
        assert!(first.join(CLIP_PATH).is_file() && second.join(CLIP_PATH).is_file());
        assert_eq!(view(&b).media[0].path, second.join(CLIP_PATH).display().to_string());
    }

    #[test]
    fn the_shipped_clip_is_small() {
        // A few hundred kilobytes: it is inside every copy of the app.
        assert!(CLIP.len() < 600 * 1024, "{} bytes", CLIP.len());
        // An MP4 (`ftyp` box first).
        assert_eq!(&CLIP[4..8], b"ftyp");
    }
}
