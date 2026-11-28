//! The visual search model's manifest through the store (add-semantic-media-search, CTO/CEO condition 3): an agent or
//! a feature never starts the 0.76 GB download; the person's install resumes after an interrupted connection, checks
//! every file's SHA-256 and refuses a damaged one; Remove deletes the files and the model reads as not installed. The
//! switch to the half-precision text encoder reuses the installed image encoder and removes the old text encoder
//! only once the new one is verified.
//!
//! The real files are 0.76 GB, so the test serves small stand-ins under the same id, version, file names, licence
//! and layout (three files from one host); the consent check uses the real pinned sizes.

mod common;

use std::sync::atomic::AtomicBool;

use common::{Mode, Server, body, sha};
use ove_models::manifest::{SIGLIP2_ID, SIGLIP2_TEXT, SIGLIP2_TOKENIZER, SIGLIP2_VISION, siglip2_manifest};
use ove_models::{Ensure, ModelError, Origin, Store, UrlOverride, needs_consent};

/// The real manifest with its files replaced by `bodies` (same names, order and everything else).
fn stand_in(bodies: &[Vec<u8>; 3]) -> ove_models::Manifest {
    let mut m = siglip2_manifest();
    for (f, b) in m.files.iter_mut().zip(bodies) {
        f.sha256 = sha(b);
        f.bytes = b.len() as u64;
    }
    m
}

fn path(name: &str) -> String {
    format!("/{SIGLIP2_ID}/{}/{name}", siglip2_manifest().version)
}

#[test]
fn the_real_pins_need_consent_and_name_size_model_and_licence() {
    let m = siglip2_manifest();
    assert!(needs_consent(&m));
    // The size people see is the sum of the pinned files.
    let sum: u64 = m.files.iter().map(|f| f.bytes).sum();
    assert_eq!(m.download_bytes(), sum);
    let size = ove_models::error::size_text(sum);
    assert_eq!(size, "0.76 GB");
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start();
    let s = Store::with_catalog(dir.path(), vec![m]).with_url_override(Some(UrlOverride::Mirror(server.base.clone())));
    for origin in [Origin::Agent, Origin::Feature] {
        match s.ensure_installed(SIGLIP2_ID, origin, &mut |_| {}, &AtomicBool::new(false)).unwrap() {
            Ensure::NeedsConsent(n) => {
                assert!(n.message.contains(&format!("{size} download from huggingface.co")), "{}", n.message);
                assert!(
                    n.message.contains("SigLIP 2 base") && n.message.contains("licence Apache-2.0"),
                    "{}",
                    n.message
                );
            }
            other => panic!("expected NeedsConsent, got {other:?}"),
        }
    }
    assert!(server.requests().is_empty(), "nothing was downloaded");
}

#[test]
fn an_interrupted_install_resumes_checks_hashes_and_remove_works() {
    let bodies = [body(400_000, 21), body(300_000, 22), body(50_000, 23)];
    let server = Server::start();
    // The text encoder's connection drops at 40 %; the downloader resumes it with a range request.
    server.add(&path(SIGLIP2_VISION), bodies[0].clone(), Mode::Normal);
    server.add(&path(SIGLIP2_TEXT), bodies[1].clone(), Mode::CutFirstAfter(120_000));
    server.add(&path(SIGLIP2_TOKENIZER), bodies[2].clone(), Mode::Normal);
    let dir = tempfile::tempdir().unwrap();
    let s = Store::with_catalog(dir.path(), vec![stand_in(&bodies)])
        .with_url_override(Some(UrlOverride::Mirror(server.base.clone())));
    let got = s.ensure_installed(SIGLIP2_ID, Origin::Person, &mut |_| {}, &AtomicBool::new(false)).unwrap();
    let Ensure::Ready(paths) = got else { panic!("not installed: {got:?}") };
    assert_eq!(paths.len(), 3);
    for (p, b) in paths.iter().zip(&bodies) {
        assert_eq!(&std::fs::read(p).unwrap(), b);
    }
    let text_requests: Vec<_> =
        server.requests().into_iter().filter(|r| r.path.ends_with(SIGLIP2_TEXT)).map(|r| r.range_start).collect();
    assert_eq!(text_requests, vec![None, Some(120_000)], "the text encoder resumed from the received bytes");
    assert!(s.is_installed(SIGLIP2_ID));

    // Remove: the files go, the space is reported, and the model reads as not installed again.
    let total: u64 = bodies.iter().map(|b| b.len() as u64).sum();
    assert_eq!(s.remove(SIGLIP2_ID).unwrap(), Some(total));
    assert!(!s.is_installed(SIGLIP2_ID));
    assert!(paths.iter().all(|p| !p.exists()));
    assert_eq!(s.remove(SIGLIP2_ID).unwrap(), None);
}

