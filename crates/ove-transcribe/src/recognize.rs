//! Speech recognition with whisper.cpp (via whisper-rs): 16 kHz mono audio → words with timestamps.
//!
//! Audio is recognised in bounded windows ([`WINDOW_SECONDS`] with [`OVERLAP_SECONDS`] of overlap), so a three-hour
//! file never needs more than one window in memory ([`transcribe_file`] decodes as it goes). Each window owns the
//! words that start between the midpoints of its overlaps; audio up to one window long is a single whisper call, as
//! before.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use serde::{Deserialize, Serialize};
use whisper_rs::{
    DtwMode, DtwModelPreset, DtwParameters, FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters,
};

use crate::error::{Result, TranscribeError};

pub const SAMPLE_RATE: u32 = 16_000;
/// Length of one recognition window (whisper's own context is 30 s).
pub const WINDOW_SECONDS: u32 = 30;
/// Overlap between consecutive windows; words are split between them at its midpoint.
pub const OVERLAP_SECONDS: u32 = 2;

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

fn open_audio(path: &Path) -> Result<ove_media::AudioDecoder> {
    use ove_media::{AudioDecoder, AudioOutputSpec, SampleFormat};
    let spec = AudioOutputSpec { sample_rate: SAMPLE_RATE, channels: 1, format: SampleFormat::F32 };
    AudioDecoder::open(path, spec).map_err(|e| match e.code() {
        "media.no_stream" => TranscribeError::NoAudio,
        _ => TranscribeError::Audio(e.to_string()),
    })
}

fn next_samples(dec: &mut ove_media::AudioDecoder) -> Result<Option<Vec<f32>>> {
    use ove_media::Samples;
    Ok(dec.next_chunk().map_err(|e| TranscribeError::Audio(e.to_string()))?.map(|c| match c.samples {
        Samples::F32(v) => v,
        Samples::S16(v) => v.into_iter().map(|s| s as f32 / 32768.0).collect(),
    }))
}

/// Decodes the whole audio of a media file to 16 kHz mono f32. For long media prefer [`transcribe_file`], which
/// decodes window by window.
pub fn load_audio(path: &Path) -> Result<Vec<f32>> {
    let mut dec = open_audio(path)?;
    let mut out = Vec::new();
    while let Some(v) = next_samples(&mut dec)? {
        out.extend_from_slice(&v);
    }
    if out.is_empty() {
        return Err(TranscribeError::NoAudio);
    }
    Ok(out)
}

/// One recognition window: 16 kHz mono samples starting at sample `start` of the whole audio.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioWindow {
    pub start: usize,
    pub samples: Vec<f32>,
    /// No audio follows this window.
    pub last: bool,
}

/// A media file's audio as overlapping windows, decoded as they are read: at most one window plus one decoded chunk
/// is held in memory.
pub struct AudioWindows {
    dec: ove_media::AudioDecoder,
    window: usize,
    step: usize,
    buf: Vec<f32>,
    buf_start: usize,
    eof: bool,
    done: bool,
    /// Largest number of samples buffered at once (for tests of the memory bound).
    pub peak_buffered: usize,
}

/// Opens `path` for reading in windows of `window` samples overlapping by `overlap` (`overlap < window`).
pub fn audio_windows(path: &Path, window: usize, overlap: usize) -> Result<AudioWindows> {
    assert!(window > 0 && overlap < window, "window {window}, overlap {overlap}");
    Ok(AudioWindows {
        dec: open_audio(path)?,
        window,
        step: window - overlap,
        buf: Vec::new(),
        buf_start: 0,
        eof: false,
        done: false,
        peak_buffered: 0,
    })
}

impl Iterator for AudioWindows {
    type Item = Result<AudioWindow>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        // One sample beyond the window tells whether this window is the last.
        while self.buf.len() <= self.window && !self.eof {
            match next_samples(&mut self.dec) {
                Ok(Some(v)) => self.buf.extend_from_slice(&v),
                Ok(None) => self.eof = true,
                Err(e) => {
                    self.done = true;
                    return Some(Err(e));
                }
            }
            self.peak_buffered = self.peak_buffered.max(self.buf.len());
        }
        let last = self.buf.len() <= self.window;
        let take = self.buf.len().min(self.window);
        let w = AudioWindow { start: self.buf_start, samples: self.buf[..take].to_vec(), last };
        if last {
            self.done = true;
        } else {
            self.buf.drain(..self.step);
            self.buf_start += self.step;
        }
        Some(Ok(w))
    }
}

/// In-memory audio as the same windows [`audio_windows`] reads from a file.
fn slice_windows(audio: &[f32], window: usize, overlap: usize) -> impl Iterator<Item = Result<AudioWindow>> + '_ {
    let step = window - overlap;
    let mut start = 0;
    let mut done = audio.is_empty();
    std::iter::from_fn(move || {
        if done {
            return None;
        }
        let end = (start + window).min(audio.len());
        let last = end == audio.len();
        let w = AudioWindow { start, samples: audio[start..end].to_vec(), last };
        done = last;
        start += step;
        Some(Ok(w))
    })
}

