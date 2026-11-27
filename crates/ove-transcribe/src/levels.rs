//! Audio levels per media file: 10 ms RMS loudness ([`Envelope`]) decoded once and cached in the bundle under the
//! media's content identity (`cache/levels/<contentId>.json`). Word cuts snap their edges to quiet points with it;
//! dead-air detection measures pauses against the speaker's level with it.

use std::path::{Path, PathBuf};

use crate::cuts::Envelope;

fn path(bundle: &Path, key: &str) -> PathBuf {
    bundle.join("cache").join("levels").join(format!("{key}.json"))
}

/// The cached envelope of a media file.
pub fn load(bundle: &Path, key: &str) -> Option<Envelope> {
    serde_json::from_slice(&std::fs::read(path(bundle, key)).ok()?).ok()
}

pub fn store(bundle: &Path, key: &str, env: &Envelope) {
    let p = path(bundle, key);
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = p.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_vec(env).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

/// Decodes a media file's audio (16 kHz mono) chunk by chunk and measures it.
#[cfg(feature = "audio")]
pub fn measure(media: &Path) -> crate::Result<Envelope> {
    let mut acc = Accumulator::default();
    decode(media, &mut |chunk| acc.push(chunk))?;
    Ok(acc.finish())
}

/// Calls `f` with every decoded chunk of 16 kHz mono f32 samples.
#[cfg(feature = "audio")]
pub fn decode(media: &Path, f: &mut dyn FnMut(&[f32])) -> crate::Result<()> {
    use ove_media::{AudioDecoder, AudioOutputSpec, SampleFormat, Samples};

    use crate::TranscribeError;
    let spec = AudioOutputSpec { sample_rate: crate::SAMPLE_RATE, channels: 1, format: SampleFormat::F32 };
    let mut dec = AudioDecoder::open(media, spec).map_err(|e| match e.code() {
        "media.no_stream" => TranscribeError::NoAudio,
        _ => TranscribeError::Audio(e.to_string()),
    })?;
    while let Some(c) = dec.next_chunk().map_err(|e| TranscribeError::Audio(e.to_string()))? {
        match c.samples {
            Samples::F32(v) => f(&v),
            Samples::S16(v) => f(&v.into_iter().map(|s| s as f32 / 32768.0).collect::<Vec<_>>()),
        }
    }
    Ok(())
}

/// Builds an envelope from chunks of any length.
#[derive(Debug, Default)]
pub struct Accumulator {
    pending: Vec<f32>,
    db: Vec<f32>,
}

impl Accumulator {
    pub fn push(&mut self, chunk: &[f32]) {
        self.pending.extend_from_slice(chunk);
        let whole = self.pending.len() / 160 * 160;
        if whole > 0 {
            self.db.extend(Envelope::from_samples(&self.pending[..whole]).db);
            self.pending.drain(..whole);
        }
    }

    pub fn finish(mut self) -> Envelope {
        if !self.pending.is_empty() {
            self.db.extend(Envelope::from_samples(&self.pending).db);
        }
        Envelope { cell_us: 10_000, db: self.db }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_of_any_size_give_the_same_envelope() {
        let samples: Vec<f32> = (0..5000).map(|i| ((i as f32) * 0.01).sin() * (i % 700) as f32 / 700.0).collect();
        let whole = Envelope::from_samples(&samples);
        let mut acc = Accumulator::default();
        for c in samples.chunks(333) {
            acc.push(c);
        }
        assert_eq!(acc.finish(), whole);
        let dir = tempfile::tempdir().unwrap();
        store(dir.path(), "k", &whole);
        assert_eq!(load(dir.path(), "k"), Some(whole));
    }
}
