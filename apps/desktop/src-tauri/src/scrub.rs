//! Scrub audio (design D6): while the person drags the play head, each scrub event plays a 60 ms grain of the real
//! timeline mix at that time. A worker thread owns its own mixer, the revision-keyed window cache and its own audio
//! output (so the playback clock is not disturbed). Only the latest event is played: a new grain replaces the one
//! still queued. After each grain the neighbouring window in the scrub direction is mixed ahead.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use ove_core::model::Document;
use ove_playback::scrub::Scrubber;
use ove_playback::{AudioOutput, SystemTime, TimeSource};
use ove_render::audio::Mixer;
use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult, ErrorCode};

/// One scrub event: play the grain at `t_us` of this revision of the project.
struct Event {
    doc: Arc<Document>,
    bundle: PathBuf,
    rev: i64,
    t_us: i64,
    at: Instant,
}

struct Worker {
    tx: Sender<Event>,
    /// The document last sent, so 60 events a second do not copy it each time.
    last: Mutex<Option<(PathBuf, i64, Arc<Document>)>>,
}

static WORKER: OnceLock<Worker> = OnceLock::new();

fn worker() -> &'static Worker {
    WORKER.get_or_init(|| {
        let (tx, rx) = channel();
        let _ = std::thread::Builder::new().name("ove-scrub".into()).spawn(move || run(rx));
        Worker { tx, last: Mutex::new(None) }
    })
}

/// The mix producer for one project: a mixer over the latest document of each revision.
pub struct DocMix {
    mixer: Mixer,
    docs: Vec<(i64, Arc<Document>)>,
}

impl DocMix {
    pub fn new(bundle: &std::path::Path) -> Self {
        DocMix { mixer: Mixer::new(Some(bundle)), docs: Vec::new() }
    }

    pub fn set(&mut self, rev: i64, doc: Arc<Document>) {
        if !self.docs.iter().any(|(r, _)| *r == rev) {
            self.docs.push((rev, doc));
            if self.docs.len() > 4 {
                self.docs.remove(0);
            }
        }
    }
}

impl ove_playback::scrub::MixSource for DocMix {
    fn mix(&mut self, rev: i64, start_us: i64, frames: usize) -> Vec<f32> {
        match self.docs.iter().find(|(r, _)| *r == rev) {
            Some((_, d)) => {
                let d = d.clone();
                self.mixer.mix(&d, start_us, frames)
            }
            None => vec![0.0; frames * 2],
        }
    }
}

fn run(rx: Receiver<Event>) {
    let time: Arc<dyn TimeSource> = Arc::new(SystemTime::new());
    let mut out: Option<AudioOutput> = None;
    let mut state: Option<(PathBuf, Scrubber<DocMix>)> = None;
    while let Ok(mut ev) = rx.recv() {
        // Only the latest event matters.
        while let Ok(next) = rx.try_recv() {
            ev = next;
        }
        if state.as_ref().is_none_or(|(b, _)| *b != ev.bundle) {
            state = Some((ev.bundle.clone(), Scrubber::new(DocMix::new(&ev.bundle))));
        }
        let Some((_, scrubber)) = state.as_mut() else { continue };
        if out.is_none() {
            out = AudioOutput::open_default(time.clone()).ok();
            if let Some(o) = &out {
                o.ring().set_paused(false);
            }
        }
        let Some(o) = out.as_ref() else { continue };
        scrubber_source(scrubber).set(ev.rev, ev.doc.clone());
        let direction = scrubber.direction(ev.t_us);
        let (grain, cached) = scrubber.grain(ev.rev, ev.t_us);
        let (rate, channels) = o.ring().format();
        o.ring().splice_grain(ev.t_us, &crate::engine::to_device(&grain, rate, channels));
        tracing::trace!(latency_ms = ev.at.elapsed().as_secs_f64() * 1e3, cached, "scrub grain");
        // Mix ahead in the scrub direction, unless a newer event is already waiting.
        scrubber.prefetch(ev.rev, ev.t_us, direction);
    }
}

fn scrubber_source(s: &mut Scrubber<DocMix>) -> &mut DocMix {
    s.source_mut()
}

