//! Audio ring shared between the engine (producer) and the device callback (consumer), with the clock bookkeeping
//! that makes audio the master clock.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::clock::{AudioPosition, Micros};

#[derive(Debug)]
struct Ring {
    buf: VecDeque<f32>,
    /// Media time of the front sample.
    front_us: Option<Micros>,
    rate: u32,
    channels: u16,
    volume: f32,
    muted: bool,
    paused: bool,
    latency_us: Micros,
    position: Option<AudioPosition>,
    underruns: u64,
}

/// Cloneable handle; all clones share one ring.
#[derive(Debug, Clone)]
pub struct AudioRing(Arc<Mutex<Ring>>);

impl AudioRing {
    pub fn new(rate: u32, channels: u16) -> Self {
        AudioRing(Arc::new(Mutex::new(Ring {
            buf: VecDeque::new(),
            front_us: None,
            rate,
            channels,
            volume: 1.0,
            muted: false,
            paused: true,
            latency_us: 0,
            position: None,
            underruns: 0,
        })))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Ring> {
        // A panic while holding the lock cannot leave the ring inconsistent: recover the data.
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn format(&self) -> (u32, u16) {
        let r = self.lock();
        (r.rate, r.channels)
    }

    /// Device format changed (new output device): drop queued audio; the producer must re-decode in the new format.
    pub fn set_format(&self, rate: u32, channels: u16) {
        let mut r = self.lock();
        r.rate = rate;
        r.channels = channels;
        r.buf.clear();
        r.front_us = None;
        r.position = None;
    }

    // --- producer side -------------------------------------------------------------------------------------------

    /// Append interleaved samples starting at media time `pts_us`. A discontinuity re-bases the ring.
    pub fn push(&self, pts_us: Micros, samples: &[f32]) {
        let mut r = self.lock();
        if r.front_us.is_none() || r.buf.is_empty() {
            r.front_us = Some(pts_us);
        }
        r.buf.extend(samples.iter().copied());
    }

    pub fn clear(&self) {
        let mut r = self.lock();
        r.buf.clear();
        r.front_us = None;
        r.position = None;
    }

    /// Queued duration (µs).
    pub fn queued_us(&self) -> Micros {
        let r = self.lock();
        (r.buf.len() / r.channels.max(1) as usize) as Micros * 1_000_000 / r.rate.max(1) as Micros
    }

    /// Media time just after the last queued sample (where the producer continues).
    pub fn end_us(&self) -> Option<Micros> {
        let r = self.lock();
        r.front_us.map(|f| {
            f + (r.buf.len() / r.channels.max(1) as usize) as Micros * 1_000_000
                / r.rate.max(1) as Micros
        })
    }

    pub fn set_volume(&self, v: f32) {
        self.lock().volume = v.clamp(0.0, 2.0);
    }

    pub fn set_muted(&self, m: bool) {
        self.lock().muted = m;
    }

    /// Paused: the callback outputs silence and does not consume (clock stops).
    pub fn set_paused(&self, p: bool) {
        let mut r = self.lock();
        r.paused = p;
        r.position = None;
    }

    pub fn position(&self) -> Option<AudioPosition> {
        self.lock().position
    }

    pub fn underruns(&self) -> u64 {
        self.lock().underruns
    }

    // --- consumer side (device callback) -------------------------------------------------------------------------

    /// Fill `out` (interleaved, device format) at wall time `wall_us`, with `latency_us` until it is heard.
    pub fn fill(&self, out: &mut [f32], wall_us: Micros, latency_us: Micros) {
        let mut r = self.lock();
        r.latency_us = latency_us;
        if r.paused {
            out.fill(0.0);
            return;
        }
        let Some(front) = r.front_us else {
            out.fill(0.0);
            return;
        };
        r.position = Some(AudioPosition {
            media_us: front,
            at_wall_us: wall_us,
            latency_us,
        });
        let gain = if r.muted { 0.0 } else { r.volume };
        let n = out.len().min(r.buf.len());
        for (o, s) in out.iter_mut().zip(r.buf.drain(..n)) {
            *o = s * gain;
        }
        if n < out.len() {
            r.underruns += 1;
            out[n..].fill(0.0);
        }
        let frames = (n / r.channels.max(1) as usize) as Micros;
        let rate = r.rate.max(1) as Micros;
        r.front_us = Some(front + frames * 1_000_000 / rate);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_consumes_and_reports_position() {
        let ring = AudioRing::new(1000, 1); // 1 kHz mono: one sample = 1 ms
        ring.push(500_000, &[0.5; 100]);
        let mut out = [1.0f32; 10];
        ring.fill(&mut out, 0, 0);
        assert_eq!(out, [0.0; 10], "paused: silence");
        assert!(ring.position().is_none());
        ring.set_paused(false);
        ring.fill(&mut out, 7_000, 20_000);
        assert_eq!(out, [0.5; 10]);
        let p = ring.position().unwrap();
        assert_eq!(
            (p.media_us, p.at_wall_us, p.latency_us),
            (500_000, 7_000, 20_000)
        );
        assert_eq!(ring.end_us(), Some(600_000));
        assert_eq!(ring.queued_us(), 90_000);
        ring.fill(&mut out, 17_000, 20_000);
        assert_eq!(ring.position().unwrap().media_us, 510_000);
    }

    #[test]
    fn volume_mute_underrun() {
        let ring = AudioRing::new(1000, 2);
        ring.set_paused(false);
        ring.push(0, &[1.0; 8]);
        ring.set_volume(0.5);
        let mut out = [9.0f32; 4];
        ring.fill(&mut out, 0, 0);
        assert_eq!(out, [0.5; 4]);
        ring.set_muted(true);
        ring.fill(&mut out, 0, 0);
        assert_eq!(out, [0.0; 4], "muted still consumes (clock keeps running)");
        assert_eq!(ring.queued_us(), 0);
        ring.fill(&mut out, 0, 0);
        assert_eq!(ring.underruns(), 1);
    }

    #[test]
    fn clear_and_format_change() {
        let ring = AudioRing::new(48_000, 2);
        ring.push(0, &[0.1; 960]);
        ring.clear();
        assert_eq!(ring.queued_us(), 0);
        assert_eq!(ring.end_us(), None);
        ring.push(1_000_000, &[0.1; 960]);
        ring.set_format(44_100, 2);
        assert_eq!(ring.format(), (44_100, 2));
        assert_eq!(ring.queued_us(), 0);
    }
}
