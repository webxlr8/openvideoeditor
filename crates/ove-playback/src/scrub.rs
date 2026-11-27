//! Scrub audio (design D6): short windowed grains of the real timeline mix, played while the play head is dragged.
//!
//! - The mix comes from a caller-supplied producer (the app passes the shared mixer), in 1 s windows kept in an LRU
//!   cache keyed by revision and window start. An edit has a new revision, so it misses the stale windows: no
//!   debounce is needed.
//! - Each scrub event takes one 60 ms grain at the play head with a Hann window, so a grain starts and ends at zero.
//! - A new grain replaces the one still queued: the queued audio fades out over 2 ms first ([`splice`]), so cutting it
//!   short makes no click.
//! - After a grain, the neighbouring window in the scrub direction is mixed ahead ([`Scrubber::prefetch`]), so the
//!   next grain usually finds its window cached.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

/// Sample rate and channels of grains (the mixer's format).
pub const SCRUB_RATE: u32 = 48_000;
pub const SCRUB_CHANNELS: usize = 2;
/// Length of a cached mix window.
pub const WINDOW_US: i64 = 1_000_000;
/// Length of a grain.
pub const GRAIN_US: i64 = 60_000;
/// Fade-out of queued audio a new grain replaces.
pub const SPLICE_FADE_FRAMES: usize = 96;
/// Windows kept (about 32 s of timeline).
const CACHE_WINDOWS: usize = 32;

const WINDOW_FRAMES: usize = (SCRUB_RATE as i64 * WINDOW_US / 1_000_000) as usize;
pub const GRAIN_FRAMES: usize = (SCRUB_RATE as i64 * GRAIN_US / 1_000_000) as usize;

/// Start of the window holding media time `t_us`.
pub fn window_start(t_us: i64) -> i64 {
    t_us.div_euclid(WINDOW_US) * WINDOW_US
}

fn frames_at(us: i64) -> usize {
    ((us as i128 * SCRUB_RATE as i128 + 500_000) / 1_000_000).max(0) as usize
}

/// Applies a Hann window to interleaved stereo in place.
pub fn hann(samples: &mut [f32]) {
    let n = samples.len() / SCRUB_CHANNELS;
    if n < 2 {
        samples.fill(0.0);
        return;
    }
    for (i, frame) in samples.chunks_exact_mut(SCRUB_CHANNELS).enumerate() {
        let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / (n - 1) as f64).cos();
        for s in frame {
            *s *= w as f32;
        }
    }
}

/// What to queue when `grain` replaces the `queued` audio: the first [`SPLICE_FADE_FRAMES`] of the queued audio
/// faded out (raised cosine) to zero, then the grain.
pub fn splice(queued: &[f32], grain: &[f32]) -> Vec<f32> {
    let keep = (queued.len() / SCRUB_CHANNELS).min(SPLICE_FADE_FRAMES);
    let mut out = Vec::with_capacity(keep * SCRUB_CHANNELS + grain.len());
    for (i, frame) in queued.chunks_exact(SCRUB_CHANNELS).take(keep).enumerate() {
        // Raised-cosine fade: it leaves the queued audio with almost no step and ends almost at zero.
        let g = (0.5 * (1.0 + (std::f64::consts::PI * (i + 1) as f64 / (keep + 1) as f64).cos())) as f32;
        out.extend(frame.iter().map(|s| s * g));
    }
    out.extend_from_slice(grain);
    out
}

/// Produces `frames` of the mix of revision `rev` from media time `start_us`.
pub trait MixSource {
    fn mix(&mut self, rev: i64, start_us: i64, frames: usize) -> Vec<f32>;
}

impl<F: FnMut(i64, i64, usize) -> Vec<f32>> MixSource for F {
    fn mix(&mut self, rev: i64, start_us: i64, frames: usize) -> Vec<f32> {
        self(rev, start_us, frames)
    }
}

/// The window cache and grain maker.
pub struct Scrubber<M: MixSource> {
    source: M,
    windows: HashMap<(i64, i64), Arc<Vec<f32>>>,
    order: VecDeque<(i64, i64)>,
    last_t: Option<i64>,
    /// Windows mixed so far (for tests and diagnostics).
    pub mixed: u64,
}

impl<M: MixSource> Scrubber<M> {
    pub fn new(source: M) -> Self {
        Scrubber { source, windows: HashMap::new(), order: VecDeque::new(), last_t: None, mixed: 0 }
    }

    /// The mix producer (to hand it new documents).
    pub fn source_mut(&mut self) -> &mut M {
        &mut self.source
    }

    pub fn is_cached(&self, rev: i64, start: i64) -> bool {
        self.windows.contains_key(&(rev, start))
    }

    fn window(&mut self, rev: i64, start: i64) -> Arc<Vec<f32>> {
        if let Some(w) = self.windows.get(&(rev, start)) {
            let w = w.clone();
            self.order.retain(|k| *k != (rev, start));
            self.order.push_back((rev, start));
            return w;
        }
        let mut v = self.source.mix(rev, start, WINDOW_FRAMES);
        v.resize(WINDOW_FRAMES * SCRUB_CHANNELS, 0.0);
        let w = Arc::new(v);
        self.mixed += 1;
        self.windows.insert((rev, start), w.clone());
        self.order.push_back((rev, start));
        while self.order.len() > CACHE_WINDOWS {
            if let Some(k) = self.order.pop_front() {
                self.windows.remove(&k);
            }
        }
        w
    }

