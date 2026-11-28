//! Min/max peak pyramid of a media file's audio (design D2): per channel (mono, or the first two channels), the base
//! level holds the lowest and highest sample of every 256 sample frames at 48 kHz, and each next level halves the bin
//! count. Values are 8-bit: the minimum rounds down and the maximum rounds up, so the drawn envelope always contains
//! the signal. Range queries read the coarsest level that is still at least as fine as asked.

use std::io::{self, Read, Write};

/// Sample frames per bin of the base level.
pub const BASE_BIN: u64 = 256;
/// Sample rate the pyramid's frame positions refer to.
pub const PEAK_RATE: u32 = 48_000;
const MAGIC: &[u8; 8] = b"OVEPEAK\0";
const FORMAT: u32 = 1;

/// Quantises a sample down (for minima) to -127..=127.
pub fn q_min(v: f32) -> i8 {
    (v.clamp(-1.0, 1.0) * 127.0).floor() as i8
}

/// Quantises a sample up (for maxima) to -127..=127.
pub fn q_max(v: f32) -> i8 {
    (v.clamp(-1.0, 1.0) * 127.0).ceil() as i8
}

/// The pyramid. Each level is laid out bin by bin, then channel by channel, as `[min, max]` pairs.
#[derive(Debug, Clone, PartialEq)]
pub struct PeakPyramid {
    pub channels: usize,
    /// Sample frames analysed (48 kHz).
    pub frames: u64,
    levels: Vec<Vec<i8>>,
}

/// Peaks for a source range: `bins` columns of `channels` `[min, max]` pairs, from pyramid level `level` (bin size
/// `BASE_BIN << level` frames).
#[derive(Debug, Clone, PartialEq)]
pub struct PeakRange {
    pub level: usize,
    pub channels: usize,
    pub bins: usize,
    /// `bins × channels × 2` values, -127..=127 (divide by 127 for sample units).
    pub data: Vec<i8>,
}

/// Builds the base level from interleaved frames, one frame at a time.
pub struct PeakBuilder {
    channels: usize,
    frames: u64,
    acc: Vec<(f32, f32)>,
    in_bin: u64,
    base: Vec<i8>,
}

impl PeakBuilder {
    /// `channels` is clamped to 1..=2: the pyramid keeps mono, or left and right.
    pub fn new(channels: usize) -> PeakBuilder {
        let channels = channels.clamp(1, 2);
        PeakBuilder { channels, frames: 0, acc: vec![(f32::MAX, f32::MIN); channels], in_bin: 0, base: Vec::new() }
    }

    /// Adds one frame of the source (any channel count; channels after the second are ignored).
    pub fn push_frame(&mut self, frame: &[f32]) {
        for (c, acc) in self.acc.iter_mut().enumerate() {
            let v = frame.get(c).or(frame.first()).copied().unwrap_or(0.0);
            acc.0 = acc.0.min(v);
            acc.1 = acc.1.max(v);
        }
        self.frames += 1;
        self.in_bin += 1;
        if self.in_bin == BASE_BIN {
            self.close_bin();
        }
    }

    fn close_bin(&mut self) {
        if self.in_bin == 0 {
            return;
        }
        for acc in &mut self.acc {
            self.base.push(q_min(acc.0));
            self.base.push(q_max(acc.1));
            *acc = (f32::MAX, f32::MIN);
        }
        self.in_bin = 0;
    }

    /// The finished pyramid.
    pub fn finish(mut self) -> PeakPyramid {
        self.close_bin();
        let ch = self.channels;
        let mut levels = vec![self.base];
        while levels.last().is_some_and(|l| l.len() > ch * 2) {
            let prev = levels.last().map(Vec::as_slice).unwrap_or_default();
            let n = prev.len() / (ch * 2);
            let mut next = Vec::with_capacity(n.div_ceil(2) * ch * 2);
            for b in (0..n).step_by(2) {
                for c in 0..ch {
                    let i = (b * ch + c) * 2;
                    let (mut lo, mut hi) = (prev[i], prev[i + 1]);
                    if b + 1 < n {
                        let j = ((b + 1) * ch + c) * 2;
                        lo = lo.min(prev[j]);
                        hi = hi.max(prev[j + 1]);
                    }
                    next.push(lo);
                    next.push(hi);
                }
            }
            levels.push(next);
        }
        PeakPyramid { channels: ch, frames: self.frames, levels }
    }
}

impl PeakPyramid {
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// Bins in a level.
    pub fn bins(&self, level: usize) -> usize {
        self.levels.get(level).map_or(0, |l| l.len() / (self.channels * 2))
    }

