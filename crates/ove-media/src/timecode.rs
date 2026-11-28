//! Source timecode (design D1): the start timecode a camera or sound recorder wrote into a file.
//!
//! Two sources are read:
//! - the container `timecode` tag, which FFmpeg fills from a QuickTime timecode track (`tmcd`) or MXF metadata;
//! - Broadcast WAV `bext.time_reference` (a sample count since midnight) and the iXML `TIMECODE_RATE` and
//!   `TIMECODE_FLAG`, read by the small RIFF chunk reader in this module.
//!
//! The start is kept as an exact rational number of seconds, so two files with shared timecode line up to the exact
//! sample: a timecode label at a fractional rate is `frames × den / num` seconds, and a Broadcast WAV start is
//! `samples / sample_rate` seconds.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Where a timecode came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimecodeSource {
    /// The container `timecode` tag (QuickTime timecode track, MXF).
    Container,
    /// Broadcast WAV `time_reference`, with the rate from iXML when present.
    Bwf,
    /// iXML only (no `bext` chunk).
    Ixml,
}

/// A start timecode with its exact rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceTimecode {
    /// Whole frames since midnight at the timecode rate (drop-frame labels already converted to a frame count).
    pub frames: i64,
    pub rate_num: u32,
    pub rate_den: u32,
    pub drop_frame: bool,
    /// Exact start in samples since midnight, when the file states it in samples (Broadcast WAV).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub samples: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<u32>,
    pub source: TimecodeSource,
}

/// Frames dropped per minute (except every tenth) for drop-frame timecode at a nominal rate. Drop-frame is defined
/// for multiples of 30; other rates drop nothing.
fn drop_count(nominal: i64) -> i64 {
    if nominal % 30 == 0 { nominal / 15 } else { 0 }
}

/// The whole-frame label rate: 24 for 24000/1001, 30 for 30000/1001.
fn nominal(num: u32, den: u32) -> i64 {
    ((num as f64 / den.max(1) as f64).round() as i64).max(1)
}

impl SourceTimecode {
    /// Parses `HH:MM:SS:FF` (`;` or `.` before the frames means drop-frame) at the given rate.
    pub fn from_label(label: &str, rate_num: u32, rate_den: u32) -> Option<SourceTimecode> {
        if rate_num == 0 || rate_den == 0 {
            return None;
        }
        let label = label.trim();
        let drop = label.contains(';') || label.contains('.');
        let parts: Vec<i64> = label.split([':', ';', '.']).map(|p| p.parse::<i64>().ok()).collect::<Option<_>>()?;
        let [h, m, s, f] = parts[..] else { return None };
        let fps = nominal(rate_num, rate_den);
        if !(0..24).contains(&h) || !(0..60).contains(&m) || !(0..60).contains(&s) || !(0..fps).contains(&f) {
            return None;
        }
        let total_minutes = h * 60 + m;
        let mut frames = ((h * 3600 + m * 60 + s) * fps) + f;
        if drop {
            frames -= drop_count(fps) * (total_minutes - total_minutes / 10);
        }
        Some(SourceTimecode {
            frames,
            rate_num,
            rate_den,
            drop_frame: drop,
            samples: None,
            sample_rate: None,
            source: TimecodeSource::Container,
        })
    }

    /// A start in samples since midnight, labelled at the given timecode rate.
    pub fn from_samples(samples: u64, sample_rate: u32, rate: (u32, u32), drop_frame: bool) -> Option<SourceTimecode> {
        let (rate_num, rate_den) = rate;
        if sample_rate == 0 || rate_num == 0 || rate_den == 0 {
            return None;
        }
        let frames = (samples as i128 * rate_num as i128 / (sample_rate as i128 * rate_den as i128)) as i64;
        Some(SourceTimecode {
            frames,
            rate_num,
            rate_den,
            drop_frame,
            samples: Some(samples),
            sample_rate: Some(sample_rate),
            source: TimecodeSource::Bwf,
        })
    }

    /// The exact start, seconds as `(numerator, denominator)`.
    pub fn start(&self) -> (i128, i128) {
        match (self.samples, self.sample_rate) {
            (Some(s), Some(r)) if r > 0 => (s as i128, r as i128),
            _ => (self.frames as i128 * self.rate_den as i128, self.rate_num.max(1) as i128),
        }
    }

    /// The start in seconds.
    pub fn seconds(&self) -> f64 {
        let (n, d) = self.start();
        n as f64 / d as f64
    }

    /// `other`'s start minus this start, in samples at `rate` (rounded to the nearest sample).
    pub fn offset_samples(&self, other: &SourceTimecode, rate: u32) -> i64 {
        let (an, ad) = self.start();
        let (bn, bd) = other.start();
        // (bn/bd − an/ad) × rate, in exact integer arithmetic.
        let num = (bn * ad - an * bd) * rate as i128;
        let den = ad * bd;
        let q = num.div_euclid(den);
        let r = num.rem_euclid(den);
        (if 2 * r >= den { q + 1 } else { q }) as i64
    }