    /// The grain for media time `t_us` of revision `rev`: 60 ms of the mix from `t_us`, Hann-windowed. Returns the
    /// grain and whether every window it needed was cached.
    pub fn grain(&mut self, rev: i64, t_us: i64) -> (Vec<f32>, bool) {
        let t_us = t_us.max(0);
        let mut out = Vec::with_capacity(GRAIN_FRAMES * SCRUB_CHANNELS);
        let mut cached = true;
        let mut at = t_us;
        while out.len() < GRAIN_FRAMES * SCRUB_CHANNELS {
            let start = window_start(at);
            cached &= self.is_cached(rev, start);
            let w = self.window(rev, start);
            let from = frames_at(at - start).min(WINDOW_FRAMES);
            let need = GRAIN_FRAMES - out.len() / SCRUB_CHANNELS;
            let take = need.min(WINDOW_FRAMES - from);
            out.extend_from_slice(&w[from * SCRUB_CHANNELS..(from + take) * SCRUB_CHANNELS]);
            at = start + WINDOW_US;
        }
        hann(&mut out);
        self.last_t = Some(t_us);
        (out, cached)
    }

    /// Scrub direction from the previous grain to `t_us`: 1 forward, -1 backward, 0 unknown.
    pub fn direction(&self, t_us: i64) -> i8 {
        match self.last_t {
            Some(l) if t_us > l => 1,
            Some(l) if t_us < l => -1,
            _ => 0,
        }
    }

    /// Mixes the window after (direction 1) or before (-1) the one holding `t_us`, if it is not cached. Returns
    /// whether it mixed one.
    pub fn prefetch(&mut self, rev: i64, t_us: i64, direction: i8) -> bool {
        if direction == 0 {
            return false;
        }
        let next = window_start(t_us) + i64::from(direction) * WINDOW_US;
        if next < 0 || self.is_cached(rev, next) {
            return false;
        }
        self.window(rev, next);
        true
    }

    /// Drops every cached window (the project changed).
    pub fn clear(&mut self) {
        self.windows.clear();
        self.order.clear();
        self.last_t = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ramp-modulated tone whose level depends on the revision, so an edit is audible.
    fn source() -> impl FnMut(i64, i64, usize) -> Vec<f32> {
        |rev, start, frames| {
            let amp = if rev == 1 { 0.5 } else { 0.1 };
            (0..frames)
                .flat_map(|i| {
                    let t = (start as f64 / 1e6) + i as f64 / SCRUB_RATE as f64;
                    let v = (amp * (std::f64::consts::TAU * 440.0 * t).sin()) as f32;
                    [v, v]
                })
                .collect()
        }
    }

    fn max_step(v: &[f32]) -> f32 {
        v.chunks_exact(2).zip(v.chunks_exact(2).skip(1)).map(|(a, b)| (a[0] - b[0]).abs()).fold(0.0, f32::max)
    }

    #[test]
    fn grains_are_windowed_and_cached_by_revision() {
        let mut s = Scrubber::new(source());
        let (g, cached) = s.grain(1, 400_000);
        assert!(!cached);
        assert_eq!(g.len(), GRAIN_FRAMES * 2);
        assert_eq!(g[0], 0.0);
        assert!(g[g.len() - 1].abs() < 1e-6);
        let (_, cached) = s.grain(1, 410_000);
        assert!(cached, "same window");
        // A grain across a window boundary needs both windows.
        let (g2, _) = s.grain(1, 980_000);
        assert_eq!(g2.len(), GRAIN_FRAMES * 2);
        assert_eq!(s.mixed, 2);
        // An edit (new revision) misses the stale window and changes the next grain.
        let (before, _) = s.grain(1, 400_000);
        let (after, cached) = s.grain(2, 400_000);
        assert!(!cached);
        let peak = |v: &[f32]| v.iter().fold(0f32, |m, x| m.max(x.abs()));
        assert!(peak(&before) > 0.45 && peak(&after) < 0.11, "{} then {}", peak(&before), peak(&after));
    }

    #[test]
    fn prefetch_follows_the_direction() {
        let mut s = Scrubber::new(source());
        s.grain(1, 2_500_000);
        assert_eq!(s.direction(2_600_000), 1);
        assert_eq!(s.direction(2_400_000), -1);
        assert!(s.prefetch(1, 2_600_000, 1));
        assert!(s.is_cached(1, 3_000_000));
        assert!(!s.prefetch(1, 2_600_000, 1), "already cached");
        assert!(s.prefetch(1, 2_400_000, -1));
        assert!(s.is_cached(1, 1_000_000));
        assert!(!s.prefetch(1, 300_000, -1), "nothing before 0");
    }

    #[test]
    fn splicing_a_new_grain_over_queued_audio_makes_no_jump() {
        let mut s = Scrubber::new(source());
        let (a, _) = s.grain(1, 1_200_000);
        let (b, _) = s.grain(1, 1_230_000);
        // The second event arrives while a third of the first grain has played.
        let cut = GRAIN_FRAMES / 3;
        let remaining = &a[cut * 2..];
        let joined = splice(remaining, &b);
        let signal_step = max_step(&a).max(max_step(&b));
        // At the cut (last sample heard of the old grain, then the first queued one) and at the join (end of the
        // fade, then the grain's first sample), the step is no larger than the signal's own plus 1e-3.
        let at_cut = (a[(cut - 1) * 2] - joined[0]).abs();
        let k = SPLICE_FADE_FRAMES;
        let at_join = (joined[(k - 1) * 2] - joined[k * 2]).abs();
        assert!(at_cut <= signal_step + 1e-3, "cut: {at_cut} vs {signal_step}");
        assert!(at_join <= signal_step + 1e-3, "join: {at_join} vs {signal_step}");
        assert!(joined[(k - 1) * 2].abs() < 1e-3, "the fade ends at zero");
        assert_eq!(joined.len(), (SPLICE_FADE_FRAMES + GRAIN_FRAMES) * 2);
        // A splice onto an empty queue is the grain.
        assert_eq!(splice(&[], &b), b);
    }
}