fn samples_to_us(n: usize) -> i64 {
    (n as i64 * 1_000_000) / SAMPLE_RATE as i64
}

/// Runs `recognise` on each window (words timed from the window's start) and merges the results on the whole
/// audio's time line. A window keeps the words that start between the midpoint of its overlap with the previous
/// window and the midpoint of its overlap with the next. Returns the words and the total number of samples.
fn merge_windows<I, F>(windows: I, overlap: usize, mut recognise: F) -> Result<(Vec<Word>, usize)>
where
    I: Iterator<Item = Result<AudioWindow>>,
    F: FnMut(&AudioWindow) -> Result<Vec<Word>>,
{
    let mut words = Vec::new();
    let mut total = 0;
    let mut from_us = i64::MIN;
    for w in windows {
        let w = w?;
        let end = w.start + w.samples.len();
        total = total.max(end);
        let offset = samples_to_us(w.start);
        let until_us = if w.last { i64::MAX } else { samples_to_us(end - overlap / 2) };
        for mut word in recognise(&w)? {
            word.start_us += offset;
            word.end_us += offset;
            if word.start_us >= from_us && word.start_us < until_us {
                words.push(word);
            }
        }
        from_us = until_us;
        if w.last {
            break;
        }
    }
    Ok((words, total))
}

type Loaded = Mutex<Vec<(String, Arc<WhisperContext>)>>;

fn contexts() -> &'static Loaded {
    static C: OnceLock<Loaded> = OnceLock::new();
    C.get_or_init(|| Mutex::new(Vec::new()))
}

/// The model cache, recovered if a thread panicked while holding it (the cache holds only loaded models, which stay
/// valid; a poisoned lock must not break transcription for the rest of the session).
fn lock_contexts() -> MutexGuard<'static, Vec<(String, Arc<WhisperContext>)>> {
    contexts().lock().unwrap_or_else(|poisoned| {
        contexts().clear_poison();
        poisoned.into_inner()
    })
}

