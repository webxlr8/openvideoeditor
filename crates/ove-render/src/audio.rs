//! Audio mixdown of the timeline: every video/audio item with sound, volume (0–200 %) and volume keyframes, item and
//! track mute, audio fades and `audio-fade` transitions, playback speed, to interleaved stereo f32 at 48 kHz, through
//! a true-peak limiter.
//! Reverse playback is silent in v1.
//!
//! - Sources are decoded and converted to 48 kHz by ove-media (libswresample, band-limited).
//! - Playback speed resamples with a band-limited sinc resampler (rubato), so 2× speed does not alias; pitch changes
//!   with speed (pitch preservation is not implemented).
//! - Volume keyframes (percent, like `volume`) are interpolated in dB with a -60 dB floor, evaluated every 32 samples
//!   on a grid fixed to timeline time, so preview and export give the same samples (design D1).
//! - The sum goes through a true-peak look-ahead limiter with a -1 dBTP ceiling instead of being clipped; audio
//!   under the ceiling passes through unchanged.

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
/// True-peak ceiling of the mix: -1 dBTP (the sample peak stays under it too).
pub const CEILING: f32 = 0.891_250_9;
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

/// Floor of the volume curve (dB): a keyframe value at or below it is silence.
pub const VOLUME_FLOOR_DB: f64 = -60.0;
/// Volume keyframes are evaluated once per block of this many sample frames, on a grid fixed to timeline time, and
/// ramped linearly inside the block (design D1).
pub const GAIN_BLOCK: usize = 32;

/// A volume value in percent (100 = unity) as dB, floored at [`VOLUME_FLOOR_DB`].
pub fn percent_to_db(percent: f64) -> f64 {
    if percent <= 0.0 { VOLUME_FLOOR_DB } else { (20.0 * (percent / 100.0).log10()).max(VOLUME_FLOOR_DB) }
}

/// Linear gain for a level in dB; the floor and below are silence.
pub fn db_to_gain(db: f64) -> f32 {
    if db <= VOLUME_FLOOR_DB { 0.0 } else { 10f64.powf(db / 20.0) as f32 }
}

/// An item's volume keyframes, in dB, interpolated in dB with each segment's easing (its start keyframe's, else the
/// track's; design D1 and add-markers-keyframe-lanes-and-versions D6).
#[derive(Debug, Clone, PartialEq)]
pub struct VolumeCurve {
    easing: String,
    /// Keyframes in time order, values in dB.
    points: Vec<ove_core::retime::Key>,
}

impl VolumeCurve {
    /// The item's `volume` keyframe track, if it has one with at least one keyframe.
    pub fn of(item: &Item) -> Option<VolumeCurve> {
        let tracks = item.fields.get("keyframeTracks")?.as_array()?;
        let track = tracks.iter().find(|t| t.get("property").and_then(Value::as_str) == Some("volume"))?;
        let easing = ove_core::retime::track_easing(track).to_string();
        let points: Vec<ove_core::retime::Key> = ove_core::retime::keys(track)
            .into_iter()
            .filter(|k| k.value.is_finite())
            .map(|k| ove_core::retime::Key { value: percent_to_db(k.value.clamp(0.0, 200.0)), ..k })
            .collect();
        (!points.is_empty()).then_some(VolumeCurve { easing, points })
    }

    /// Level in dB at item-relative time `rel`: held before the first and after the last keyframe.
    pub fn db_at(&self, rel: Micros) -> f64 {
        ove_core::retime::value_at(&self.points, &self.easing, rel).unwrap_or(VOLUME_FLOOR_DB)
    }

    /// Linear gain at item-relative time `rel`.
    pub fn gain_at(&self, rel: Micros) -> f32 {
        db_to_gain(self.db_at(rel))
    }
}

/// Gain of a volume curve at global timeline sample frame `frame`: exact at multiples of [`GAIN_BLOCK`], linear in
/// between, so the result does not depend on how the timeline is split into mix calls.
struct BlockGain<'a> {
    curve: &'a VolumeCurve,
    item_start: Micros,
    block: Option<(i64, f32, f32)>,
}

impl<'a> BlockGain<'a> {
    fn new(curve: &'a VolumeCurve, item_start: Micros) -> Self {
        BlockGain { curve, item_start, block: None }
    }

