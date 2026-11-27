//! The meter tap (design D5): the mixer's output passes through it on the producer side, before the device ring.
//! Every [`READING_FRAMES`] sample frames it emits a reading per channel — sample peak, RMS, true peak (4×
//! oversampled) — plus momentary (400 ms) and short-term (3 s) loudness of the mix, tagged with the media time of
//! the frames it covers. The app sends batches of readings to the UI, which shows the one under the heard position,
//! so the meters line up with what the person hears.

use std::collections::VecDeque;

use serde::Serialize;

use crate::audio::{CHANNELS, RATE};
use crate::loudness::{Biquad, TruePeak, k_weighting, lufs};

/// Sample frames per reading (5.3 ms at 48 kHz): fine enough to place a click within one display frame.
pub const READING_FRAMES: usize = 256;
/// Sample frames per loudness sub-block (100 ms).
const SUB_FRAMES: usize = (RATE / 10) as usize;

/// Level of a linear amplitude in dB, floored at -120 dB.
pub fn amp_db(x: f64) -> f64 {
    if x <= 0.0 { -120.0 } else { (20.0 * x.log10()).max(-120.0) }
}

/// One reading of the meter tap. Levels are dB (dBFS for peak and RMS, dBTP for true peak, LUFS for loudness).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeterReading {
    /// Media time of the first and just after the last frame covered (µs).
    pub start_us: i64,
    pub end_us: i64,
    pub peak_db: [f64; 2],
    pub rms_db: [f64; 2],
    pub true_peak_db: [f64; 2],
    /// `None` until 400 ms (3 s) of audio has passed since the last restart, or when below -70 LUFS.
    pub momentary_lufs: Option<f64>,
    pub short_term_lufs: Option<f64>,
}

/// The tap's running state. Sequential blocks continue it; a jump in media time restarts the loudness windows.
pub struct MeterTap {
    filters: [[Biquad; 2]; CHANNELS],
    tp: [TruePeak; CHANNELS],
    sub_acc: f64,
    sub_n: usize,
    /// Mean-square energies of the last 30 sub-blocks (3 s).
    subs: VecDeque<f64>,
    next_us: Option<i64>,
    // The reading being built.
    r_start: i64,
    r_n: usize,
    r_peak: [f64; CHANNELS],
    r_sq: [f64; CHANNELS],
    r_tp: [f64; CHANNELS],
}

impl Default for MeterTap {
    fn default() -> Self {
        let f = k_weighting(RATE as f64);
        MeterTap {
            filters: [f, f],
            tp: [TruePeak::default(), TruePeak::default()],
            sub_acc: 0.0,
            sub_n: 0,
            subs: VecDeque::with_capacity(31),
            next_us: None,
            r_start: 0,
            r_n: 0,
            r_peak: [0.0; CHANNELS],
            r_sq: [0.0; CHANNELS],
            r_tp: [0.0; CHANNELS],
        }
    }
}

fn frame_us(frames: i64) -> i64 {
    frames * 1_000_000 / RATE as i64
}

impl MeterTap {
    pub fn reset(&mut self) {
        *self = MeterTap::default();
    }

    /// Meters interleaved stereo `block` starting at media time `t0_us`; returns the readings it completed.
    pub fn process(&mut self, t0_us: i64, block: &[f32]) -> Vec<MeterReading> {
        if self.next_us.is_some_and(|n| (n - t0_us).abs() > 2) {
            self.reset();
        }
        if self.next_us.is_none() || self.r_n == 0 {
            self.r_start = t0_us;
        }
        let mut out = Vec::new();
        let frames = block.len() / CHANNELS;
        for (i, frame) in block.chunks_exact(CHANNELS).enumerate() {
            if self.r_n == 0 {
                self.r_start = t0_us + frame_us(i as i64);
            }
            let mut weighted = 0.0;
            for (c, s) in frame.iter().enumerate() {
                let x = *s as f64;
                self.r_peak[c] = self.r_peak[c].max(x.abs());
                self.r_sq[c] += x * x;
                self.r_tp[c] = self.r_tp[c].max(self.tp[c].push(x));
                let [a, b] = &mut self.filters[c];
                let z = b.run(a.run(x));
                weighted += z * z;
            }
            self.sub_acc += weighted;
            self.sub_n += 1;
            if self.sub_n == SUB_FRAMES {
                self.subs.push_back(self.sub_acc / SUB_FRAMES as f64);
                if self.subs.len() > 30 {
                    self.subs.pop_front();
                }
                self.sub_acc = 0.0;
                self.sub_n = 0;
            }
            self.r_n += 1;
            if self.r_n == READING_FRAMES {
                out.push(self.reading(t0_us + frame_us(i as i64 + 1)));
            }
        }
        self.next_us = Some(t0_us + frame_us(frames as i64));
        out
    }

    fn window(&self, n: usize) -> Option<f64> {
        if self.subs.len() < n {
            return None;
        }
        let e = self.subs.iter().rev().take(n).sum::<f64>() / n as f64;
        (e > 0.0).then(|| lufs(e)).filter(|l| *l > -70.0)
    }

    fn reading(&mut self, end_us: i64) -> MeterReading {
        let n = self.r_n.max(1) as f64;
        let r = MeterReading {
            start_us: self.r_start,
            end_us,
            peak_db: [amp_db(self.r_peak[0]), amp_db(self.r_peak[1])],
            rms_db: [amp_db((self.r_sq[0] / n).sqrt()), amp_db((self.r_sq[1] / n).sqrt())],
            true_peak_db: [amp_db(self.r_tp[0]), amp_db(self.r_tp[1])],
            momentary_lufs: self.window(4),
            short_term_lufs: self.window(30),
        };
        self.r_n = 0;
        self.r_peak = [0.0; CHANNELS];
        self.r_sq = [0.0; CHANNELS];
        self.r_tp = [0.0; CHANNELS];
        r
    }
}

