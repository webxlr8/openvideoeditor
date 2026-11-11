//! Speech recognition with whisper.cpp (via whisper-rs): 16 kHz mono audio → words with timestamps.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use whisper_rs::{
    DtwMode, DtwModelPreset, DtwParameters, FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters,
};

use crate::error::{Result, TranscribeError};

pub const SAMPLE_RATE: u32 = 16_000;

/// One recognised word, times in microseconds from the start of the audio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Word {
    pub text: String,
    pub start_us: i64,
    pub end_us: i64,
    /// Mean token probability (0–1).
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    /// ISO 639-1 code detected (or requested).
    pub language: String,
    pub model: String,
    pub words: Vec<Word>,
    pub duration_us: i64,
}

impl Transcript {
    pub fn text(&self) -> String {
        self.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
    }
}

/// Decodes the audio of a media file to 16 kHz mono f32.
pub fn load_audio(path: &Path) -> Result<Vec<f32>> {
    use ove_media::{AudioDecoder, AudioOutputSpec, SampleFormat, Samples};
    let spec = AudioOutputSpec { sample_rate: SAMPLE_RATE, channels: 1, format: SampleFormat::F32 };
    let mut dec = AudioDecoder::open(path, spec).map_err(|e| match e.code() {
        "media.no_stream" => TranscribeError::NoAudio,
        _ => TranscribeError::Audio(e.to_string()),
    })?;
    let mut out = Vec::new();
    while let Some(chunk) = dec.next_chunk().map_err(|e| TranscribeError::Audio(e.to_string()))? {
        match chunk.samples {
            Samples::F32(v) => out.extend_from_slice(&v),
            Samples::S16(v) => out.extend(v.into_iter().map(|s| s as f32 / 32768.0)),
        }
    }
    if out.is_empty() {
        return Err(TranscribeError::NoAudio);
    }
    Ok(out)
}

type Loaded = Mutex<Vec<(String, Arc<WhisperContext>)>>;

fn contexts() -> &'static Loaded {
    static C: OnceLock<Loaded> = OnceLock::new();
    C.get_or_init(|| Mutex::new(Vec::new()))
}

/// Loads (and caches) a model.
fn context(model_path: &Path) -> Result<Arc<WhisperContext>> {
    // whisper.cpp prints every decoded token to stderr; route its logging to (silent) hooks instead.
    static QUIET: std::sync::Once = std::sync::Once::new();
    QUIET.call_once(whisper_rs::install_logging_hooks);
    let key = model_path.display().to_string();
    let mut cache = contexts().lock().unwrap();
    if let Some((_, c)) = cache.iter().find(|(k, _)| *k == key) {
        return Ok(c.clone());
    }
    // DTW alignment heads for the model gives each token a time where it is actually spoken.
    let name = model_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let preset = if name.contains("tiny") {
        Some(DtwModelPreset::Tiny)
    } else if name.contains("small") {
        Some(DtwModelPreset::Small)
    } else if name.contains("base") {
        Some(DtwModelPreset::Base)
    } else {
        None
    };
    let mut params = WhisperContextParameters::default();
    if let Some(model_preset) = preset {
        params.dtw_parameters(DtwParameters { mode: DtwMode::ModelPreset { model_preset }, ..Default::default() });
    }
    let ctx = WhisperContext::new_with_params(&key, params)
        .map_err(|e| TranscribeError::Model(format!("could not load the speech model: {e}")))?;
    let ctx = Arc::new(ctx);
    cache.clear(); // keep one model in memory
    cache.push((key, ctx.clone()));
    Ok(ctx)
}

#[derive(Debug, Clone)]
pub struct Options {
    /// ISO 639-1 code, or `None` to detect.
    pub language: Option<String>,
    pub threads: usize,
    pub model_name: String,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            language: None,
            threads: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8),
            model_name: "base".into(),
        }
    }
}

fn is_special(t: &str) -> bool {
    (t.starts_with("[_") || t.starts_with("<|")) && (t.ends_with(']') || t.ends_with("|>"))
}

