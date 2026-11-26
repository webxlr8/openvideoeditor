//! Voice clean-up: removes steady background noise (fans, hum, hiss, traffic, room tone) from speech, on this
//! computer. Each channel runs through RNNoise, a small recurrent network trained on speech and noise (the
//! `nnnoiseless` port, BSD-3-Clause). The result is a 48 kHz 16-bit WAV of the same length as the source, aligned
//! with it sample for sample, so a clip that switches to it keeps its trims.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use nnnoiseless::DenoiseState;
use ove_media::{AudioDecoder, AudioOutputSpec, SampleFormat, Samples};

use crate::{Error, Result};

/// RNNoise works on 48 kHz audio only.
pub const RATE: u32 = 48_000;
const FRAME: usize = DenoiseState::FRAME_SIZE;
/// RNNoise takes and gives samples in the range of an i16.
const SCALE: f32 = i16::MAX as f32;

/// What [`clean_voice`] wrote.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cleaned {
    pub seconds: f64,
    pub channels: u16,
    /// Share of the audio (0–1) RNNoise judged to be speech.
    pub speech: f64,
}

/// One channel's denoiser and the samples waiting for a full frame.
struct Lane {
    state: Box<DenoiseState<'static>>,
    pending: Vec<f32>,
    out: Vec<f32>,
    /// The first frame out is the network warming up: it is dropped, which also removes the one-frame delay.
    warm: bool,
}

impl Lane {
    fn new() -> Lane {
        Lane { state: DenoiseState::new(), pending: Vec::with_capacity(FRAME), out: Vec::new(), warm: false }
    }

    /// Feeds one sample; returns the speech probability when a frame completes.
    fn push(&mut self, x: f32) -> Option<f32> {
        self.pending.push(x * SCALE);
        if self.pending.len() < FRAME {
            return None;
        }
        let mut buf = [0.0f32; FRAME];
        let vad = self.state.process_frame(&mut buf, &self.pending);
        self.pending.clear();
        if self.warm {
            self.out.extend(buf.iter().map(|v| v / SCALE));
        }
        self.warm = true;
        Some(vad)
    }
}

/// Writes interleaved 16-bit samples to a WAV file; sizes are patched in at the end.
struct Wav {
    file: BufWriter<File>,
    channels: u16,
    frames: u64,
}

impl Wav {
    fn create(path: &Path, channels: u16) -> std::io::Result<Wav> {
        let mut w = Wav { file: BufWriter::new(File::create(path)?), channels, frames: 0 };
        w.header(0)?;
        Ok(w)
    }

    fn header(&mut self, data_len: u32) -> std::io::Result<()> {
        let ch = self.channels as u32;
        let f = &mut self.file;
        f.write_all(b"RIFF")?;
        f.write_all(&(36 + data_len).to_le_bytes())?;
        f.write_all(b"WAVEfmt ")?;
        f.write_all(&16u32.to_le_bytes())?;
        f.write_all(&1u16.to_le_bytes())?;
        f.write_all(&(ch as u16).to_le_bytes())?;
        f.write_all(&RATE.to_le_bytes())?;
        f.write_all(&(RATE * ch * 2).to_le_bytes())?;
        f.write_all(&((ch * 2) as u16).to_le_bytes())?;
        f.write_all(&16u16.to_le_bytes())?;
        f.write_all(b"data")?;
        f.write_all(&data_len.to_le_bytes())
    }

    fn frame(&mut self, samples: impl Iterator<Item = f32>) -> std::io::Result<()> {
        for v in samples {
            let s = (v.clamp(-1.0, 1.0) * SCALE).round() as i16;
            self.file.write_all(&s.to_le_bytes())?;
        }
        self.frames += 1;
        Ok(())
    }

    fn finish(mut self) -> std::io::Result<()> {
        let len = self.frames * self.channels as u64 * 2;
        let len = u32::try_from(len).map_err(|_| std::io::Error::other("the cleaned audio is over 4 GB"))?;
        self.file.seek(SeekFrom::Start(0))?;
        self.header(len)?;
        self.file.flush()
    }
}