    /// `other`'s start minus this start, in microseconds (rounded).
    pub fn offset_us(&self, other: &SourceTimecode) -> i64 {
        self.offset_samples(other, 1_000_000)
    }

    /// The label, `HH:MM:SS:FF` or `HH:MM:SS;FF` for drop-frame.
    pub fn label(&self) -> String {
        let fps = nominal(self.rate_num, self.rate_den);
        let mut n = self.frames.rem_euclid(24 * 3600 * fps);
        let drop = if self.drop_frame { drop_count(fps) } else { 0 };
        if drop > 0 {
            // Add back the dropped labels: per 10 minutes `drop × 9` are skipped.
            let per_10 = fps * 600 - drop * 9;
            let per_min = fps * 60 - drop;
            let tens = n / per_10;
            let rem = n % per_10;
            n += drop * 9 * tens + if rem > drop { drop * ((rem - drop) / per_min) } else { 0 };
        }
        let f = n % fps;
        let s = (n / fps) % 60;
        let m = (n / (fps * 60)) % 60;
        let h = n / (fps * 3600);
        let sep = if self.drop_frame { ';' } else { ':' };
        format!("{h:02}:{m:02}:{s:02}{sep}{f:02}")
    }
}

/// What a WAV file's `fmt `, `bext` and `iXML` chunks say about its start.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WavTimecodeInfo {
    pub sample_rate: Option<u32>,
    pub time_reference: Option<u64>,
    pub ixml_rate: Option<(u32, u32)>,
    pub ixml_drop: Option<bool>,
    /// `BWF_TIME_REFERENCE_LOW/HIGH` from iXML, used when there is no `bext` chunk.
    pub ixml_time_reference: Option<u64>,
}

/// Longest iXML chunk read (they are a few kilobytes).
const MAX_IXML: u64 = 1 << 20;

/// Reads the RIFF (or RF64/BW64) chunk list of a WAV file. Returns `None` when the file is not a WAV file.
pub fn read_wav_chunks(path: &Path) -> Option<WavTimecodeInfo> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 12];
    f.read_exact(&mut head).ok()?;
    if !matches!(&head[0..4], b"RIFF" | b"RF64" | b"BW64") || &head[8..12] != b"WAVE" {
        return None;
    }
    let len = f.metadata().ok()?.len();
    let mut out = WavTimecodeInfo::default();
    let mut pos = 12u64;
    while pos + 8 <= len {
        f.seek(SeekFrom::Start(pos)).ok()?;
        let mut ch = [0u8; 8];
        f.read_exact(&mut ch).ok()?;
        let id = [ch[0], ch[1], ch[2], ch[3]];
        let size = u32::from_le_bytes([ch[4], ch[5], ch[6], ch[7]]) as u64;
        let body = pos + 8;
        match &id {
            b"fmt " if size >= 8 => {
                let mut b = [0u8; 8];
                f.read_exact(&mut b).ok()?;
                out.sample_rate = Some(u32::from_le_bytes([b[4], b[5], b[6], b[7]]));
            }
            b"bext" if size >= 346 => {
                f.seek(SeekFrom::Start(body + 338)).ok()?;
                let mut b = [0u8; 8];
                f.read_exact(&mut b).ok()?;
                out.time_reference = Some(u64::from_le_bytes(b));
            }
            b"iXML" if size <= MAX_IXML => {
                let mut b = vec![0u8; size as usize];
                f.read_exact(&mut b).ok()?;
                parse_ixml(&String::from_utf8_lossy(&b), &mut out);
            }
            // The data chunk of an RF64 file says 0xFFFFFFFF; its real size is in ds64. Metadata chunks before
            // it are all we need, and BWF writers put `bext` first.
            b"data" if size == u32::MAX as u64 => break,
            _ => {}
        }
        pos = body + size + (size & 1);
    }
    Some(out)
}

fn xml_value<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find("</")? + start;
    Some(xml[start..end].trim())
}

fn parse_ixml(xml: &str, out: &mut WavTimecodeInfo) {
    if let Some(r) = xml_value(xml, "TIMECODE_RATE") {
        let rate = match r.split_once('/') {
            Some((n, d)) => n.trim().parse().ok().zip(d.trim().parse().ok()),
            None => r.parse::<u32>().ok().map(|n| (n, 1)),
        };
        out.ixml_rate = rate.filter(|(n, d)| *n > 0 && *d > 0);
    }
    if let Some(flag) = xml_value(xml, "TIMECODE_FLAG") {
        out.ixml_drop = Some(flag.eq_ignore_ascii_case("DF"));
    }
    let lo = xml_value(xml, "BWF_TIME_REFERENCE_LOW").and_then(|v| v.parse::<u64>().ok());
    let hi = xml_value(xml, "BWF_TIME_REFERENCE_HIGH").and_then(|v| v.parse::<u64>().ok());
    if let Some(lo) = lo {
        out.ixml_time_reference = Some((hi.unwrap_or(0) << 32) | (lo & 0xFFFF_FFFF));
    }
}

