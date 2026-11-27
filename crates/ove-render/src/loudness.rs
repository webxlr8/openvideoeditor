//! The ITU-R BS.1770-4 building blocks shared by the meters (playback), the analysis pass (`ove-analyze`) and, later,
//! loudness-normalised export: the K-weighting filter, the 4× oversampled true-peak estimator, gating and loudness
//! range. One implementation, so what the meters show, what analysis reports and what export targets agree.

/// Loudness of a mean-square energy (LUFS).
pub fn lufs(e: f64) -> f64 {
    -0.691 + 10.0 * e.log10()
}

/// One biquad section, transposed direct form II.
#[derive(Debug, Clone, Copy, Default)]
pub struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl Biquad {
    pub fn run(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }

    pub fn reset(&mut self) {
        self.z = [0.0; 2];
    }
}

/// The two K-weighting stages (high shelf, then high pass) of BS.1770, designed for sample rate `fs`.
pub fn k_weighting(fs: f64) -> [Biquad; 2] {
    use std::f64::consts::PI;
    // Stage 1: high shelf, +4 dB above ~1.7 kHz.
    let (f0, g, q) = (1_681.974_450_955_533, 3.999_843_853_973_347, 0.707_175_236_955_419_6);
    let k = (PI * f0 / fs).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.499_666_774_154_541_6);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b: [(vh + vb * k / q + k * k) / a0, 2.0 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        z: [0.0; 2],
    };
    // Stage 2: high pass at ~38 Hz.
    let (f0, q) = (38.135_470_876_024_44, 0.500_327_037_323_877_3);
    let k = (PI * f0 / fs).tan();
    let a0 = 1.0 + k / q + k * k;
    let hp = Biquad { b: [1.0, -2.0, 1.0], a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0], z: [0.0; 2] };
    [shelf, hp]
}

/// BS.1770 channel weights for the default FFmpeg layouts (LFE excluded, surrounds +1.5 dB).
pub fn channel_weights(ch: usize) -> Vec<f64> {
    match ch {
        6 => vec![1.0, 1.0, 1.0, 0.0, 1.41, 1.41],
        8 => vec![1.0, 1.0, 1.0, 0.0, 1.41, 1.41, 1.41, 1.41],
        n => vec![1.0; n],
    }
}

/// Taps per phase of the 4× true-peak interpolator.
pub const TP_TAPS: usize = 12;

/// 4× oversampling interpolator for true peak: 12-tap Hann-windowed sinc per phase (the three phases between two
/// input samples).
pub fn tp_kernel() -> [[f64; TP_TAPS]; 3] {
    use std::f64::consts::PI;
    let mut k = [[0.0; TP_TAPS]; 3];
    for (p, row) in k.iter_mut().enumerate() {
        let frac = (p + 1) as f64 / 4.0;
        for (j, v) in row.iter_mut().enumerate() {
            // Distance from the interpolated point (between taps 5 and 6) to tap j.
            let d = 5.0 + frac - j as f64;
            let sinc = if d.abs() < 1e-12 { 1.0 } else { (PI * d).sin() / (PI * d) };
            let w = if d.abs() < 6.5 { 0.5 * (1.0 + (PI * d / 6.5).cos()) } else { 0.0 };
            *v = sinc * w;
        }
    }
    k
}

/// Streaming true-peak estimate of one channel.
#[derive(Debug, Clone)]
pub struct TruePeak {
    history: [f64; TP_TAPS],
    kernel: [[f64; TP_TAPS]; 3],
}

impl Default for TruePeak {
    fn default() -> Self {
        TruePeak { history: [0.0; TP_TAPS], kernel: tp_kernel() }
    }
}

impl TruePeak {
    /// Adds one sample; returns the largest absolute value among the sample and the three interpolated points before
    /// the middle of the history.
    pub fn push(&mut self, x: f64) -> f64 {
        let h = &mut self.history;
        h.copy_within(1.., 0);
        h[TP_TAPS - 1] = x;
        let mut m = x.abs();
        for phase in &self.kernel {
            let y: f64 = phase.iter().zip(h.iter()).map(|(k, v)| k * v).sum();
            m = m.max(y.abs());
        }
        m
    }

    pub fn reset(&mut self) {
        self.history = [0.0; TP_TAPS];
    }
}

/// Energies of windows of `len` sub-blocks stepped by one sub-block (100 ms).
pub fn window_energies(subs: &[f64], len: usize) -> Vec<f64> {
    if subs.len() < len || len == 0 {
        return vec![];
    }
    let mut out = Vec::with_capacity(subs.len() - len + 1);
    let mut acc: f64 = subs[..len].iter().sum();
    out.push(acc / len as f64);
    for i in len..subs.len() {
        acc += subs[i] - subs[i - len];
        out.push(acc.max(0.0) / len as f64);
    }
    out
}