    /// Bytes of peak data in all levels.
    pub fn data_bytes(&self) -> usize {
        self.levels.iter().map(Vec::len).sum()
    }

    /// The coarsest level whose bins are no wider than `frames_per_bin`.
    pub fn level_for(&self, frames_per_bin: f64) -> usize {
        let mut level = 0;
        while level + 1 < self.levels.len() && (BASE_BIN << (level + 1)) as f64 <= frames_per_bin {
            level += 1;
        }
        level
    }

    /// Peaks of source frames `start..end` in `bins` equal columns. Each column joins every bin of the chosen level
    /// that overlaps it; columns past the analysed audio are silent (0, 0).
    pub fn range(&self, start: f64, end: f64, bins: usize) -> PeakRange {
        let ch = self.channels;
        let bins = bins.max(1);
        let span = ((end - start) / bins as f64).max(0.0);
        let level = self.level_for(span);
        let size = (BASE_BIN << level) as f64;
        let lv = &self.levels[level];
        let n = lv.len() / (ch * 2);
        let mut data = vec![0i8; bins * ch * 2];
        for col in 0..bins {
            let a = start + col as f64 * span;
            let b = a + span;
            let lo = (a / size).floor().max(0.0) as usize;
            let hi = ((b / size).ceil() as usize).max(lo + 1).min(n);
            if lo >= hi {
                continue;
            }
            for c in 0..ch {
                let (mut mn, mut mx) = (i8::MAX, i8::MIN);
                for bin in lo..hi {
                    let i = (bin * ch + c) * 2;
                    mn = mn.min(lv[i]);
                    mx = mx.max(lv[i + 1]);
                }
                data[(col * ch + c) * 2] = mn;
                data[(col * ch + c) * 2 + 1] = mx;
            }
        }
        PeakRange { level, channels: ch, bins, data }
    }

    /// Serialises the pyramid (little endian): magic, format, channels, frames, level count, then each level's length
    /// and values.
    pub fn write_to(&self, w: &mut impl Write) -> io::Result<()> {
        w.write_all(MAGIC)?;
        w.write_all(&FORMAT.to_le_bytes())?;
        w.write_all(&(self.channels as u32).to_le_bytes())?;
        w.write_all(&self.frames.to_le_bytes())?;
        w.write_all(&(self.levels.len() as u32).to_le_bytes())?;
        for l in &self.levels {
            w.write_all(&(l.len() as u64).to_le_bytes())?;
            let bytes: Vec<u8> = l.iter().map(|v| *v as u8).collect();
            w.write_all(&bytes)?;
        }
        Ok(())
    }

    /// Reads a pyramid written by [`PeakPyramid::write_to`].
    pub fn read_from(r: &mut impl Read) -> io::Result<PeakPyramid> {
        let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
        let mut magic = [0u8; 8];
        r.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(bad("not a peak file"));
        }
        let mut u32b = [0u8; 4];
        let mut u64b = [0u8; 8];
        r.read_exact(&mut u32b)?;
        if u32::from_le_bytes(u32b) != FORMAT {
            return Err(bad("unknown peak file format"));
        }
        r.read_exact(&mut u32b)?;
        let channels = u32::from_le_bytes(u32b) as usize;
        if !(1..=2).contains(&channels) {
            return Err(bad("bad channel count"));
        }
        r.read_exact(&mut u64b)?;
        let frames = u64::from_le_bytes(u64b);
        r.read_exact(&mut u32b)?;
        let count = u32::from_le_bytes(u32b) as usize;
        if count == 0 || count > 64 {
            return Err(bad("bad level count"));
        }
        let mut levels = Vec::with_capacity(count);
        for _ in 0..count {
            r.read_exact(&mut u64b)?;
            let len = u64::from_le_bytes(u64b) as usize;
            if !len.is_multiple_of(channels * 2) || len > 1 << 34 {
                return Err(bad("bad level length"));
            }
            let mut bytes = vec![0u8; len];
            r.read_exact(&mut bytes)?;
            levels.push(bytes.into_iter().map(|b| b as i8).collect());
        }
        Ok(PeakPyramid { channels, frames, levels })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic stereo signal with a varying envelope.
    fn signal(frames: usize) -> Vec<[f32; 2]> {
        (0..frames)
            .map(|i| {
                let t = i as f32 / PEAK_RATE as f32;
                let env = 0.5 + 0.5 * (t * 0.7).sin();
                let l = env * (t * 440.0 * std::f32::consts::TAU).sin();
                let r = 0.6 * env * (t * 97.0 * std::f32::consts::TAU).cos();
                [l, r]
            })
            .collect()
    }

