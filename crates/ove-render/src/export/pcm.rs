//! PCM output: float mix samples to 16-bit and 24-bit integers, and a WAV file writer. WAV is written here rather than
//! through FFmpeg so audio-only export needs no muxer, and so its samples are exactly the mixer's, rounded once.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

/// A float sample in -1…1 as 16-bit, rounded to the nearest step.
pub fn to_s16(x: f32) -> i16 {
    (x * 32_767.0).round().clamp(-32_768.0, 32_767.0) as i16
}

/// A float sample in -1…1 as 24-bit (in an `i32`), rounded to the nearest step.
pub fn to_s24(x: f32) -> i32 {
    (x as f64 * 8_388_607.0).round().clamp(-8_388_608.0, 8_388_607.0) as i32
}

/// Appends interleaved samples as little-endian 16-bit or 24-bit integers.
pub fn pack(samples: &[f32], bit_depth: u8, out: &mut Vec<u8>) {
    if bit_depth == 16 {
        out.reserve(samples.len() * 2);
        for &s in samples {
            out.extend_from_slice(&to_s16(s).to_le_bytes());
        }
    } else {
        out.reserve(samples.len() * 3);
        for &s in samples {
            out.extend_from_slice(&to_s24(s).to_le_bytes()[..3]);
        }
    }
}

/// KSDATAFORMAT_SUBTYPE_PCM, the sub-format of an extensible WAV header.
const PCM_SUBTYPE: [u8; 16] = [1, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71];

/// Writes a PCM WAV file: 16-bit with the plain header, 24-bit with the extensible one (as WAV readers expect for
/// more than 16 bits).
pub struct WavWriter {
    file: BufWriter<File>,
    bit_depth: u8,
    channels: u16,
    data_bytes: u64,
    header_len: u64,
    buf: Vec<u8>,
}

impl WavWriter {
    pub fn create(path: &Path, rate: u32, channels: u16, bit_depth: u8) -> std::io::Result<WavWriter> {
        let mut file = BufWriter::new(File::create(path)?);
        let bytes = u32::from(bit_depth / 8);
        let block = u16::try_from(u32::from(channels) * bytes).unwrap_or(u16::MAX);
        let extensible = bit_depth > 16;
        let fmt_len: u32 = if extensible { 40 } else { 16 };
        let mut h = Vec::with_capacity(68);
        h.extend_from_slice(b"RIFF");
        h.extend_from_slice(&0u32.to_le_bytes()); // patched in `finish`
        h.extend_from_slice(b"WAVEfmt ");
        h.extend_from_slice(&fmt_len.to_le_bytes());
        h.extend_from_slice(&(if extensible { 0xFFFEu16 } else { 1u16 }).to_le_bytes());
        h.extend_from_slice(&channels.to_le_bytes());
        h.extend_from_slice(&rate.to_le_bytes());
        h.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
        h.extend_from_slice(&block.to_le_bytes());
        h.extend_from_slice(&u16::from(bit_depth).to_le_bytes());
        if extensible {
            h.extend_from_slice(&22u16.to_le_bytes());
            h.extend_from_slice(&u16::from(bit_depth).to_le_bytes());
            // Front left and front right for stereo; unspecified otherwise.
            let mask: u32 = if channels == 2 { 3 } else { 0 };
            h.extend_from_slice(&mask.to_le_bytes());
            h.extend_from_slice(&PCM_SUBTYPE);
        }
        h.extend_from_slice(b"data");
        h.extend_from_slice(&0u32.to_le_bytes()); // patched in `finish`
        file.write_all(&h)?;
        Ok(WavWriter { file, bit_depth, channels, data_bytes: 0, header_len: h.len() as u64, buf: Vec::new() })
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Appends interleaved float samples.
    pub fn write(&mut self, samples: &[f32]) -> std::io::Result<()> {
        self.buf.clear();
        pack(samples, self.bit_depth, &mut self.buf);
        self.data_bytes += self.buf.len() as u64;
        if self.header_len + self.data_bytes > u64::from(u32::MAX) {
            return Err(std::io::Error::other(
                "the WAV file would pass 4 GB, the format's limit; export a shorter range",
            ));
        }
        self.file.write_all(&self.buf)
    }

    /// Writes the sizes into the header and flushes.
    pub fn finish(mut self) -> std::io::Result<()> {
        // A data chunk of odd length is padded to an even one.
        if self.data_bytes % 2 == 1 {
            self.file.write_all(&[0])?;
        }
        let riff = (self.header_len - 8 + self.data_bytes + self.data_bytes % 2) as u32;
        self.file.seek(SeekFrom::Start(4))?;
        self.file.write_all(&riff.to_le_bytes())?;
        self.file.seek(SeekFrom::Start(self.header_len - 4))?;
        self.file.write_all(&(self.data_bytes as u32).to_le_bytes())?;
        self.file.flush()
    }
}

/// Reads the samples of a WAV file this module wrote, as integers (tests and the mix check).
pub fn read_wav(path: &Path) -> std::io::Result<(u32, u16, u8, Vec<i32>)> {
    let b = std::fs::read(path)?;
    let bad = || std::io::Error::new(std::io::ErrorKind::InvalidData, "not a PCM WAV file");
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err(bad());
    }
    let (mut rate, mut channels, mut depth) = (0u32, 0u16, 0u8);
    let mut i = 12;
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let len = u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
        let body = b.get(i + 8..i + 8 + len).ok_or_else(bad)?;
        if id == b"fmt " && body.len() >= 16 {
            channels = u16::from_le_bytes([body[2], body[3]]);
            rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
            depth = body[14];
        } else if id == b"data" {
            let samples = match depth {
                16 => body.chunks_exact(2).map(|c| i32::from(i16::from_le_bytes([c[0], c[1]]))).collect(),
                24 => body.chunks_exact(3).map(|c| i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8).collect(),
                _ => return Err(bad()),
            };
            return Ok((rate, channels, depth, samples));
        }
        i += 8 + len + len % 2;
    }
    Err(bad())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_round_and_clip() {
        assert_eq!(to_s16(0.0), 0);
        assert_eq!(to_s16(1.0), 32_767);
        assert_eq!(to_s16(-2.0), -32_768);
        assert_eq!(to_s24(0.5), 4_194_304);
        assert_eq!(to_s24(-1.0), -8_388_607);
        let mut v = Vec::new();
        pack(&[-1.0 / 8_388_607.0], 24, &mut v);
        assert_eq!(v, [0xff, 0xff, 0xff]);
    }

    #[test]
    fn wav_round_trip_16_and_24_bit() {
        let d = tempfile::tempdir().unwrap();
        for depth in [16u8, 24] {
            let p = d.path().join(format!("t{depth}.wav"));
            let samples: Vec<f32> = (0..9_601).map(|i| ((i as f32) * 0.01).sin() * 0.8).collect();
            let mut w = WavWriter::create(&p, 48_000, 1, depth).unwrap();
            w.write(&samples).unwrap();
            w.finish().unwrap();
            let (rate, ch, got_depth, got) = read_wav(&p).unwrap();
            assert_eq!((rate, ch, got_depth, got.len()), (48_000, 1, depth, samples.len()));
            for (g, s) in got.iter().zip(&samples) {
                let want = if depth == 16 { i32::from(to_s16(*s)) } else { to_s24(*s) };
                assert_eq!(*g, want);
            }
            let len = std::fs::metadata(&p).unwrap().len();
            assert_eq!(len % 2, 0, "chunks are padded to even sizes");
        }
    }
}
