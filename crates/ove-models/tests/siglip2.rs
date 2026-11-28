//! The visual search model's manifest through the store (add-semantic-media-search, CTO/CEO condition 3): an agent or
//! a feature never starts the 0.63 GB download; the person's install resumes after an interrupted connection, checks
//! every file's SHA-256 and refuses a damaged one; Remove deletes the files and the model reads as not installed.
//!
//! The real files are 0.63 GB, so the test serves small stand-ins under the same id, version, file names, licence
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
    assert_eq!(m.download_bytes(), 633_152_112);
    assert_eq!(ove_models::error::size_text(m.download_bytes()), "0.63 GB");
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start();
    let s = Store::with_catalog(dir.path(), vec![m]).with_url_override(Some(UrlOverride::Mirror(server.base.clone())));
    for origin in [Origin::Agent, Origin::Feature] {
        match s.ensure_installed(SIGLIP2_ID, origin, &mut |_| {}, &AtomicBool::new(false)).unwrap() {
            Ensure::NeedsConsent(n) => {
                assert!(n.message.contains("0.63 GB download from huggingface.co"), "{}", n.message);
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