    fn build(sig: &[[f32; 2]]) -> PeakPyramid {
        let mut b = PeakBuilder::new(2);
        for f in sig {
            b.push_frame(f);
        }
        b.finish()
    }

    /// Brute force: the min/max of the raw samples over the same aligned bins the query reads.
    fn reference(sig: &[[f32; 2]], start: f64, end: f64, bins: usize, level: usize) -> Vec<i8> {
        let size = (BASE_BIN << level) as f64;
        let span = (end - start) / bins as f64;
        let mut out = vec![0i8; bins * 4];
        for col in 0..bins {
            let a = start + col as f64 * span;
            let lo = ((a / size).floor().max(0.0) * size) as usize;
            let hi_bin = (((a + span) / size).ceil() as usize).max((a / size).floor() as usize + 1);
            let hi = (hi_bin as f64 * size) as usize;
            let s = &sig[lo.min(sig.len())..hi.min(sig.len())];
            if s.is_empty() {
                continue;
            }
            for c in 0..2 {
                let mn = s.iter().map(|f| f[c]).fold(f32::MAX, f32::min);
                let mx = s.iter().map(|f| f[c]).fold(f32::MIN, f32::max);
                out[(col * 2 + c) * 2] = q_min(mn);
                out[(col * 2 + c) * 2 + 1] = q_max(mx);
            }
        }
        out
    }

    #[test]
    fn range_matches_brute_force_at_every_zoom() {
        let sig = signal(PEAK_RATE as usize * 7 + 123);
        let p = build(&sig);
        assert_eq!(p.frames, sig.len() as u64);
        assert_eq!(p.bins(0), sig.len().div_ceil(256));
        // Zoomed in (finer than the base level), several middle zooms, and the whole file in a few columns.
        for (start, end, bins) in [
            (1000.0, 5000.0, 400),
            (0.0, sig.len() as f64, 800),
            (48_000.0, 96_000.0, 375),
            (12_345.0, 300_000.0, 97),
            (0.0, sig.len() as f64, 3),
            (300_000.0, 400_000.0, 1),
        ] {
            let r = p.range(start, end, bins);
            let want = reference(&sig, start, end, bins, r.level);
            assert_eq!(r.data, want, "range {start}..{end} in {bins} bins (level {})", r.level);
        }
    }

    #[test]
    fn picks_the_coarsest_sufficient_level() {
        let p = build(&signal(PEAK_RATE as usize * 60));
        assert_eq!(p.level_for(10.0), 0);
        assert_eq!(p.level_for(255.0), 0);
        assert_eq!(p.level_for(256.0), 0);
        assert_eq!(p.level_for(511.0), 0);
        assert_eq!(p.level_for(512.0), 1);
        assert_eq!(p.level_for(5000.0), 4); // 4096 ≤ 5000 < 8192
        assert_eq!(p.level_for(1e12), p.level_count() - 1);
        assert_eq!(p.range(0.0, 48_000.0 * 60.0, 100).level, 6); // 28 800 frames per column: 16 384-frame bins
    }

    #[test]
    fn envelope_contains_the_signal_and_round_trips() {
        let sig = signal(100_000);
        let p = build(&sig);
        for (i, f) in sig.iter().enumerate() {
            let bin = i / 256;
            for (c, v) in f.iter().enumerate() {
                let (mn, mx) = (p.levels[0][(bin * 2 + c) * 2], p.levels[0][(bin * 2 + c) * 2 + 1]);
                assert!(f32::from(mn) / 127.0 <= v + 1e-6 && f32::from(mx) / 127.0 >= v - 1e-6);
            }
        }
        let mut bytes = Vec::new();
        p.write_to(&mut bytes).unwrap();
        assert_eq!(PeakPyramid::read_from(&mut bytes.as_slice()).unwrap(), p);
        assert!(PeakPyramid::read_from(&mut &bytes[..20]).is_err());
        assert!(PeakPyramid::read_from(&mut &b"garbage!garbage!garbage!"[..]).is_err());
    }

    #[test]
    fn mono_and_empty() {
        let mut b = PeakBuilder::new(1);
        for i in 0..1000 {
            b.push_frame(&[if i == 500 { 1.0 } else { 0.0 }]);
        }
        let p = b.finish();
        assert_eq!(p.channels, 1);
        assert_eq!(p.range(0.0, 1000.0, 1).data, vec![0, 127]);
        let e = PeakBuilder::new(2).finish();
        assert_eq!(e.frames, 0);
        assert_eq!(e.range(0.0, 1000.0, 4).data, vec![0; 16]);
    }
}