/// Recognises speech in 16 kHz mono audio. `progress(0–100)`; `cancel` aborts.
pub fn transcribe(
    model_path: &Path,
    audio: &[f32],
    opts: &Options,
    progress: impl FnMut(i32) + 'static,
    cancel: Arc<AtomicBool>,
) -> Result<Transcript> {
    let ctx = context(model_path)?;
    let mut state = ctx.create_state().map_err(|e| TranscribeError::Model(e.to_string()))?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(opts.threads as i32);
    params.set_language(Some(opts.language.as_deref().unwrap_or("auto")));
    params.set_translate(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_print_special(false);
    // One word per segment, with token-level timestamps: whisper.cpp's word-timing mode.
    params.set_token_timestamps(true);
    params.set_max_len(1);
    params.set_split_on_word(true);
    params.set_suppress_blank(true);
    params.set_progress_callback_safe(progress);
    // whisper-rs 0.16's `set_abort_callback_safe` casts its boxed closure to the wrong type (the encoder then reads
    // garbage and aborts), so the abort flag is wired through the raw API.
    unsafe extern "C" fn should_abort(user: *mut std::ffi::c_void) -> bool {
        // SAFETY: `user` is the AtomicBool behind `cancel`, which outlives `state.full` below.
        unsafe { (*(user as *const AtomicBool)).load(Ordering::Relaxed) }
    }
    // SAFETY: the callback only reads the AtomicBool, kept alive by `cancel` for the whole call.
    unsafe {
        params.set_abort_callback(Some(should_abort));
        params.set_abort_callback_user_data(Arc::as_ptr(&cancel) as *mut std::ffi::c_void);
    }
    let r = state.full(params, audio);
    if cancel.load(Ordering::Relaxed) {
        return Err(TranscribeError::Cancelled);
    }
    r.map_err(|e| TranscribeError::Model(e.to_string()))?;

    let mut words: Vec<Word> = Vec::new();
    for seg in state.as_iter() {
        let text = seg.to_str_lossy().map(|s| s.into_owned()).unwrap_or_default();
        let t = text.trim();
        if t.is_empty() || is_special(t) {
            continue;
        }
        let mut probs = Vec::new();
        let mut dtw: Option<i64> = None;
        for i in 0..seg.n_tokens() {
            if let Some(tok) = seg.get_token(i) {
                let s = tok.to_str_lossy().map(|s| s.into_owned()).unwrap_or_default();
                if !is_special(s.trim()) && !s.trim().is_empty() {
                    probs.push(tok.token_probability());
                    let t = tok.token_data().t_dtw;
                    if dtw.is_none() && t >= 0 {
                        dtw = Some(t);
                    }
                }
            }
        }
        let confidence = if probs.is_empty() { 0.0 } else { probs.iter().sum::<f32>() / probs.len() as f32 };
        // Timestamps are in centiseconds. A word segment starts where the previous one ended, so the start comes from
        // the DTW time of its first token when available (clamped into the segment).
        let seg_start = seg.start_timestamp().max(0) * 10_000;
        let seg_end = (seg.end_timestamp() * 10_000).max(seg_start + 10_000);
        let start_us = dtw.map(|t| t * 10_000).filter(|t| *t >= 0).unwrap_or(seg_start);
        let end_us = seg_end;
        // A punctuation-only "word" attaches to the previous word.
        if t.chars().all(|c| c.is_ascii_punctuation()) {
            if let Some(last) = words.last_mut() {
                last.text.push_str(t);
                last.end_us = last.end_us.max(end_us);
            }
            continue;
        }
        words.push(Word { text: t.to_string(), start_us, end_us, confidence });
    }
    let duration_us = (audio.len() as i64 * 1_000_000) / SAMPLE_RATE as i64;
    finish_word_times(&mut words, duration_us);
    let lang_id = state.full_lang_id_from_state();
    let language = opts
        .language
        .clone()
        .or_else(|| whisper_rs::get_lang_str(lang_id).map(str::to_string))
        .unwrap_or_else(|| "en".into());
    Ok(Transcript { language, model: opts.model_name.clone(), words, duration_us })
}

/// A spoken word's typical length, from its letters (used where the next word is far away).
fn natural_length_us(text: &str) -> i64 {
    (text.chars().filter(|c| c.is_alphanumeric()).count() as i64 * 75_000).clamp(180_000, 900_000)
}

/// Word starts come from the alignment; segment end times lag behind the speech, so each word instead ends where the
/// next begins, or after its natural length when a pause follows. Times end up monotonic, non-overlapping and inside
/// the audio.
fn finish_word_times(words: &mut [Word], duration_us: i64) {
    for i in 1..words.len() {
        if words[i].start_us < words[i - 1].start_us {
            words[i].start_us = words[i - 1].start_us;
        }
    }
    for i in 0..words.len() {
        let natural = words[i].start_us + natural_length_us(&words[i].text);
        let end = match words.get(i + 1) {
            Some(next) => next.start_us.min(natural.max(words[i].start_us + 1)),
            None => natural,
        };
        words[i].end_us = end.min(duration_us).max(words[i].start_us);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(t: &str, s: f64) -> Word {
        Word { text: t.into(), start_us: (s * 1e6) as i64, end_us: 0, confidence: 1.0 }
    }

    #[test]
    fn words_end_at_the_next_word_or_after_their_natural_length() {
        let mut words = vec![w("Welcome", 0.8), w("to", 1.0), w("the", 1.3), w("editor.", 2.1), w("Late", 1.9)];
        finish_word_times(&mut words, 2_500_000);
        let ends: Vec<i64> = words.iter().map(|w| w.end_us).collect();
        // "the" is followed by a pause: it ends after ~3 letters, not at 2.1 s.
        assert_eq!(ends[..3], [1_000_000, 1_180_000, 1_525_000]);
        // An out-of-order start is pulled forward; the last word stops at the end of the audio.
        assert_eq!(words[4].start_us, 2_100_000);
        assert_eq!(words[3].end_us, 2_100_000);
        assert_eq!(words[4].end_us, 2_400_000);
        assert!(words.windows(2).all(|p| p[0].end_us <= p[1].start_us));
    }
}
