//! Synthetic recordings for sync tests: speech-like bursts heard by several "microphones", each with its own pink
//! noise, colouring, start time and clock rate. A recording can be read at any sample position, so a two-hour
//! fixture never has to sit in memory or on disk.
//!
//! The *scene* is the sound in the room: bursts of three partials (300 Hz – 3.5 kHz) of 80–160 ms in 250 ms slots,
//! grouped into phrases with pauses. A *recording* hears the scene from scene time `offset_s` on, with its clock
//! running `clock` times as fast as real time (1.00005 is a recorder 50 ppm fast).

use crate::sync::{RATE, Source};

fn hash(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn unit(seed: u64, a: u64, b: u64) -> f64 {
    (hash(seed ^ hash(a.wrapping_mul(31).wrapping_add(b))) >> 11) as f64 / (1u64 << 53) as f64
}

/// The sound in the room.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scene {
    pub seed: u64,
    /// Repeat the first `period` seconds forever (looped music).
    pub period: Option<f64>,
}

const SLOT: f64 = 0.25;
const PHRASE: f64 = 3.0;

struct Burst {
    start: f64,
    len: f64,
    partials: [(f64, f64, f64); 3],
}

impl Scene {
    pub fn speech(seed: u64) -> Scene {
        Scene { seed, period: None }
    }

    pub fn looped(seed: u64, period: f64) -> Scene {
        Scene { seed, period: Some(period) }
    }

    fn burst(&self, slot: i64) -> Option<Burst> {
        if slot < 0 {
            return None;
        }
        let s = slot as u64;
        let phrase = ((slot as f64 * SLOT) / PHRASE) as u64;
        if hash(self.seed ^ phrase.wrapping_mul(0x5851_F42D)).is_multiple_of(4) || unit(self.seed, s, 1) > 0.65 {
            return None;
        }
        let start = slot as f64 * SLOT + 0.08 * unit(self.seed, s, 2);
        let len = 0.08 + 0.08 * unit(self.seed, s, 3);
        let p = |k: u64| {
            (300.0 + 3_200.0 * unit(self.seed, s, 10 + k), 0.3, std::f64::consts::TAU * unit(self.seed, s, 20 + k))
        };
        Some(Burst { start, len, partials: [p(0), p(1), p(2)] })
    }

    /// Adds the scene's sound to `out`, where `out[i]` is heard at scene time `t0 + i · dt`.
    fn render(&self, t0: f64, dt: f64, out: &mut [f64]) {
        if out.is_empty() {
            return;
        }
        match self.period {
            Some(period) => {
                // Split where the loop wraps, and render each piece from the loop's start.
                let mut i = 0usize;
                while i < out.len() {
                    let t = t0 + i as f64 * dt;
                    let k = t.div_euclid(period);
                    let local = t - k * period;
                    let left = ((period - local) / dt).ceil().max(1.0) as usize;
                    let n = left.min(out.len() - i);
                    self.render_plain(local, dt, &mut out[i..i + n]);
                    i += n;
                }
            }
            None => self.render_plain(t0, dt, out),
        }
    }

    fn render_plain(&self, t0: f64, dt: f64, out: &mut [f64]) {
        let t1 = t0 + out.len() as f64 * dt;
        let first = ((t0 - 0.5) / SLOT).floor() as i64;
        let last = (t1 / SLOT).ceil() as i64;
        for slot in first..=last {
            let Some(b) = self.burst(slot) else { continue };
            if let Some(period) = self.period
                && b.start + b.len > period
            {
                continue;
            }
            let i0 = (((b.start - t0) / dt).ceil().max(0.0)) as usize;
            let i1 = ((((b.start + b.len) - t0) / dt).ceil().max(0.0) as usize).min(out.len());
            if i0 >= i1 {
                continue;
            }
            let ramp = 0.01;
            for (f, a, ph) in b.partials {
                let t = t0 + i0 as f64 * dt - b.start;
                let w = std::f64::consts::TAU * f * dt;
                let (mut re, mut im) =
                    ((std::f64::consts::TAU * f * t + ph).cos(), (std::f64::consts::TAU * f * t + ph).sin());
                let (cw, sw) = (w.cos(), w.sin());
                for (k, o) in out[i0..i1].iter_mut().enumerate() {
                    let tt = t + k as f64 * dt;
                    let env = (tt / ramp).min((b.len - tt) / (2.0 * ramp)).clamp(0.0, 1.0);
                    *o += a * env * im;
                    let nre = re * cw - im * sw;
                    im = re * sw + im * cw;
                    re = nre;
                }
            }
        }
    }
}

/// One microphone's recording of a scene.
#[derive(Debug, Clone, PartialEq)]
pub struct Recording {
    pub scene: Scene,
    /// Scene time at the recording's first sample.
    pub offset_s: f64,
    /// Recording seconds per real second (1.00005: the recorder's clock runs 50 ppm fast).
    pub clock: f64,
    /// Speech-to-noise ratio, dB (`None`: no noise).
    pub snr_db: Option<f64>,
    /// One-pole low-pass on the recording, Hz.
    pub lowpass_hz: Option<f64>,
    pub noise_seed: u64,
    /// Length in samples at [`RATE`].
    pub len: u64,
    noise_gain: f64,
}