/// Plays the grain of the open project's timeline mix at `t` seconds. Called up to 60 times a second while the play
/// head is dragged; it returns at once (the worker plays only the latest).
#[tauri::command]
#[specta::specta]
pub async fn scrub_audio(app: AppHandle, t: f64) -> AppResult<()> {
    crate::worker::blocking("scrub_audio", move || {
        if !t.is_finite() || t < 0.0 {
            return Err(AppError::new(ErrorCode::InvalidArgument, "A scrub time must be a time in the timeline."));
        }
        let state = app.state::<crate::AppState>();
        let w = worker();
        let mut last = w.last.lock().map_err(|_| AppError::new(ErrorCode::Internal, "Scrub is busy."))?;
        let (bundle, rev) = {
            let g = crate::projects::lock(&state.projects.open)?;
            let Some(p) = g.as_ref() else { return Ok(()) };
            let rev = p.doc().project.rev;
            let bundle = p.bundle().to_path_buf();
            if last.as_ref().is_none_or(|(b, r, _)| *b != bundle || *r != rev) {
                *last = Some((bundle.clone(), rev, Arc::new(p.doc().clone())));
            }
            (bundle, rev)
        };
        let doc = last
            .as_ref()
            .map(|(_, _, d)| d.clone())
            .ok_or_else(|| AppError::new(ErrorCode::Internal, "No project."))?;
        let _ = w.tx.send(Event { doc, bundle, rev, t_us: (t * 1e6) as i64, at: Instant::now() });
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use ove_core::ops::{Op, apply_batch};
    use serde_json::json;

    use super::*;

    fn p95(mut v: Vec<f64>) -> f64 {
        v.sort_by(f64::total_cmp);
        v[((v.len() as f64 * 0.95).ceil() as usize).saturating_sub(1).min(v.len() - 1)]
    }

    /// Event-to-grain latency with the real mixer on a 20 s two-track timeline (no device: the grain is ready).
    #[test]
    fn scrub_latency_and_edits() {
        let dir = tempfile::tempdir().unwrap();
        let a = crate::waveforms::testing::wav(dir.path(), "a.wav", 440.0, 0.5, 20.0);
        let b = crate::waveforms::testing::wav(dir.path(), "b.wav", 220.0, 0.3, 20.0);
        let mut ops = vec![];
        for (i, path) in [a, b].iter().enumerate() {
            ops.push(Op::AddTrack { track: json!({"id": format!("t{i}"), "name": "A", "order": i, "type": "audio"}) });
            ops.push(Op::AddMedia {
                media: json!({"id": format!("m{i}"), "data": {"type": "audio", "fileName": "a.wav", "duration": 20}}),
                entry: Some(json!({"mediaId": format!("m{i}"), "nativePath": path.display().to_string(), "ownedByBundle": false})),
            });
            ops.push(Op::AddItem {
                item: json!({"id": format!("i{i}"), "trackId": format!("t{i}"), "type": "audio", "startTime": 0, "endTime": 20, "mediaItemId": format!("m{i}")}),
            });
        }
        let doc = Arc::new(apply_batch(&Document::new("p", "Scrub", 1), &ops).unwrap().doc);
        let mut s = Scrubber::new(DocMix::new(dir.path()));
        s.source_mut().set(1, doc.clone());
        let (mut cold, mut warm) = (vec![], vec![]);
        // A slow drag forward over 12 s: 60 events a second, 10 ms of timeline each.
        let mut t = 1_000_000i64;
        while t < 13_000_000 {
            let dir = s.direction(t);
            let start = Instant::now();
            let (_, cached) = s.grain(1, t);
            let ms = start.elapsed().as_secs_f64() * 1e3;
            if cached {
                warm.push(ms)
            } else {
                cold.push(ms)
            }
            s.prefetch(1, t, dir);
            t += 10_000;
        }
        // Jumps to areas not played before: cold every time.
        let mut s2 = Scrubber::new(DocMix::new(dir.path()));
        s2.source_mut().set(1, doc.clone());
        for k in 0..19 {
            let start = Instant::now();
            let (_, cached) = s2.grain(1, k * 1_000_000 + 500_000);
            assert!(!cached);
            cold.push(start.elapsed().as_secs_f64() * 1e3);
        }
        let (pc, pw) = (p95(cold.clone()), p95(warm.clone()));
        eprintln!(
            "scrub event to grain: cached p95 {pw:.2} ms ({} events), cold p95 {pc:.2} ms ({} events)",
            warm.len(),
            cold.len()
        );
        assert!(pw <= 40.0, "cached p95 {pw} ms");
        assert!(pc <= 120.0, "cold p95 {pc} ms");
        // An edit (mute track 0, new revision) changes the next grain.
        let (before, _) = s.grain(1, 5_000_000);
        let muted = Arc::new(
            apply_batch(
                &doc,
                &[Op::UpdateTrack { id: "t0".into(), set: json!({"muted": true}).as_object().unwrap().clone() }],
            )
            .unwrap()
            .doc,
        );
        s.source_mut().set(2, muted);
        let (after, cached) = s.grain(2, 5_000_000);
        assert!(!cached);
        let peak = |v: &[f32]| v.iter().fold(0f32, |m, x| m.max(x.abs()));
        assert!(peak(&before) > 0.6 && peak(&after) < 0.32, "{} then {}", peak(&before), peak(&after));
    }
}
