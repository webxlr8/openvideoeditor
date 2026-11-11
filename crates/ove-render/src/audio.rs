//! Audio mixdown of the timeline: every video/audio item with sound, volume (0–200 %), item and track mute, audio
//! fades and `audio-fade` transitions, playback speed (nearest-sample resampling; pitch changes with speed), to
//! interleaved stereo f32 at 48 kHz. Reverse playback is silent in v1.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ove_core::model::{Document, Item};
use ove_core::time::{Micros, json_to_micros};
use ove_media::{AudioDecoder, AudioOutputSpec, SampleFormat, Samples};
use serde_json::Value;

use crate::eval::source_time;
use crate::sources::media_path;

pub const RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;

/// Sample frames for a duration.
pub fn frames_for(us: Micros) -> usize {
    // Rounded: block starts are whole microseconds, so truncating could lose a sample at every join.
    ((us as i128 * RATE as i128 + 500_000) / 1_000_000).max(0) as usize
}

struct Source {
    dec: Option<AudioDecoder>,
    /// Source time of `buf[0]`.
    buf_start: Micros,
    buf: Vec<f32>,
    eof: bool,
    used: u64,
}

impl Source {
    fn open(path: &Path) -> Source {
        let spec = AudioOutputSpec { sample_rate: RATE, channels: CHANNELS as u16, format: SampleFormat::F32 };
        Source { dec: AudioDecoder::open(path, spec).ok(), buf_start: 0, buf: Vec::new(), eof: false, used: 0 }
    }

    /// `n` sample frames starting at source time `at` (zeros where there is no audio).
    fn read(&mut self, at: Micros, n: usize) -> Vec<f32> {
        let Some(dec) = self.dec.as_mut() else { return vec![0.0; n * CHANNELS] };
        let buffered_end = self.buf_start + (self.buf.len() / CHANNELS) as Micros * 1_000_000 / RATE as Micros;
        let tolerance = 1_000_000 / RATE as Micros + 1;
        if at + tolerance >= self.buf_start && at <= buffered_end + tolerance && !(self.buf.is_empty() && self.eof) {
            // Drop what is before `at`.
            let skip = frames_for((at - self.buf_start).max(0)).min(self.buf.len() / CHANNELS);
            self.buf.drain(..skip * CHANNELS);
            self.buf_start = at;
        } else {
            if dec.seek(at).is_err() {
                return vec![0.0; n * CHANNELS];
            }
            self.buf.clear();
            self.buf_start = at;
            self.eof = false;
        }
        while self.buf.len() < n * CHANNELS && !self.eof {
            match dec.next_chunk() {
                Ok(Some(c)) => match c.samples {
                    Samples::F32(v) => self.buf.extend_from_slice(&v),
                    Samples::S16(v) => self.buf.extend(v.into_iter().map(|s| s as f32 / 32768.0)),
                },
                Ok(None) | Err(_) => self.eof = true,
            }
        }
        let take = (n * CHANNELS).min(self.buf.len());
        let mut out: Vec<f32> = self.buf[..take].to_vec();
        out.resize(n * CHANNELS, 0.0);
        out
    }
}

fn has_sound(doc: &Document, item: &Item) -> bool {
    match item.kind.as_str() {
        "audio" => true,
        "video" => item
            .media_id()
            .and_then(|m| doc.project.media.get(m))
            .and_then(|m| m.data())
            .map(|d| {
                d.get("hasAudio").and_then(Value::as_bool).unwrap_or(true)
                    && d.get("type").and_then(Value::as_str) != Some("image")
            })
            .unwrap_or(false),
        _ => false,
    }
}

fn fade_gain(item: &Item, t: Micros) -> f32 {
    let mut g = 1.0f32;
    let dur = |k: &str| item.fields.get(k).and_then(|v| v.get("duration")).and_then(json_to_micros).filter(|d| *d > 0);
    if let Some(d) = dur("audioFadeIn")
        && t - item.start < d
    {
        g *= ((t - item.start) as f32 / d as f32).clamp(0.0, 1.0);
    }
    if let Some(d) = dur("audioFadeOut")
        && item.end - t < d
    {
        g *= ((item.end - t) as f32 / d as f32).clamp(0.0, 1.0);
    }
    // An `audio-fade` transition ramps the sound (the picture is untouched).
    for (key, entering) in [("transitionIn", true), ("transitionOut", false)] {
        if let Some(tr) = crate::eval::transition(item, key).filter(|t| t.kind == "audio-fade") {
            let into = if entering { t - item.start } else { item.end - t };
            if into < tr.duration {
                g *= crate::eval::ease(&tr.easing, into as f64 / tr.duration as f64).clamp(0.0, 1.0) as f32;
            }
        }
    }
    g
}

/// Mixes timeline audio. Keeps decoders between calls so sequential blocks decode once.
pub struct Mixer {
    bundle: Option<PathBuf>,
    sources: HashMap<String, Source>,
    tick: u64,
}

impl Mixer {
    pub fn new(bundle: Option<&Path>) -> Self {
        Mixer { bundle: bundle.map(Path::to_path_buf), sources: HashMap::new(), tick: 0 }
    }

    /// `frames` stereo sample frames starting at timeline time `t0`.
    pub fn mix(&mut self, doc: &Document, t0: Micros, frames: usize) -> Vec<f32> {
        self.tick += 1;
        let mut out = vec![0f32; frames * CHANNELS];
        let t1 = t0 + frames as Micros * 1_000_000 / RATE as Micros;
        for track in &doc.project.tracks {
            if track.fields.get("muted").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            for item in doc.items_on_track(&track.id) {
                if item.end <= t0 || item.start >= t1 || item.is_pending() || !has_sound(doc, item) {
                    continue;
                }
                let f = &item.fields;
                if f.get("muted").and_then(Value::as_bool) == Some(true)
                    || f.get("playbackDirection").and_then(Value::as_str) == Some("reverse")
                {
                    continue;
                }
                let volume = f.get("volume").and_then(Value::as_f64).unwrap_or(100.0).clamp(0.0, 200.0) as f32 / 100.0;
                if volume <= 0.0 {
                    continue;
                }
                let speed = f.get("playbackSpeed").and_then(Value::as_f64).unwrap_or(1.0).max(0.01);
                let a = item.start.max(t0);
                let b = item.end.min(t1);
                let off = frames_for(a - t0);
                let n = frames_for(b - t0).min(frames).saturating_sub(off);
                if n == 0 {
                    continue;
                }
                let Some(mid) = item.media_id() else { continue };
                let Some(path) = media_path(doc, self.bundle.as_deref(), mid) else { continue };
                let src = self.sources.entry(item.id.clone()).or_insert_with(|| Source::open(&path));
                src.used = self.tick;
                let n_src = ((n as f64) * speed).round().max(1.0) as usize;
                let samples = src.read(source_time(item, a), n_src);
                for i in 0..n {
                    let t = a + i as Micros * 1_000_000 / RATE as Micros;
                    let g = volume * fade_gain(item, t);
                    let si = (((i as f64) * speed) as usize).min(n_src - 1);
                    for c in 0..CHANNELS {
                        out[(off + i) * CHANNELS + c] += samples[si * CHANNELS + c] * g;
                    }
                }
            }
        }
        // Overlapping loud sources: clamp rather than wrap.
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
        let tick = self.tick;
        self.sources.retain(|_, s| tick - s.used < 64);
        out
    }
}