/// Speech RMS of the scene over its active time, measured once.
const SPEECH_RMS: f64 = 0.36;
/// Samples rendered before a read so the noise and low-pass filters settle.
const PREROLL: usize = 4_800;

impl Recording {
    pub fn new(scene: Scene, offset_s: f64, seconds: f64) -> Recording {
        Recording {
            scene,
            offset_s,
            clock: 1.0,
            snr_db: None,
            lowpass_hz: None,
            noise_seed: 1,
            len: (seconds * RATE as f64) as u64,
            noise_gain: 0.0,
        }
    }

    pub fn clock(mut self, clock: f64) -> Recording {
        self.clock = clock;
        self
    }

    pub fn lowpass(mut self, hz: f64) -> Recording {
        self.lowpass_hz = Some(hz);
        self
    }

    pub fn noise(mut self, snr_db: f64, seed: u64) -> Recording {
        self.snr_db = Some(snr_db);
        self.noise_seed = seed;
        // Scale the pink noise to the speech level.
        let mut probe = vec![0.0f64; RATE as usize];
        self.noise_gain = 1.0;
        self.pink(0, &mut probe);
        let rms = (probe.iter().map(|v| v * v).sum::<f64>() / probe.len() as f64).sqrt();
        self.noise_gain = SPEECH_RMS / 10f64.powf(snr_db / 20.0) / rms.max(1e-12);
        self
    }

    /// Pink noise (economy three-pole filter over hashed white noise) for samples `start..`, settled by a pre-roll.
    fn pink(&self, start: i64, out: &mut [f64]) {
        let (mut b0, mut b1, mut b2) = (0.0, 0.0, 0.0);
        let from = start - PREROLL as i64;
        for n in from..start + out.len() as i64 {
            let w = (hash(self.noise_seed ^ hash(n as u64)) >> 11) as f64 / (1u64 << 52) as f64 - 1.0;
            b0 = 0.99765 * b0 + w * 0.099_046;
            b1 = 0.963 * b1 + w * 0.296_516_4;
            b2 = 0.57 * b2 + w * 1.052_691_3;
            if n >= start {
                out[(n - start) as usize] = (b0 + b1 + b2 + w * 0.1848) * self.noise_gain;
            }
        }
    }

    /// Samples `start..start + out.len()` (zero outside the recording).
    pub fn render(&self, start: i64, out: &mut [f32]) {
        let pre = if self.lowpass_hz.is_some() { PREROLL } else { 0 };
        let mut buf = vec![0.0f64; out.len() + pre];
        let first = start - pre as i64;
        let dt = 1.0 / (RATE as f64 * self.clock);
        self.scene.render(self.offset_s + first as f64 * dt, dt, &mut buf);
        if let Some(hz) = self.lowpass_hz {
            let a = 1.0 - (-std::f64::consts::TAU * hz / RATE as f64).exp();
            let mut y = 0.0;
            for v in buf.iter_mut() {
                y += a * (*v - y);
                *v = y;
            }
        }
        if self.snr_db.is_some() {
            let mut n = vec![0.0f64; buf.len()];
            self.pink(first, &mut n);
            buf.iter_mut().zip(&n).for_each(|(v, x)| *v += x);
        }
        for (i, o) in out.iter_mut().enumerate() {
            let n = start + i as i64;
            *o = if n >= 0 && (n as u64) < self.len { buf[i + pre] as f32 } else { 0.0 };
        }
    }

    /// The onset envelope the analysis cache would hold for this recording.
    pub fn envelope(&self) -> Vec<u8> {
        let mut env = crate::analysis::OnsetEnvelope::default();
        let chunk = RATE as usize * 4;
        let mut buf = vec![0.0f32; chunk];
        let mut at = 0u64;
        while at < self.len {
            let n = chunk.min((self.len - at) as usize);
            self.render(at as i64, &mut buf[..n]);
            buf[..n].iter().for_each(|v| env.push(*v));
            at += n as u64;
        }
        env.finish()
    }

    /// The recording as 16-bit mono PCM WAV bytes.
    pub fn wav(&self) -> Vec<u8> {
        let mut pcm = vec![0.0f32; self.len as usize];
        self.render(0, &mut pcm);
        let data: Vec<u8> =
            pcm.iter().flat_map(|v| ((v.clamp(-1.0, 1.0) * 32_767.0).round() as i16).to_le_bytes()).collect();
        let mut out = Vec::with_capacity(data.len() + 44);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&RATE.to_le_bytes());
        out.extend_from_slice(&(RATE * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend(data);
        out
    }
}

impl Source for Recording {
    fn read(&mut self, start: i64, out: &mut [f32]) -> crate::Result<()> {
        self.render(start, out);
        Ok(())
    }
}