/// Loads (and caches) a model.
fn context(model_path: &Path) -> Result<Arc<WhisperContext>> {
    // whisper.cpp prints every decoded token to stderr; route its logging to (silent) hooks instead.
    static QUIET: std::sync::Once = std::sync::Once::new();
    QUIET.call_once(whisper_rs::install_logging_hooks);
    let key = model_path.display().to_string();
    let mut cache = lock_contexts();
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

/// Recognises speech in 16 kHz mono audio. `progress(0–100)`; `cancel` aborts. Long audio is recognised in windows
/// (see the module docs); audio up to [`WINDOW_SECONDS`] is one whisper call.
pub fn transcribe(
    model_path: &Path,
    audio: &[f32],
    opts: &Options,
    progress: impl FnMut(i32) + 'static,
    cancel: Arc<AtomicBool>,
) -> Result<Transcript> {
    let (window, overlap) = window_samples();
    let windows = audio.len().saturating_sub(overlap).div_ceil(window - overlap).max(1);
    transcribe_windows(model_path, slice_windows(audio, window, overlap), overlap, windows, opts, progress, cancel)
}

/// Recognises the speech of a media file, decoding its audio one window at a time so memory stays bounded however
/// long the file is. Gives the same result as [`transcribe`] on [`load_audio`].
pub fn transcribe_file(
    model_path: &Path,
    media: &Path,
    opts: &Options,
    progress: impl FnMut(i32) + 'static,
    cancel: Arc<AtomicBool>,
) -> Result<Transcript> {
    let (window, overlap) = window_samples();
    let expected = ove_media::probe(media)
        .ok()
        .and_then(|i| i.duration_us)
        .map(|us| ((us.max(0) as u64 * SAMPLE_RATE as u64 / 1_000_000) as usize).saturating_sub(overlap))
        .map(|n| n.div_ceil(window - overlap).max(1))
        .unwrap_or(1);
    let windows = audio_windows(media, window, overlap)?;
    transcribe_windows(model_path, windows, overlap, expected, opts, progress, cancel)
}

fn window_samples() -> (usize, usize) {
    ((WINDOW_SECONDS * SAMPLE_RATE) as usize, (OVERLAP_SECONDS * SAMPLE_RATE) as usize)
}

fn transcribe_windows(
    model_path: &Path,
    windows: impl Iterator<Item = Result<AudioWindow>>,
    overlap: usize,
    expected_windows: usize,
    opts: &Options,
    progress: impl FnMut(i32) + 'static,
    cancel: Arc<AtomicBool>,
) -> Result<Transcript> {
    let ctx = context(model_path)?;
    let progress = Rc::new(RefCell::new(progress));
    let mut language = opts.language.clone();
    let mut detected: Option<i32> = None;
    let mut index = 0usize;
    let (words, total) = merge_windows(windows, overlap, |w| {
        if cancel.load(Ordering::Relaxed) {
            return Err(TranscribeError::Cancelled);
        }
        let k = index;
        index += 1;
        let n = expected_windows.max(index) as i32;
        let p = progress.clone();
        let report = move |pct: i32| (*p.borrow_mut())((k as i32 * 100 + pct.clamp(0, 100)) / n);
        let (words, lang_id) = recognise_window(&ctx, &w.samples, language.as_deref(), opts.threads, report, &cancel)?;
        // Later windows keep the language detected in the first one.
        if language.is_none() {
            detected = Some(lang_id);
            language = whisper_rs::get_lang_str(lang_id).map(str::to_string);
        }
        Ok(words)
    })?;
    if total == 0 {
        return Err(TranscribeError::NoAudio);
    }
    let mut words = words;
    let duration_us = samples_to_us(total);
    finish_word_times(&mut words, duration_us);
    let language = opts
        .language
        .clone()
        .or_else(|| detected.and_then(whisper_rs::get_lang_str).map(str::to_string))
        .unwrap_or_else(|| "en".into());
    Ok(Transcript { language, model: opts.model_name.clone(), words, duration_us })
}

/// One whisper call: words timed from the start of `audio` (not yet given their final end times), and the language
/// id whisper used.
fn recognise_window(
    ctx: &WhisperContext,
    audio: &[f32],
    language: Option<&str>,
    threads: usize,
    progress: impl FnMut(i32) + 'static,
    cancel: &Arc<AtomicBool>,
) -> Result<(Vec<Word>, i32)> {
    let mut state = ctx.create_state().map_err(|e| TranscribeError::Model(e.to_string()))?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(threads as i32);
    params.set_language(Some(language.unwrap_or("auto")));
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
    /// whisper.cpp's abort callback.
    ///
    /// # Safety
    ///
    /// `user` must point to a live `AtomicBool` for the duration of the call.
    unsafe extern "C" fn should_abort(user: *mut std::ffi::c_void) -> bool {
        // SAFETY: `user` is the AtomicBool behind `cancel`, which outlives `state.full` below.
        unsafe { (*(user as *const AtomicBool)).load(Ordering::Relaxed) }
    }
    // SAFETY: `should_abort` has the signature whisper.cpp expects and only reads the `AtomicBool` it is given; the
    // user data is that `AtomicBool`, kept alive by the `cancel` Arc we borrow until after `state.full` returns,
    // which is the only time whisper.cpp calls back.
    unsafe {
        params.set_abort_callback(Some(should_abort));
        params.set_abort_callback_user_data(Arc::as_ptr(cancel) as *mut std::ffi::c_void);
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
    Ok((words, state.full_lang_id_from_state()))
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

    fn word(text: String, start_us: i64) -> Word {
        Word { text, start_us, end_us: start_us + 100_000, confidence: 1.0 }
    }

    /// Stand-in recogniser: one word per started second of absolute time inside the window, named after it.
    fn per_second(w: &AudioWindow) -> Result<Vec<Word>> {
        let (start, end) = (samples_to_us(w.start), samples_to_us(w.start + w.samples.len()));
        Ok((start / 1_000_000..)
            .map(|s| s * 1_000_000)
            .skip_while(|t| *t < start)
            .take_while(|t| *t < end)
            .map(|t| word(format!("s{}", t / 1_000_000), t - start))
            .collect())
    }

    #[test]
    fn short_audio_is_one_window_and_unchanged() {
        let audio: Vec<f32> = (0..16_000 * 9).map(|i| (i as f32 * 0.01).sin()).collect();
        let (window, overlap) = window_samples();
        let mut calls = Vec::new();
        let raw = vec![word("hello".into(), 1_000_000), word("world.".into(), 1_500_000)];
        let (words, total) = merge_windows(slice_windows(&audio, window, overlap), overlap, |w| {
            calls.push(w.clone());
            Ok(raw.clone())
        })
        .unwrap();
        // Exactly the old single call: the whole audio, words as recognised.
        assert_eq!(calls, vec![AudioWindow { start: 0, samples: audio.clone(), last: true }]);
        assert_eq!((words, total), (raw, audio.len()));
    }

    #[test]
    fn long_audio_words_are_kept_once_on_the_whole_time_line() {
        let audio = vec![0f32; 16_000 * 95]; // 95 s: four 30 s windows overlapping by 2 s
        let (window, overlap) = window_samples();
        let mut sizes = Vec::new();
        let (words, total) = merge_windows(slice_windows(&audio, window, overlap), overlap, |w| {
            sizes.push(w.samples.len());
            per_second(w)
        })
        .unwrap();
        assert_eq!(total, audio.len());
        assert_eq!(sizes.len(), 4);
        assert!(sizes.iter().all(|n| *n <= window), "{sizes:?}");
        let names: Vec<String> = words.iter().map(|w| w.text.clone()).collect();
        let want: Vec<String> = (0..95).map(|s| format!("s{s}")).collect();
        assert_eq!(names, want, "every second exactly once, in order");
        assert!(words.iter().all(|w| w.start_us == w.text[1..].parse::<i64>().unwrap() * 1_000_000));
    }

    #[test]
    fn a_poisoned_model_cache_recovers() {
        let _ = std::thread::spawn(|| {
            let _guard = lock_contexts();
            panic!("poison the model cache");
        })
        .join();
        assert!(contexts().is_poisoned());
        assert!(lock_contexts().is_empty());
        assert!(!contexts().is_poisoned());
    }

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