#[test]
fn a_damaged_file_is_refused_by_its_hash_and_nothing_is_installed() {
    let bodies = [body(200_000, 31), body(150_000, 32), body(40_000, 33)];
    let server = Server::start();
    server.add(&path(SIGLIP2_VISION), bodies[0].clone(), Mode::Normal);
    server.add(&path(SIGLIP2_TEXT), bodies[1].clone(), Mode::Normal);
    server.add(&path(SIGLIP2_TOKENIZER), bodies[2].clone(), Mode::Corrupt);
    let dir = tempfile::tempdir().unwrap();
    let s = Store::with_catalog(dir.path(), vec![stand_in(&bodies)])
        .with_url_override(Some(UrlOverride::Mirror(server.base.clone())));
    let e = s.install(SIGLIP2_ID, &mut |_| {}, &AtomicBool::new(false)).unwrap_err();
    assert_eq!(e, ModelError::Corrupt { model: SIGLIP2_ID.into(), file: SIGLIP2_TOKENIZER.into() });
    assert!(!s.is_installed(SIGLIP2_ID));
    let version = siglip2_manifest().version;
    assert!(!dir.path().join(SIGLIP2_ID).join(&version).join(SIGLIP2_TOKENIZER).exists());
}

/// The manifest update from the 4-bit to the half-precision text encoder: the image encoder and the tokenizer keep
/// their pins, so the person's install reuses them (no request for them), downloads only the new text encoder, checks
/// it, and only then deletes the old text encoder and the previous version's folder. A failed download of the new
/// text encoder leaves the old files in place.
#[test]
fn a_text_encoder_update_reuses_the_image_encoder_and_removes_the_old_text_file() {
    let image = body(300_000, 41);
    let old_text = body(200_000, 42);
    let new_text = body(250_000, 43);
    let tok = body(30_000, 44);
    let mut old = stand_in(&[image.clone(), old_text.clone(), tok.clone()]);
    old.version = "previous".into();
    old.files[1].name = "text_model_q4f16.onnx".into();
    let new = stand_in(&[image.clone(), new_text.clone(), tok.clone()]);
    assert_eq!(new.files[0].sha256, old.files[0].sha256, "the image encoder keeps its pin");

    let server = Server::start();
    server.add(&format!("/{SIGLIP2_ID}/previous/{SIGLIP2_VISION}"), image.clone(), Mode::Normal);
    server.add(&format!("/{SIGLIP2_ID}/previous/text_model_q4f16.onnx"), old_text.clone(), Mode::Normal);
    server.add(&format!("/{SIGLIP2_ID}/previous/{SIGLIP2_TOKENIZER}"), tok.clone(), Mode::Normal);
    let dir = tempfile::tempdir().unwrap();
    let mirror = || Some(UrlOverride::Mirror(server.base.clone()));
    let before = Store::with_catalog(dir.path(), vec![old]).with_url_override(mirror());
    before.install(SIGLIP2_ID, &mut |_| {}, &AtomicBool::new(false)).unwrap();
    let old_text_path = dir.path().join(SIGLIP2_ID).join("previous").join("text_model_q4f16.onnx");
    assert!(old_text_path.is_file());

    // The new text encoder is damaged on its first try: nothing old is deleted, and the model is not installed.
    server.add(&path(SIGLIP2_TEXT), new_text.clone(), Mode::Corrupt);
    let after = Store::with_catalog(dir.path(), vec![new]).with_url_override(mirror());
    assert!(!after.is_installed(SIGLIP2_ID), "a new manifest version needs its new file");
    let e = after.install(SIGLIP2_ID, &mut |_| {}, &AtomicBool::new(false)).unwrap_err();
    assert_eq!(e, ModelError::Corrupt { model: SIGLIP2_ID.into(), file: SIGLIP2_TEXT.into() });
    assert!(old_text_path.is_file(), "the old text encoder stays until the new one is verified");

    // The good file: only the new text encoder is requested; the image encoder and tokenizer are reused.
    server.add(&path(SIGLIP2_TEXT), new_text.clone(), Mode::Normal);
    let requests_before = server.requests().len();
    let out = after.install(SIGLIP2_ID, &mut |_| {}, &AtomicBool::new(false)).unwrap();
    let new_requests: Vec<String> = server.requests()[requests_before..].iter().map(|r| r.path.clone()).collect();
    assert_eq!(new_requests, vec![path(SIGLIP2_TEXT)], "only the new text encoder downloads");
    for (p, b) in out.paths.iter().zip([&image, &new_text, &tok]) {
        assert_eq!(&std::fs::read(p).unwrap(), b);
    }
    assert!(after.is_installed(SIGLIP2_ID));
    assert!(!old_text_path.exists(), "the old 4-bit text encoder is removed");
    assert!(!dir.path().join(SIGLIP2_ID).join("previous").exists(), "the previous version's folder is gone");
}
