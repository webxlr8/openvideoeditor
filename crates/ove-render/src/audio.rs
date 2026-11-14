//! Audio mixdown of the timeline: every video/audio item with sound, volume (0–200 %), item and track mute, audio
//! fades and `audio-fade` transitions, playback speed, to interleaved stereo f32 at 48 kHz, through a peak limiter.
//! Reverse playback is silent in v1.
//!
//! - Sources are decoded and converted to 48 kHz by ove-media (libswresample, band-limited).
//! - Playback speed resamples with a band-limited sinc resampler (rubato), so 2× speed does not alias; pitch changes
//!   with speed (pitch preservation is not implemented).
//! - The sum goes through a soft-knee peak limiter with a -1 dBFS ceiling instead of being clipped.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use ove_core::model::{Document, Item};
use ove_core::time::{Micros, json_to_micros};
use ove_media::{AudioDecoder, AudioOutputSpec, SampleFormat, Samples};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Async, FixedAsync, Resampler, SincInterpolationParameters, WindowFunction};
use serde_json::Value;

use crate::eval::source_time;
use crate::sources::media_path;

pub const RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;
/// Peak ceiling of the mix: -1 dBFS.
pub const CEILING: f32 = 0.891_250_9;
const CEILING_DB: f32 = -1.0;
/// Width of the limiter's soft knee (dB), centred on the ceiling: gain reduction starts at -4 dBFS.
const KNEE_DB: f32 = 6.0;
/// Limiter release time constant (s).
const RELEASE_S: f32 = 0.08;
/// Output frames per resampler call.
const RESAMPLE_CHUNK: usize = 1024;

/// Sample frames for a duration.
pub fn frames_for(us: Micros) -> usize {
    // Rounded: block starts are whole microseconds, so truncating could lose a sample at every join.
    ((us as i128 * RATE as i128 + 500_000) / 1_000_000).max(0) as usize
}

/// Speed change by band-limited resampling: the resampler's state carries over between sequential blocks, so block
/// boundaries are seamless; a jump restarts it.
struct SpeedState {
    resampler: Async<f32>,
    speed: f64,
    /// Timeline time the next output frame is for.
    next_t: Micros,
    /// Source position (48 kHz frame index) of the next input frame.
    src_frame: i64,
    /// Output frames still to drop at a (re)start: the resampler's delay.
    skip: usize,
    ready: VecDeque<f32>,
}

impl SpeedState {
    fn new(speed: f64) -> Option<SpeedState> {
        let params = SincInterpolationParameters::new(128, WindowFunction::BlackmanHarris2);
        let resampler = Async::<f32>::new_sinc(1.0 / speed, 1.0, &params, RESAMPLE_CHUNK, CHANNELS, FixedAsync::Output)
            .inspect_err(|e| tracing::warn!(error = %e, speed, "cannot resample for this speed"))
            .ok()?;
        Some(SpeedState { resampler, speed, next_t: Micros::MIN, src_frame: 0, skip: 0, ready: VecDeque::new() })
    }
}

fn frame_to_us(frame: i64) -> Micros {
    (frame as i128 * 1_000_000 / RATE as i128) as Micros
}

struct Source {
    dec: Option<AudioDecoder>,
    /// Source time of `buf[0]`.
    buf_start: Micros,
    buf: Vec<f32>,
    eof: bool,
    used: u64,
    speed: Option<SpeedState>,
}

impl Source {
    fn open(path: &Path) -> Source {
        let spec = AudioOutputSpec { sample_rate: RATE, channels: CHANNELS as u16, format: SampleFormat::F32 };
        Source {
            dec: AudioDecoder::open(path, spec).ok(),
            buf_start: 0,
            buf: Vec::new(),
            eof: false,
            used: 0,
            speed: None,
        }
    }