/// Label rate used for a WAV start without an iXML rate.
const DEFAULT_WAV_RATE: (u32, u32) = (25, 1);

/// The start timecode of a WAV file, from `bext` and iXML. A zero `time_reference` without an iXML timecode rate
/// means "not set" (recorders write zero when they have no timecode).
pub fn wav_timecode(path: &Path) -> Option<SourceTimecode> {
    let info = read_wav_chunks(path)?;
    let rate = info.sample_rate?;
    let (samples, source) = match (info.time_reference, info.ixml_time_reference) {
        (Some(t), _) => (t, TimecodeSource::Bwf),
        (None, Some(t)) => (t, TimecodeSource::Ixml),
        (None, None) => return None,
    };
    if samples == 0 && info.ixml_rate.is_none() {
        return None;
    }
    let mut tc = SourceTimecode::from_samples(
        samples,
        rate,
        info.ixml_rate.unwrap_or(DEFAULT_WAV_RATE),
        info.ixml_drop.unwrap_or(false),
    )?;
    tc.source = source;
    Some(tc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_round_trip_at_every_common_rate() {
        for &(n, d) in &[(24000u32, 1001u32), (24, 1), (25, 1), (30000, 1001), (50, 1), (60000, 1001)] {
            for label in ["00:00:00:00", "10:00:00:00", "01:02:03:04", "23:59:59:20"] {
                let tc = SourceTimecode::from_label(label, n, d).unwrap();
                assert_eq!(tc.label(), label, "{n}/{d}");
            }
        }
    }

    #[test]
    fn drop_frame_skips_two_labels_per_minute_except_tenths() {
        let a = SourceTimecode::from_label("00:00:59;29", 30000, 1001).unwrap();
        let b = SourceTimecode::from_label("00:01:00;02", 30000, 1001).unwrap();
        assert_eq!(b.frames - a.frames, 1);
        let ten = SourceTimecode::from_label("00:10:00;00", 30000, 1001).unwrap();
        assert_eq!(ten.frames, 17_982);
        assert_eq!(ten.label(), "00:10:00;00");
        // One hour of 29.97 drop-frame is 107,892 frames: within 4 ms of an hour of real time.
        let hour = SourceTimecode::from_label("01:00:00;00", 30000, 1001).unwrap();
        assert_eq!(hour.frames, 107_892);
        assert!((hour.seconds() - 3600.0).abs() < 0.004, "{}", hour.seconds());
        for f in [0i64, 1_799, 1_800, 1_801, 17_981, 17_982, 107_891, 107_892, 2_000_000] {
            let tc = SourceTimecode { frames: f, ..hour };
            let back = SourceTimecode::from_label(&tc.label(), 30000, 1001).unwrap();
            assert_eq!(back.frames, f, "{}", tc.label());
        }
    }

    #[test]
    fn bad_labels_are_refused() {
        assert!(SourceTimecode::from_label("25:00:00:00", 25, 1).is_none());
        assert!(SourceTimecode::from_label("00:00:00:25", 25, 1).is_none());
        assert!(SourceTimecode::from_label("00:00:00", 25, 1).is_none());
        assert!(SourceTimecode::from_label("00:00:00:00", 0, 1).is_none());
    }

    #[test]
    fn video_and_wav_with_shared_timecode_align_to_the_sample() {
        // Video at 10:00:00:00, 23.976; the recorder starts 4 s later and states its start in samples at 48 kHz.
        let video = SourceTimecode::from_label("10:00:00:00", 24000, 1001).unwrap();
        let video_samples = video.frames as u64 * 1001 * 48_000 / 24_000;
        let wav = SourceTimecode::from_samples(video_samples + 4 * 48_000, 48_000, (24000, 1001), false).unwrap();
        assert_eq!(video.offset_samples(&wav, 48_000), 192_000);
        assert_eq!(video.offset_us(&wav), 4_000_000);
        assert_eq!(wav.offset_samples(&video, 48_000), -192_000);
        // Four real seconds at 23.976 are 95.9 frames, so the label reads one frame short of 04:00.
        assert_eq!(wav.label(), "10:00:03:23");
    }

    #[test]
    fn ixml_rate_flag_and_reference_are_read() {
        let mut out = WavTimecodeInfo::default();
        parse_ixml(
            "<BWFXML><SPEED><TIMECODE_RATE>30000/1001</TIMECODE_RATE><TIMECODE_FLAG>DF</TIMECODE_FLAG>\
             <BWF_TIME_REFERENCE_LOW>5</BWF_TIME_REFERENCE_LOW><BWF_TIME_REFERENCE_HIGH>1</BWF_TIME_REFERENCE_HIGH>\
             </SPEED></BWFXML>",
            &mut out,
        );
        assert_eq!(out.ixml_rate, Some((30000, 1001)));
        assert_eq!(out.ixml_drop, Some(true));
        assert_eq!(out.ixml_time_reference, Some((1 << 32) | 5));
    }
}