    fn at(&mut self, frame: i64) -> f32 {
        let start = frame.div_euclid(GAIN_BLOCK as i64) * GAIN_BLOCK as i64;
        let (s, g0, g1) = match self.block {
            Some(b) if b.0 == start => b,
            _ => {
                let g = |f: i64| self.curve.gain_at(frame_to_us(f) - self.item_start);
                let b = (start, g(start), g(start + GAIN_BLOCK as i64));
                self.block = Some(b);
                b
            }
        };
        let p = (frame - s) as f32 / GAIN_BLOCK as f32;
        g0 + (g1 - g0) * p
    }
}

/// True-peak look-ahead limiter (design D8): the output's true peak (4× oversampled) stays at or under -1 dBTP.
///
/// Each input frame's required gain is the ceiling over its true peak (1 when under the ceiling). The applied gain is
/// the minimum of the required gains over the look-ahead window, released smoothly, then averaged over
/// [`LOOKAHEAD`] frames, so the gain is already down when a peak leaves the delay line and never jumps. Audio that
/// stays under the ceiling passes through unchanged (only delayed, which the mixer compensates). The result is the
/// same however the audio is split into blocks.
pub struct Limiter {
    /// Input frames waiting to leave (the delay line).
    delay: VecDeque<[f32; CHANNELS]>,
    /// Required gains of the last [`MIN_WINDOW`] input frames.
    req: VecDeque<f32>,
    env: f32,
    boxed: VecDeque<f32>,
    box_sum: f64,
    tp: [crate::loudness::TruePeak; CHANNELS],
    release: f32,
}

/// Look-ahead in frames: 1.5 ms at 48 kHz.
pub const LOOKAHEAD: usize = 72;
/// The true-peak estimate describes a point about 6 frames back, so the delay is longer than the look-ahead by that.
const TP_LAG: usize = 7;
/// Frames between a sample entering the limiter and leaving it.
pub const LIMITER_DELAY: usize = LOOKAHEAD + TP_LAG;
const MIN_WINDOW: usize = LOOKAHEAD + TP_LAG + 2;
/// The gain target sits a hair under the ceiling so gain changes inside a frame cannot push a true peak over it.
const TARGET: f32 = CEILING * 0.995;

impl Default for Limiter {
    fn default() -> Self {
        Limiter {
            delay: VecDeque::with_capacity(LIMITER_DELAY + 1),
            req: VecDeque::with_capacity(MIN_WINDOW + 1),
            env: 1.0,
            boxed: VecDeque::with_capacity(LOOKAHEAD + 1),
            box_sum: 0.0,
            tp: Default::default(),
            release: 1.0 - (-1.0 / (RELEASE_S * RATE as f32)).exp(),
        }
    }
}

impl Limiter {
    /// Adds one frame; once the delay line is full, returns the limited frame that entered [`LIMITER_DELAY`] frames
    /// earlier.
    pub fn push(&mut self, frame: [f32; CHANNELS]) -> Option<[f32; CHANNELS]> {
        let mut peak = 0f32;
        for (c, s) in frame.iter().enumerate() {
            peak = peak.max(self.tp[c].push(f64::from(*s)) as f32);
        }
        let need = if peak > TARGET { TARGET / peak } else { 1.0 };
        self.req.push_back(need);
        if self.req.len() > MIN_WINDOW {
            self.req.pop_front();
        }
        let m = self.req.iter().copied().fold(1.0f32, f32::min);
        self.env = if m < self.env { m } else { self.env + (m - self.env) * self.release };
        self.boxed.push_back(self.env);
        self.box_sum += f64::from(self.env);
        if self.boxed.len() > LOOKAHEAD {
            self.box_sum -= f64::from(self.boxed.pop_front().unwrap_or(1.0));
        }
        self.delay.push_back(frame);
        if self.delay.len() <= LIMITER_DELAY {
            return None;
        }
        let out = self.delay.pop_front()?;
        // A full box averages gains that are all at or under every required gain of the frame leaving now.
        let g = if self.boxed.len() == LOOKAHEAD { (self.box_sum / LOOKAHEAD as f64) as f32 } else { self.env };
        let g = g.min(1.0);
        Some(out.map(|s| (s * g).clamp(-CEILING, CEILING)))
    }