/// Readings produced ahead of the play head, waiting to be heard.
#[derive(Debug, Default)]
pub struct MeterQueue {
    readings: VecDeque<MeterReading>,
}

impl MeterQueue {
    pub fn push(&mut self, r: impl IntoIterator<Item = MeterReading>) {
        for r in r {
            // A re-mix of times already queued (after an edit) replaces the older readings.
            while self.readings.back().is_some_and(|b| b.start_us >= r.start_us) {
                self.readings.pop_back();
            }
            self.readings.push_back(r);
        }
        // Bounded: about 2.7 s of readings.
        while self.readings.len() > 512 {
            self.readings.pop_front();
        }
    }

    pub fn clear(&mut self) {
        self.readings.clear();
    }

    /// The readings from `heard_us - behind_us` to `heard_us + ahead_us` (by start time), for one batch to the UI;
    /// readings that ended before the window are dropped.
    pub fn batch(&mut self, heard_us: i64, behind_us: i64, ahead_us: i64) -> Vec<MeterReading> {
        while self.readings.front().is_some_and(|r| r.end_us < heard_us - behind_us) {
            self.readings.pop_front();
        }
        self.readings.iter().take_while(|r| r.start_us <= heard_us + ahead_us).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(amp: f64, freq: f64, frames: usize, phase0: usize) -> Vec<f32> {
        (phase0..phase0 + frames)
            .flat_map(|i| {
                let v = (amp * (2.0 * std::f64::consts::PI * freq * i as f64 / RATE as f64).sin()) as f32;
                [v, v]
            })
            .collect()
    }

    #[test]
    fn a_minus_6_dbfs_sine_reads_minus_6() {
        let amp = 10f64.powf(-6.0 / 20.0);
        let mut tap = MeterTap::default();
        let mut readings = Vec::new();
        for b in 0..200 {
            let t = frame_us(b * 1024);
            // 750 Hz: a whole number of periods per reading, so each reading sees the true RMS.
            readings.extend(tap.process(t, &sine(amp, 750.0, 1024, b as usize * 1024)));
        }
        assert_eq!(readings.len(), 200 * 1024 / READING_FRAMES);
        for r in &readings[10..] {
            for c in 0..2 {
                assert!((r.peak_db[c] + 6.0).abs() < 0.1, "peak {}", r.peak_db[c]);
                assert!((r.true_peak_db[c] + 6.0).abs() < 0.1, "true peak {}", r.true_peak_db[c]);
                assert!((r.rms_db[c] + 9.01).abs() < 0.1, "rms {}", r.rms_db[c]);
            }
        }
        // Loudness at the BS.1770 calibration frequency: a 997 Hz stereo sine at -6 dBFS is -6 - 3.01 (sine RMS)
        // + 3.01 (two channels) = -6.0 LUFS.
        let mut tap = MeterTap::default();
        let mut last = None;
        for b in 0..200 {
            last = tap.process(frame_us(b * 1024), &sine(amp, 997.0, 1024, b as usize * 1024)).pop().or(last);
        }
        let last = last.unwrap();
        let (m, s) = (last.momentary_lufs.unwrap(), last.short_term_lufs.unwrap());
        assert!((m + 6.0).abs() < 0.1 && (s + 6.0).abs() < 0.1, "momentary {m}, short-term {s}");
        // Reading times are contiguous media time.
        for w in readings.windows(2) {
            assert!((w[1].start_us - w[0].end_us).abs() <= 1);
        }
    }

    #[test]
    fn a_click_lands_in_the_reading_for_its_media_time() {
        let mut tap = MeterTap::default();
        let click_frame = 48_000 + 777;
        let mut readings = Vec::new();
        for b in 0..100i64 {
            let mut block = vec![0f32; 1024 * 2];
            let start = b as usize * 1024;
            if (start..start + 1024).contains(&click_frame) {
                block[(click_frame - start) * 2] = 1.0;
                block[(click_frame - start) * 2 + 1] = 1.0;
            }
            readings.extend(tap.process(frame_us(b * 1024), &block));
        }
        let click_us = frame_us(click_frame as i64);
        let hot: Vec<&MeterReading> = readings.iter().filter(|r| r.peak_db[0] > -1.0).collect();
        assert_eq!(hot.len(), 1);
        assert!(hot[0].start_us <= click_us && click_us < hot[0].end_us);
        assert!(hot[0].end_us - hot[0].start_us <= 5_400);
    }

    #[test]
    fn a_jump_restarts_the_loudness_windows_and_the_queue_batches_by_heard_time() {
        let mut tap = MeterTap::default();
        for b in 0..40 {
            tap.process(frame_us(b * 1024), &sine(0.5, 440.0, 1024, b as usize * 1024));
        }
        let r = tap.process(frame_us(10_000_000), &sine(0.5, 440.0, 1024, 0));
        assert!(r.iter().all(|r| r.momentary_lufs.is_none()), "restarted after a seek");
        let mut q = MeterQueue::default();
        q.push((0..20).map(|i| MeterReading {
            start_us: i * 5_000,
            end_us: (i + 1) * 5_000,
            peak_db: [-6.0; 2],
            rms_db: [-9.0; 2],
            true_peak_db: [-6.0; 2],
            momentary_lufs: None,
            short_term_lufs: None,
        }));
        let b = q.batch(50_000, 10_000, 20_000);
        assert_eq!(b.first().unwrap().start_us, 35_000); // Ends at 40 ms: still inside the window.
        assert_eq!(b.last().unwrap().start_us, 70_000);
    }
}