/// Cleans the speech in `input`'s audio and writes it to `output` (a WAV). `progress` gets 0–1; setting `cancel`
/// stops early with an error and no file. Sources with more than two channels are mixed down to stereo.
pub fn clean_voice(input: &Path, output: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(f64)) -> Result<Cleaned> {
    ove_media::init();
    let info = ove_media::probe(input)?;
    let src = info.audio.as_ref().ok_or_else(|| Error::NoAudio(input.display().to_string()))?;
    let channels = src.channels.clamp(1, 2) as u16;
    let total = info.duration_us.unwrap_or(0).max(1) as f64 / 1e6;
    let tmp = output.with_extension("wav.part");
    let r = run(input, &tmp, channels, total, cancel, progress);
    match r {
        Ok(c) => {
            std::fs::rename(&tmp, output)
                .map_err(|e| Error::Invalid(format!("cannot write {}: {e}", output.display())))?;
            Ok(c)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// How much of the start the network hears before cleaning.
const PRIME_SECONDS: usize = 10;

/// RNNoise lets noise through until it has heard some speech (its recurrent state starts empty), so noise before the
/// first word would stay. Each channel first hears up to [`PRIME_SECONDS`] of the file; that output is thrown away,
/// and cleaning starts again from the beginning with the warmed-up state.
fn prime(input: &Path, spec: AudioOutputSpec, lanes: &mut [Lane], cancel: &AtomicBool) -> Result<()> {
    let n = spec.channels as usize;
    let mut left = PRIME_SECONDS * RATE as usize;
    let mut dec = AudioDecoder::open(input, spec)?;
    while left > 0
        && let Some(chunk) = dec.next_chunk()?
    {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Invalid("cancelled".into()));
        }
        let Samples::F32(s) = &chunk.samples else { continue };
        for frame in s.chunks_exact(n).take(left) {
            for (lane, &x) in lanes.iter_mut().zip(frame) {
                lane.push(x);
            }
            left -= 1;
        }
    }
    for lane in lanes.iter_mut() {
        lane.pending.clear();
        lane.out.clear();
        // The first frame out after priming still carries the primed audio's tail: drop it, as at the start.
        lane.warm = false;
    }
    Ok(())
}

fn run(
    input: &Path,
    tmp: &Path,
    channels: u16,
    total: f64,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f64),
) -> Result<Cleaned> {
    let io = |e: std::io::Error| Error::Invalid(format!("cannot write {}: {e}", tmp.display()));
    let n = channels as usize;
    let spec = AudioOutputSpec { sample_rate: RATE, channels, format: SampleFormat::F32 };
    let mut lanes: Vec<Lane> = (0..n).map(|_| Lane::new()).collect();
    prime(input, spec, &mut lanes, cancel)?;
    let mut dec = AudioDecoder::open(input, spec)?;
    let mut wav = Wav::create(tmp, channels).map_err(io)?;
    let (mut fed, mut speech, mut frames) = (0u64, 0.0f64, 0u64);
    let drain = |lanes: &mut [Lane], wav: &mut Wav| -> std::io::Result<()> {
        let ready = lanes.iter().map(|l| l.out.len()).min().unwrap_or(0);
        for i in 0..ready {
            wav.frame(lanes.iter().map(|l| l.out[i]))?;
        }
        lanes.iter_mut().for_each(|l| drop(l.out.drain(..ready)));
        Ok(())
    };
    while let Some(chunk) = dec.next_chunk()? {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Invalid("cancelled".into()));
        }
        let Samples::F32(s) = &chunk.samples else { continue };
        // A gap before this chunk (audio that starts after the video, or a hole) stays silent, so the output keeps
        // the source's timing.
        let at = (chunk.pts_us.max(0) as u128 * RATE as u128 / 1_000_000) as u64;
        let mut push = |lanes: &mut [Lane], frame: &[f32]| {
            for (lane, &x) in lanes.iter_mut().zip(frame) {
                if let Some(v) = lane.push(x) {
                    speech += v as f64;
                    frames += 1;
                }
            }
        };
        while fed + 1 < at {
            push(&mut lanes, &[0.0; 2][..n]);
            fed += 1;
        }
        for frame in s.chunks_exact(n) {
            push(&mut lanes, frame);
            fed += 1;
        }
        drain(&mut lanes, &mut wav).map_err(io)?;
        progress((fed as f64 / RATE as f64 / total).min(1.0));
    }
    // Flush: one more frame of silence pushes the last real samples out (the dropped first frame was the delay).
    for _ in 0..FRAME * 2 {
        for lane in lanes.iter_mut() {
            lane.push(0.0);
        }
    }
    drain(&mut lanes, &mut wav).map_err(io)?;
    // Exactly as long as the source.
    let keep = fed.min(wav.frames);
    if wav.frames > keep {
        let len = (keep * n as u64 * 2) as usize;
        wav.file.flush().map_err(io)?;
        wav.file.get_ref().set_len(44 + len as u64).map_err(io)?;
        wav.file.seek(SeekFrom::Start(44 + len as u64)).map_err(io)?;
        wav.frames = keep;
    }
    wav.finish().map_err(io)?;
    progress(1.0);
    Ok(Cleaned {
        seconds: keep as f64 / RATE as f64,
        channels,
        speech: if frames == 0 { 0.0 } else { speech / frames as f64 },
    })
}