    /// Limits interleaved stereo; returns as many frames as left the delay line.
    pub fn process(&mut self, samples: &[f32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(samples.len());
        for f in samples.chunks_exact(CHANNELS) {
            if let Some(o) = self.push([f[0], f[1]]) {
                out.extend_from_slice(&o);
            }
        }
        out
    }

    pub fn reset(&mut self) {
        *self = Limiter::default();
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
    /// Static gain before the limiter (loudness normalisation), linear.
    gain: f32,
}

impl Mixer {
    pub fn new(bundle: Option<&Path>) -> Self {
        Mixer {
            bundle: bundle.map(Path::to_path_buf),
            sources: HashMap::new(),
            tick: 0,
            limiter: Limiter::default(),
            next_t0: None,
            gain: 1.0,
        }
    }

    /// A static gain applied to the whole mix before the limiter (loudness normalisation on export).
    pub fn set_gain_db(&mut self, db: f64) {
        self.gain = 10f64.powf(db / 20.0) as f32;
        self.next_t0 = None;
    }

    /// `frames` stereo sample frames starting at timeline time `t0`, through the gain and the true-peak limiter. The
    /// limiter's delay is compensated: the mix is read [`LIMITER_DELAY`] frames ahead, so the output lines up with `t0`
    /// and a sequential block continues the previous one exactly.
    pub fn mix(&mut self, doc: &Document, t0: Micros, frames: usize) -> Vec<f32> {
        let base = frames_for(t0) as i64;
        if self.next_t0 != Some(t0) {
            // After a jump: fill the delay line with the frames before the look-ahead point.
            self.limiter.reset();
            let prime = self.mix_raw(doc, frame_to_us(base), LIMITER_DELAY);
            let _ = self.limiter.process(&self.scaled(prime));
        }
        let ahead = self.mix_raw(doc, frame_to_us(base + LIMITER_DELAY as i64), frames);
        let mut out = self.limiter.process(&self.scaled(ahead));
        out.resize(frames * CHANNELS, 0.0);
        self.next_t0 = Some(t0 + frames as Micros * 1_000_000 / RATE as Micros);
        out
    }

    fn scaled(&self, mut v: Vec<f32>) -> Vec<f32> {
        if self.gain != 1.0 {
            v.iter_mut().for_each(|s| *s *= self.gain);
        }
        v
    }

    /// The sum of every audible item for `frames` frames from `t0`, before gain and limiter.
    pub fn mix_raw(&mut self, doc: &Document, t0: Micros, frames: usize) -> Vec<f32> {
        self.tick += 1;
        let mut out = vec![0f32; frames * CHANNELS];
        let t1 = t0 + frames as Micros * 1_000_000 / RATE as Micros;
        let base = frames_for(t0) as i64;
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
                let curve = VolumeCurve::of(item);
                let mut keyed = curve.as_ref().map(|c| BlockGain::new(c, item.start));
                for i in 0..n {
                    // Times come from the global sample index, so any split into mix calls gives the same samples.
                    let frame = base + (off + i) as i64;
                    let t = frame_to_us(frame);
                    let mut g = volume * fade_gain(item, t);
                    if let Some(k) = keyed.as_mut() {
                        g *= k.at(frame);
                    }
                    for c in 0..CHANNELS {
                        out[(off + i) * CHANNELS + c] += samples[i * CHANNELS + c] * g;
                    }
                }
            }
        }
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

    fn true_peak(v: &[f32]) -> f32 {
        let mut tp = [crate::loudness::TruePeak::default(), crate::loudness::TruePeak::default()];
        let mut m = 0f64;
        for f in v.chunks_exact(2) {
            for c in 0..2 {
                m = m.max(tp[c].push(f64::from(f[c])));
            }
        }
        m as f32
    }

    #[test]
    fn hot_audio_stays_under_minus_one_dbtp() {
        // A full-scale 11 kHz tone phased so its true peak sits between samples, at +6 dB.
        let hot: Vec<f32> = (0..RATE as usize)
            .flat_map(|i| {
                let v = 2.0 * (2.0 * std::f32::consts::PI * 11_025.0 * i as f32 / RATE as f32 + 0.7).sin();
                [v, v]
            })
            .collect();
        let out = Limiter::default().process(&hot);
        let tp = 20.0 * true_peak(&out).log10();
        assert!(tp <= -1.0, "true peak {tp:.3} dBTP");
        // Limited, not crushed: the steady part sits just under the ceiling.
        let tail = &out[out.len() / 2..];
        assert!(20.0 * true_peak(tail).log10() > -2.0);
    }

    #[test]
    fn audio_under_the_ceiling_passes_through_exactly() {
        let quiet = sine(440.0, 0.5, 9_600);
        let out = Limiter::default().process(&quiet);
        assert_eq!(out.len(), quiet.len() - LIMITER_DELAY * 2);
        assert_eq!(&out[..], &quiet[..out.len()]);
    }

    #[test]
    fn a_transient_is_caught_before_it_arrives() {
        // Silence, then a sudden full-scale burst: the gain is down when the burst leaves the delay line.
        let mut v = vec![0f32; 2_000 * 2];
        v.extend(sine(1_000.0, 1.0, 4_000));
        let out = Limiter::default().process(&v);
        assert!(out.iter().all(|s| s.abs() <= CEILING), "sample peak over the ceiling");
        assert!(20.0 * true_peak(&out).log10() <= -1.0);
    }

    #[test]
    fn limiter_output_does_not_depend_on_block_size() {
        let input = sine(220.0, 1.6, 9_600);
        let whole = Limiter::default().process(&input);
        let mut l = Limiter::default();
        let (a, b) = input.split_at(1_234 * CHANNELS);
        let mut split = l.process(a);
        split.extend(l.process(b));
        assert_eq!(whole, split);
    }
}

#[cfg(test)]
mod volume_tests {
    use super::*;
    use serde_json::json;

