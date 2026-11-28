//! Index files keyed by content identity (add-semantic-media-search task 2.2): a relinked file keeps its index, a
//! damaged file is discarded and rebuilt, and scene detection answers from the cut list (task 2.3). Runs with a
//! stand-in encoder; needs the ffmpeg CLI for the fixture.
#![cfg(feature = "media")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::Path;
use std::process::Command;

use ove_vision::index::{DIM, Index, Provenance, index_path, normalise};
use ove_vision::indexer::{MediaKind, cached_scenes, ensure_index};
use ove_vision::model::Encoder;

fn ffmpeg() -> Option<String> {
    let bin = std::env::var("OVE_FIXTURE_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    Command::new(&bin).arg("-version").output().ok().filter(|o| o.status.success()).map(|_| bin)
}

macro_rules! need_ffmpeg {
    () => {
        if ffmpeg().is_none() {
            eprintln!("skipped: no ffmpeg CLI to make fixtures (set OVE_FIXTURE_FFMPEG)");
            return;
        }
    };
}

/// An encoder whose embedding is the mean of each channel and a few coarse pixel statistics: enough to tell the
/// fixture's shots apart, with no model.
#[derive(Default)]
struct Means {
    calls: usize,
}

impl Encoder for Means {
    fn images(&mut self, pixels: &[Vec<f32>]) -> ove_vision::Result<Vec<Vec<f32>>> {
        self.calls += 1;
        Ok(pixels
            .iter()
            .map(|p| {
                let plane = p.len() / 3;
                let mut v = vec![0f32; DIM];
                for c in 0..3 {
                    for k in 0..8 {
                        let part = &p[c * plane + k * plane / 8..c * plane + (k + 1) * plane / 8];
                        v[c * 8 + k] = part.iter().sum::<f32>() / part.len() as f32 + 1.0;
                    }
                }
                normalise(&mut v);
                v
            })
            .collect())
    }

    fn text(&mut self, _text: &str) -> ove_vision::Result<Vec<f32>> {
        Ok(vec![0.0; DIM])
    }

    fn provenance(&self) -> Provenance {
        Provenance {
            model: "means".into(),
            model_hash: "0".repeat(64),
            sampler: ove_vision::SAMPLER_VERSION,
            preprocessing: "test".into(),
        }
    }
}

fn clip(dir: &Path) -> std::path::PathBuf {
    let out = dir.join("two.mp4");
    let st = Command::new(ffmpeg().unwrap())
        .args(["-loglevel", "error", "-y", "-f", "lavfi", "-t", "2", "-i", "testsrc2=s=320x180:r=25"])
        .args(["-f", "lavfi", "-t", "9", "-i", "mandelbrot=s=320x180:r=25"])
        .args(["-filter_complex", "[0:v][1:v]concat=n=2:v=1:a=0", "-pix_fmt", "yuv420p", "-c:v", "libx264"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success());
    out
}

#[test]
fn a_relinked_file_keeps_its_index_and_a_damaged_one_is_rebuilt() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    let bundle = d.path().join("p.ove");
    let source = clip(d.path());
    let cid = ove_store_content_id(&source);
    let mut enc = Means::default();
    let built = ensure_index(&bundle, &source, &cid, MediaKind::Video, &mut enc, &mut |_| true).unwrap();
    assert!(!built.reused);
    // 2 s shot: 1 sample; 9 s shot: 2 samples.
    assert_eq!((built.shots, built.rows), (2, 3));
    let i = Index::open(&index_path(&bundle, &cid)).unwrap();
    assert_eq!(i.shots.len(), 2);
    assert!((i.cuts[0] - 2.0).abs() <= 0.08);
    // An open index holds a memory map; Windows refuses to replace a mapped file, so readers drop it first.
    drop(i);

    // The file moves (a relink): same content, same id, the index is reused without decoding.
    let moved_dir = d.path().join("elsewhere");
    std::fs::create_dir_all(&moved_dir).unwrap();
    let moved = moved_dir.join("renamed.mp4");
    std::fs::rename(&source, &moved).unwrap();
    assert_eq!(ove_store_content_id(&moved), cid);
    let calls = enc.calls;
    let again = ensure_index(&bundle, &moved, &cid, MediaKind::Video, &mut enc, &mut |_| true).unwrap();
    assert!(again.reused);
    assert_eq!(enc.calls, calls, "nothing was embedded again");

    // A damaged file is discarded and rebuilt.
    let path = index_path(&bundle, &cid);
    let mut bytes = std::fs::read(&path).unwrap();
    let n = bytes.len();
    bytes[n - 5] ^= 0x5a;
    std::fs::write(&path, &bytes).unwrap();
    let rebuilt = ensure_index(&bundle, &moved, &cid, MediaKind::Video, &mut enc, &mut |_| true).unwrap();
    assert!(!rebuilt.reused && rebuilt.replaced_damaged);
    assert_eq!(Index::open(&path).unwrap().rows(), 3);

    // Scene detection answers from the cut list, the same as decoding again.
    let cached = cached_scenes(&bundle, &cid, 0.3).unwrap();
    assert_eq!(cached, ove_analyze::scenes(&moved, 0.3).unwrap());
    assert!(cached_scenes(&bundle, &cid, 0.5).is_none(), "other thresholds decode");
}

#[test]
fn cancelling_writes_nothing_and_images_get_one_row() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    let bundle = d.path().join("p.ove");
    let source = clip(d.path());
    let mut enc = Means::default();
    let e = ensure_index(&bundle, &source, "c1", MediaKind::Video, &mut enc, &mut |f| f < 0.5).unwrap_err();
    assert_eq!(e.code(), "cancelled");
    assert!(!index_path(&bundle, "c1").exists());

    let png = d.path().join("still.png");
    let st = Command::new(ffmpeg().unwrap())
        .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=s=640x360", "-frames:v", "1"])
        .arg(&png)
        .status()
        .unwrap();
    assert!(st.success());
    let b = ensure_index(&bundle, &png, "c2", MediaKind::Image, &mut enc, &mut |_| true).unwrap();
    assert_eq!((b.rows, b.shots), (1, 1));
    assert!(cached_scenes(&bundle, "c2", 0.3).is_none(), "a still image has no scenes to answer");
}

