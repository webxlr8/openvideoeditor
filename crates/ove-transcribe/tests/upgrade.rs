//! Upgrade from the whisper-only downloader to the shared model store: a speech model saved by an earlier version
//! (directly in `<data dir>/models`) is adopted in place, nothing downloads, and captions still run.
//! Needs the tiny model (77 MB): set `OVE_WHISPER_MODEL_DIR` to a folder holding `ggml-tiny.bin` (downloaded there if
//! missing) and run with `--ignored`.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use ove_transcribe::{Model, Options, load_audio, models, transcribe};

fn speech_wav() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/e2e-app/fixtures/speech.wav")
}

#[test]
#[ignore = "needs the tiny speech model (77 MB download)"]
fn whisper_model_from_before_the_store_is_adopted_without_downloading() {
    // The cached model, laid out the way earlier versions saved it.
    let cache = std::env::var_os("OVE_WHISPER_MODEL_DIR").map(PathBuf::from).expect("set OVE_WHISPER_MODEL_DIR");
    let source = models::ensure_model(&cache, Model::Tiny, &mut |_, _| {}, &AtomicBool::new(false)).unwrap();
    let data = tempfile::tempdir().unwrap();
    let old_dir = data.path().join("models");
    std::fs::create_dir_all(&old_dir).unwrap();
    let old_file = old_dir.join("ggml-tiny.bin");
    std::fs::copy(&source, &old_file).unwrap();

    // Listed as installed straight away (as before the store).
    assert!(models::is_installed(&old_dir, Model::Tiny));

    // First use: adopted in place. Progress jumps straight to the end and no download folder appears.
    let mut seen = Vec::new();
    let path =
        models::ensure_model(&old_dir, Model::Tiny, &mut |d, t| seen.push((d, t)), &AtomicBool::new(false)).unwrap();
    assert_eq!(path, old_file);
    assert_eq!(seen, vec![(Model::Tiny.size_bytes(), Model::Tiny.size_bytes())]);
    assert!(!old_dir.join("whisper-tiny").exists(), "nothing was downloaded into the store layout");
    let index = std::fs::read_to_string(old_dir.join("installed.json")).unwrap();
    assert!(index.contains("\"id\": \"whisper-tiny\"") && index.contains("\"adopted\": true"), "{index}");
    assert_eq!(models::model_path(&old_dir, Model::Tiny), old_file);

    // Captions still run with the adopted file.
    let audio = load_audio(&speech_wav()).unwrap();
    let tr = transcribe(
        &path,
        &audio,
        &Options { language: Some("en".into()), ..Default::default() },
        |_| {},
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    eprintln!("transcript after the upgrade: {}", tr.text());
    assert!(!tr.words.is_empty(), "the adopted model recognised no words");
}