    fn item(tracks: Value) -> Item {
        let v = json!({"id": "a", "trackId": "t", "type": "audio", "startTime": 2, "endTime": 10, "keyframeTracks": tracks});
        Item::from_json(&v, "a").unwrap()
    }

    #[test]
    fn linear_fade_follows_the_db_curve() {
        let it =
            item(json!([{"property": "volume", "keyframes": [{"time": 0, "value": 100}, {"time": 1, "value": 0}]}]));
        let c = VolumeCurve::of(&it).unwrap();
        // 100 % → 0 % over 1 s, linear in dB: 0 dB to the -60 dB floor.
        for ms in (0..1000).step_by(50) {
            let want = -60.0 * ms as f64 / 1000.0;
            let got = c.db_at(ms * 1000);
            assert!((got - want).abs() < 0.1, "{ms} ms: {got} dB, want {want}");
        }
        assert!((c.db_at(500_000) + 30.0).abs() < 1e-9);
        assert_eq!(c.gain_at(1_000_000), 0.0, "silent from the last keyframe");
        assert_eq!(c.gain_at(5_000_000), 0.0);
        assert_eq!(c.gain_at(-5), 1.0, "held before the first keyframe");
        // The block gain at the global grid tracks the curve within 0.1 dB away from the floor.
        let mut b = BlockGain::new(&c, 2_000_000);
        for frame in (96_000..96_000 + 40_000).step_by(7) {
            let rel = frame_to_us(frame) - 2_000_000;
            let want = c.db_at(rel);
            let got = 20.0 * (b.at(frame) as f64).log10();
            assert!((got - want).abs() < 0.1, "frame {frame}: {got} dB, want {want}");
        }
    }

    #[test]
    fn percent_and_db_conversions() {
        assert_eq!(percent_to_db(100.0), 0.0);
        assert!((percent_to_db(200.0) - 6.0206).abs() < 1e-3);
        assert!((percent_to_db(50.0) + 6.0206).abs() < 1e-3);
        assert_eq!(percent_to_db(0.0), VOLUME_FLOOR_DB);
        assert_eq!(percent_to_db(0.01), VOLUME_FLOOR_DB);
        assert_eq!(db_to_gain(VOLUME_FLOOR_DB), 0.0);
        assert_eq!(db_to_gain(0.0), 1.0);
    }

    #[test]
    fn easing_and_missing_tracks() {
        let none = item(json!([{"property": "opacity", "keyframes": [{"time": 0, "value": 1}]}]));
        assert!(VolumeCurve::of(&none).is_none());
        let empty = item(json!([{"property": "volume", "keyframes": []}]));
        assert!(VolumeCurve::of(&empty).is_none());
        let eased = item(json!([{"property": "volume", "easing": "ease-in", "keyframes": [
            {"time": 0, "value": 100}, {"time": 1, "value": 10}]}]));
        let c = VolumeCurve::of(&eased).unwrap();
        // -20 dB at the end; ease-in moves less than half way by the midpoint.
        assert!((c.db_at(1_000_000) + 20.0).abs() < 1e-9);
        assert!(c.db_at(500_000) > -10.0);
    }
}