/// The content id the media registry stores (same function as `ove_store::media::content_id`, read here through
/// the store crate's public API to keep the test honest about relinks).
fn ove_store_content_id(p: &Path) -> String {
    ove_store::media::content_id(p).unwrap()
}

/// Indexing runs as a P3 (analysis) task in the inference class; a second submit for the same content returns the
/// queued task, and the finished index is reused.
#[test]
fn indexing_is_a_background_analysis_task() {
    need_ffmpeg!();
    use ove_vision::task::{Job, KIND, submit};
    let d = tempfile::tempdir().unwrap();
    let bundle = d.path().join("p.ove");
    let source = clip(d.path());
    let cid = ove_store_content_id(&source);
    let scheduler = ove_tasks::Scheduler::new(ove_tasks::Options::default());
    let job = Job {
        project: "p".into(),
        bundle: bundle.clone(),
        source: source.clone(),
        content_id: cid.clone(),
        kind: MediaKind::Video,
        name: "two.mp4".into(),
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let tx2 = tx.clone();
    let a = submit(&scheduler, job.clone(), || Ok(Box::new(Means::default())), move |r| tx.send(r).unwrap());
    let b = submit(&scheduler, job.clone(), || Ok(Box::new(Means::default())), move |r| tx2.send(r).unwrap());
    assert_eq!(a, b, "one task per content");
    let info = scheduler.get(a).unwrap();
    assert_eq!(
        (info.kind.as_str(), info.priority, info.class),
        (KIND, ove_tasks::Priority::Analysis, ove_tasks::Class::Inference)
    );
    let done = scheduler.wait_task(a, std::time::Duration::from_secs(60)).unwrap();
    assert_eq!(done.state, ove_tasks::TaskState::Done, "{:?}", done.error);
    let built = rx.recv().unwrap().unwrap();
    assert_eq!(built.rows, 3);
    assert!(index_path(&bundle, &cid).is_file());

    // The model is missing: the task fails with "model not installed" and writes nothing.
    let job2 = Job { content_id: "other".into(), ..job };
    let (tx, rx) = std::sync::mpsc::channel();
    let id = submit(
        &scheduler,
        job2,
        || Err(ove_vision::VisionError::ModelNotInstalled { message: "not installed".into(), bytes: 1 }),
        move |r| tx.send(r).unwrap(),
    );
    let done = scheduler.wait_task(id, std::time::Duration::from_secs(60)).unwrap();
    assert_eq!(done.state, ove_tasks::TaskState::Failed);
    assert!(rx.recv().unwrap().unwrap_err().contains("not installed"));
    assert!(!index_path(&bundle, "other").exists());
}
