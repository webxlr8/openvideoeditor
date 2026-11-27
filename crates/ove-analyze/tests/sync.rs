//! Tasks 2.1–2.3: coarse, fine and drift measurements on synthetic recordings (speech-like bursts plus pink noise,
//! low-passed, 10 dB SNR), read through the same onset envelope the analysis cache stores.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::time::Instant;

use ove_analyze::sync::{SyncParams, align, coarse, envelope};
use ove_analyze::synth::{Recording, Scene};

/// Camera: 60 s of the scene. Recorder: 40 s from scene time 12.345, low-passed at 4 kHz, 10 dB SNR.
fn pair() -> (Recording, Recording) {
    let scene = Scene::speech(7);
    let cam = Recording::new(scene, 0.0, 60.0).noise(10.0, 11);
    let rec = Recording::new(scene, 12.345, 40.0).lowpass(4_000.0).noise(10.0, 12);
    (cam, rec)
}

#[test]
fn coarse_search_finds_the_offset_within_10_ms() {
    let (cam, rec) = pair();
    let (ec, er) = (envelope(&cam.envelope()), envelope(&rec.envelope()));
    let c = coarse(&ec, &er, &SyncParams::default()).unwrap();
    eprintln!("coarse lag {:.4} s (error {:.2} ms), {c:?}", c.lag_s, (c.lag_s - 12.345).abs() * 1000.0);
    assert!((c.lag_s - 12.345).abs() <= 0.010, "coarse {c:?}");
    assert!(c.confidence > 0.3 && c.peak_ratio > 1.5, "{c:?}");
    assert!((c.overlap_s - 40.0).abs() < 0.1, "{c:?}");
    // The other way round: the camera sits 12.345 s before the recorder's start.
    let back = coarse(&er, &ec, &SyncParams::default()).unwrap();
    assert!((back.lag_s + 12.345).abs() <= 0.010, "{back:?}");
}

#[test]
fn looped_music_and_unrelated_sound_are_refused() {
    let looped = Scene::looped(3, 4.0);
    let a = Recording::new(looped, 0.0, 60.0).noise(30.0, 1);
    let b = Recording::new(looped, 9.0, 30.0).noise(30.0, 2);
    let r = coarse(&envelope(&a.envelope()), &envelope(&b.envelope()), &SyncParams::default()).unwrap_err();
    assert!(r.reason.contains("ambiguous"), "{r:?}");

    let other = Recording::new(Scene::speech(99), 0.0, 30.0).noise(10.0, 3);
    let (cam, _) = pair();
    let r = coarse(&envelope(&cam.envelope()), &envelope(&other.envelope()), &SyncParams::default()).unwrap_err();
    assert!(r.coarse.is_some(), "the best candidate is reported: {r:?}");
}

#[test]
fn fine_search_is_within_1_ms() {
    let (mut cam, mut rec) = pair();
    let (ec, er) = (envelope(&cam.envelope()), envelope(&rec.envelope()));
    let m = align(&ec, &mut cam, &er, &mut rec, &SyncParams::default()).unwrap().unwrap();
    let err_ms = (m.offset_s - 12.345).abs() * 1000.0;
    eprintln!("fine offset {:.6} s, error {err_ms:.4} ms, peak {:?}", m.offset_s, m.fine_peak);
    assert!(err_ms <= 1.0, "{m:?}");
    assert!(m.drift.is_none(), "40 s is too short for a drift measurement");
}

/// The recording-time error of a match at reference time `t` against the truth `u = clock · (t − offset)`.
fn error_ms(m: &ove_analyze::sync::Match, t: f64, offset: f64, clock: f64) -> f64 {
    let pred = (1.0 + m.speed_ppm * 1e-6) * (t - m.offset_s);
    let truth = clock * (t - offset);
    (pred - truth).abs() * 1000.0
}

#[test]
fn two_hour_recorder_50_ppm_fast_is_corrected() {
    let scene = Scene::speech(21);
    let clock = 1.000_05;
    let mut cam = Recording::new(scene, 0.0, 7_200.0).noise(10.0, 31);
    let mut rec = Recording::new(scene, 30.0, 7_100.0).clock(clock).lowpass(4_000.0).noise(10.0, 32);
    let t = Instant::now();
    let (ec, er) = (envelope(&cam.envelope()), envelope(&rec.envelope()));
    eprintln!("envelopes: {:.1} s", t.elapsed().as_secs_f64());
    let t = Instant::now();
    let m = align(&ec, &mut cam, &er, &mut rec, &SyncParams::default()).unwrap().unwrap();
    eprintln!("align: {:.2} s", t.elapsed().as_secs_f64());
    let d = m.drift.clone().unwrap();
    let head = 30.0;
    let tail = 30.0 + 7_100.0 / clock;
    let (eh, et) = (error_ms(&m, head, 30.0, clock), error_ms(&m, tail, 30.0, clock));
    eprintln!(
        "drift {:.3} ppm (true 50), head error {eh:.4} ms, tail error {et:.4} ms, residual {:.4} ms, windows {}/{}",
        d.ppm, d.residual_ms, d.used, d.windows
    );
    assert!(d.applied, "{d:?}");
    assert!((d.ppm - 50.0).abs() <= 3.0, "{d:?}");
    assert!((m.speed_ppm - 50.0).abs() <= 3.0);
    assert!(eh <= 1.0 && et <= 2.0, "head {eh} ms, tail {et} ms");
}

#[test]
fn drift_of_2000_ppm_is_reported_and_not_applied() {
    let scene = Scene::speech(5);
    let clock = 1.002;
    let mut cam = Recording::new(scene, 0.0, 600.0).noise(10.0, 41);
    let mut rec = Recording::new(scene, 20.0, 540.0).clock(clock).lowpass(4_000.0).noise(10.0, 42);
    let (ec, er) = (envelope(&cam.envelope()), envelope(&rec.envelope()));
    let m = align(&ec, &mut cam, &er, &mut rec, &SyncParams::default()).unwrap().unwrap();
    let d = m.drift.clone().unwrap();
    eprintln!("drift {d:?}");
    assert!(!d.applied, "{d:?}");
    assert!((d.ppm - 2000.0).abs() < 50.0, "{d:?}");
    assert!(d.note.as_deref().unwrap_or("").contains("out of range"), "{d:?}");
    assert_eq!(m.speed_ppm, 0.0);
}

/// Task 2.5: a two-hour reference against ten 10-minute targets with cached envelopes (the envelopes are made first,
/// as the import analysis cache holds them; raw audio for the fine step is synthesised on read).
#[test]
fn two_hour_reference_against_ten_targets_timing() {
    let scene = Scene::speech(33);
    let mut cam = Recording::new(scene, 0.0, 7_200.0).noise(15.0, 1);
    let ec = envelope(&cam.envelope());
    let mut targets: Vec<(f64, Recording, Vec<f32>)> = (0..10)
        .map(|k| {
            let at = 60.0 + 690.0 * k as f64;
            let r = Recording::new(scene, at, 600.0).lowpass(4_000.0).noise(10.0, 100 + k);
            let e = envelope(&r.envelope());
            (at, r, e)
        })
        .collect();
    let t = Instant::now();
    for (at, r, e) in targets.iter_mut() {
        let m = align(&ec, &mut cam, e, r, &SyncParams::default()).unwrap().unwrap();
        assert!((m.offset_s - *at).abs() <= 0.001, "{at}: {m:?}");
    }
    let secs = t.elapsed().as_secs_f64();
    eprintln!("2 h reference, 10 targets of 10 min: {secs:.2} s");
    assert!(secs < 10.0, "{secs} s");
}