/// Integrated loudness of momentary-block energies (absolute −70 LUFS gate, then relative −10 LU gate).
pub fn integrated(blocks: &[f64]) -> Option<f64> {
    let abs: Vec<f64> = blocks.iter().copied().filter(|e| *e > 0.0 && lufs(*e) > -70.0).collect();
    if abs.is_empty() {
        return None;
    }
    let gate = lufs(abs.iter().sum::<f64>() / abs.len() as f64) - 10.0;
    let rel: Vec<f64> = abs.into_iter().filter(|e| lufs(*e) > gate).collect();
    (!rel.is_empty()).then(|| lufs(rel.iter().sum::<f64>() / rel.len() as f64))
}

/// Loudness range of short-term energies (absolute −70 LUFS, relative −20 LU gate; 95th − 10th percentile).
pub fn loudness_range(short: &[f64]) -> f64 {
    let abs: Vec<f64> = short.iter().copied().filter(|e| *e > 0.0 && lufs(*e) > -70.0).collect();
    if abs.is_empty() {
        return 0.0;
    }
    let gate = lufs(abs.iter().sum::<f64>() / abs.len() as f64) - 20.0;
    let mut l: Vec<f64> = abs.into_iter().map(lufs).filter(|v| *v > gate).collect();
    if l.len() < 2 {
        return 0.0;
    }
    l.sort_by(f64::total_cmp);
    let pct = |p: f64| {
        let x = p * (l.len() - 1) as f64;
        let (i, f) = (x.floor() as usize, x.fract());
        l[i] + (l[(i + 1).min(l.len() - 1)] - l[i]) * f
    };
    pct(0.95) - pct(0.10)
}

/// Integrated loudness, loudness range and true peak of interleaved stereo at 48 kHz, fed block by block (export's
/// measuring pass and `ove audio loudness --timeline`).
pub struct StereoLoudness {
    filters: [[Biquad; 2]; 2],
    tp: [TruePeak; 2],
    sub_acc: f64,
    sub_n: usize,
    subs: Vec<f64>,
    true_peak: f64,
    frames: u64,
}

/// What [`StereoLoudness`] measured.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoudnessSummary {
    /// `None` when every block is below the −70 LUFS gate (silence).
    pub integrated_lufs: Option<f64>,
    pub loudness_range_lu: f64,
    pub true_peak_dbtp: f64,
    pub duration: f64,
}

impl Default for StereoLoudness {
    fn default() -> Self {
        let f = k_weighting(48_000.0);
        StereoLoudness {
            filters: [f, f],
            tp: [TruePeak::default(), TruePeak::default()],
            sub_acc: 0.0,
            sub_n: 0,
            subs: Vec::new(),
            true_peak: 0.0,
            frames: 0,
        }
    }
}

impl StereoLoudness {
    pub fn push(&mut self, interleaved: &[f32]) {
        for f in interleaved.chunks_exact(2) {
            let mut e = 0.0;
            for (c, s) in f.iter().enumerate() {
                let x = f64::from(*s);
                self.true_peak = self.true_peak.max(self.tp[c].push(x));
                let [a, b] = &mut self.filters[c];
                let z = b.run(a.run(x));
                e += z * z;
            }
            self.sub_acc += e;
            self.sub_n += 1;
            if self.sub_n == 4_800 {
                self.subs.push(self.sub_acc / 4_800.0);
                self.sub_acc = 0.0;
                self.sub_n = 0;
            }
            self.frames += 1;
        }
    }

    pub fn summary(&self) -> LoudnessSummary {
        let r2 = |v: f64| (v * 100.0).round() / 100.0;
        LoudnessSummary {
            integrated_lufs: integrated(&window_energies(&self.subs, 4)).map(r2),
            loudness_range_lu: r2(loudness_range(&window_energies(&self.subs, 30))),
            true_peak_dbtp: r2(if self.true_peak <= 0.0 {
                -120.0
            } else {
                (20.0 * self.true_peak.log10()).max(-120.0)
            }),
            duration: self.frames as f64 / 48_000.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn k_weighting_reads_997hz_full_scale_mono_as_minus_3() {
        let mut f = k_weighting(48_000.0);
        let mut sum = 0.0;
        let n = 48_000 * 5;
        for i in 0..n {
            let x = (2.0 * std::f64::consts::PI * 997.0 * i as f64 / 48_000.0).sin();
            let [a, b] = &mut f;
            let z = b.run(a.run(x));
            if i >= 48_000 {
                sum += z * z;
            }
        }
        let l = lufs(sum / (n - 48_000) as f64);
        assert!((l + 3.01).abs() < 0.05, "{l}");
    }

    #[test]
    fn true_peak_sees_intersample_overs() {
        let mut tp = TruePeak::default();
        let mut m: f64 = 0.0;
        for i in 0..4_800 {
            let v = (std::f64::consts::PI / 2.0 * i as f64 + std::f64::consts::PI / 4.0).sin();
            m = m.max(tp.push(v));
        }
        assert!(20.0 * m.log10() > -0.5, "{m}");
    }
}
