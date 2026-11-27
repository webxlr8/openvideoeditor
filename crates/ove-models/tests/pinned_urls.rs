//! The pinned download locations still answer (add-model-runtime task 3.4). The live check runs daily in the nightly
//! CI job; the local test proves the check fails on a 404.
#![allow(clippy::unwrap_used, reason = "test code: a failure here is a failed test")]

mod common;

use common::{Mode, Server};
use ove_models::check::{all_pinned_urls, head_all};

#[test]
fn the_check_fails_on_a_404_and_passes_on_a_file_that_is_there() {
    let server = Server::start();
    server.add("/there.bin", vec![1, 2, 3], Mode::Normal);
    let urls = vec![format!("{}/there.bin", server.base), format!("{}/gone.bin", server.base)];
    let results = head_all(&urls);
    assert!(results[0].ok(), "{:?}", results[0]);
    assert!(!results[1].ok());
    assert_eq!(results[1].outcome, Ok(404));
}

#[test]
fn every_platform_and_model_is_covered() {
    let urls = all_pinned_urls();
    assert!(urls.iter().any(|u| u.ends_with("ggml-small.bin")));
    assert!(urls.iter().any(|u| u.ends_with("win_amd64.whl")));
    assert!(urls.iter().any(|u| u.ends_with("macosx_14_0_arm64.whl")));
    assert!(urls.iter().any(|u| u.ends_with("silero_vad.onnx")));
    assert_eq!(urls.len(), 6, "3 speech models, voice activity and 2 runtime wheels, each once");
}

/// The live check: every pinned URL answers a HEAD request with success. Fails (and names the URL) on a 404.
#[test]
#[ignore = "network: run daily by the nightly CI job"]
fn every_pinned_url_answers() {
    let failed: Vec<String> = head_all(&all_pinned_urls())
        .into_iter()
        .inspect(|r| println!("{:?} {}", r.outcome, r.url))
        .filter(|r| !r.ok())
        .map(|r| format!("{} -> {:?}", r.url, r.outcome))
        .collect();
    assert!(failed.is_empty(), "pinned URLs that no longer answer:\n{}", failed.join("\n"));
}