    /// `n` output frames for timeline time `t` (source time `at`) played at `speed` ≠ 1.
    fn read_speed(&mut self, t: Micros, at: Micros, n: usize, speed: f64) -> Vec<f32> {
        if self.speed.as_ref().is_none_or(|s| s.speed != speed) {
            self.speed = SpeedState::new(speed);
        }
        let Some(mut st) = self.speed.take() else { return vec![0.0; n * CHANNELS] };
        // Sequential blocks continue the stream (within a sample of rounding); anything else restarts it.
        if st.next_t.abs_diff(t) > 2 {
            st.resampler.reset();
            st.src_frame = (at as i128 * RATE as i128 / 1_000_000) as i64;
            st.skip = st.resampler.output_delay();
            st.ready.clear();
        }
        let mut out_buf = vec![0f32; st.resampler.output_frames_max() * CHANNELS];
        while st.ready.len() < n * CHANNELS {
            let need = st.resampler.input_frames_next();
            let input = self.read(frame_to_us(st.src_frame), need);
            st.src_frame += need as i64;
            let frames_out = st.resampler.output_frames_next();
            let (Ok(inp), Ok(mut outp)) = (
                InterleavedSlice::new(&input[..], CHANNELS, need),
                InterleavedSlice::new_mut(&mut out_buf[..], CHANNELS, frames_out),
            ) else {
                break;
            };
            let Ok((_, produced)) = st.resampler.process_into_buffer(&inp, &mut outp, None) else { break };
            let drop = st.skip.min(produced);
            st.skip -= drop;
            st.ready.extend(&out_buf[drop * CHANNELS..produced * CHANNELS]);
        }
        let take = (n * CHANNELS).min(st.ready.len());
        let mut out: Vec<f32> = st.ready.drain(..take).collect();
        out.resize(n * CHANNELS, 0.0);
        st.next_t = t + frame_to_us(n as i64);
        self.speed = Some(st);
        out
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

/// Peak limiter with a soft knee: gain reduction starts 3 dB below the -1 dBFS ceiling and the output never exceeds
/// it. Attack is instant (the ceiling holds without look-ahead, so the result does not depend on how the timeline is
/// split into blocks); release is smooth.
pub struct Limiter {
    gain: f32,
    release: f32,
}

impl Default for Limiter {
    fn default() -> Self {
        Limiter { gain: 1.0, release: 1.0 - (-1.0 / (RELEASE_S * RATE as f32)).exp() }
    }
}

impl Limiter {
    /// Static curve: the gain for a peak level (linear), from the soft-knee characteristic with an infinite ratio.
    fn target(peak: f32) -> f32 {
        const KNEE_START: f32 = 0.630_957_3; // -4 dBFS
        if peak <= KNEE_START {
            return 1.0;
        }
        let level = 20.0 * peak.log10();
        let over = level - CEILING_DB + KNEE_DB / 2.0;
        let out = if over < KNEE_DB { level - over * over / (2.0 * KNEE_DB) } else { CEILING_DB };
        10f32.powf((out - level) / 20.0)
    }

    /// Limits interleaved stereo in place.
    pub fn process(&mut self, samples: &mut [f32]) {
        for frame in samples.chunks_exact_mut(CHANNELS) {
            let peak = frame.iter().fold(0f32, |m, s| m.max(s.abs()));
            let target = Self::target(peak);
            self.gain = if target < self.gain { target } else { self.gain + (target - self.gain) * self.release };
            for s in frame {
                *s = (*s * self.gain).clamp(-CEILING, CEILING);
            }
        }
    }

    pub fn reset(&mut self) {
        self.gain = 1.0;
    }
}

/// Mixes timeline audio. Keeps decoders between calls so sequential blocks decode once.
pub struct Mixer {
    bundle: Option<PathBuf>,
    sources: HashMap<String, Source>,
    tick: u64,
    limiter: Limiter,
    /// Timeline time the next sequential block starts at (the limiter restarts after a jump).
    next_t0: Option<Micros>,
}

impl Mixer {
    pub fn new(bundle: Option<&Path>) -> Self {
        Mixer {
            bundle: bundle.map(Path::to_path_buf),
            sources: HashMap::new(),
            tick: 0,
            limiter: Limiter::default(),
            next_t0: None,
        }
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
                let samples = if (speed - 1.0).abs() < 1e-9 {
                    src.read(source_time(item, a), n)
                } else {
                    src.read_speed(a, source_time(item, a), n, speed)
                };
                for i in 0..n {
                    let t = a + i as Micros * 1_000_000 / RATE as Micros;
                    let g = volume * fade_gain(item, t);
                    for c in 0..CHANNELS {
                        out[(off + i) * CHANNELS + c] += samples[i * CHANNELS + c] * g;
                    }
                }
            }
        }
        // Loud or overlapping sources: limit to -1 dBFS rather than clip.
        if self.next_t0 != Some(t0) {
            self.limiter.reset();
        }
        self.limiter.process(&mut out);
        self.next_t0 = Some(t1);
        let tick = self.tick;
        self.sources.retain(|_, s| tick - s.used < 64);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, amp: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let v = amp * (2.0 * std::f32::consts::PI * freq * i as f32 / RATE as f32).sin();
                [v, v]
            })
            .collect()
    }

    fn peak(v: &[f32]) -> f32 {
        v.iter().fold(0f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn limiter_holds_the_ceiling_and_leaves_quiet_audio_alone() {
        let mut l = Limiter::default();
        let mut loud = sine(1000.0, 2.0, RATE as usize);
        l.process(&mut loud);
        assert!(peak(&loud) <= CEILING, "peak {}", peak(&loud));
        assert!(peak(&loud[RATE as usize..]) > 0.85, "not over-attenuated: {}", peak(&loud[RATE as usize..]));
        // After the burst, quiet audio (-12 dBFS) comes back untouched once the gain has released.
        let mut l = Limiter::default();
        let quiet = sine(440.0, 0.25, RATE as usize / 2);
        let mut q = quiet.clone();
        l.process(&mut q);
        assert_eq!(q, quiet);
    }

    #[test]
    fn limiter_curve_is_continuous_and_soft() {
        assert_eq!(Limiter::target(0.5), 1.0);
        let mut last = f32::MAX;
        for i in 0..400 {
            let p = 0.6 + i as f32 * 0.01;
            let out = p * Limiter::target(p);
            assert!(out <= CEILING + 1e-6, "{p}: {out}");
            assert!(Limiter::target(p) <= last + 1e-6, "gain falls monotonically");
            last = Limiter::target(p);
        }
        // Inside the knee the reduction is gentle: at -2 dBFS less than 1 dB.
        let at = 10f32.powf(-2.0 / 20.0);
        assert!(Limiter::target(at) > 10f32.powf(-1.0 / 20.0));
    }

    #[test]
    fn limiter_output_does_not_depend_on_block_size() {
        let input = sine(220.0, 1.6, 9_600);
        let mut whole = input.clone();
        Limiter::default().process(&mut whole);
        let mut split = input;
        let mut l = Limiter::default();
        let (a, b) = split.split_at_mut(1_234 * CHANNELS);
        l.process(a);
        l.process(b);
        assert_eq!(whole, split);
    }
}
