//! The daily check of the pinned download locations (add-model-runtime task 3.4): a HEAD request on every URL any
//! build pins (every model, and the runtime wheel for each shipped platform), so a host that removes a file is noticed
//! before a person's install fails. Run by the nightly CI job (`cargo test -p ove-models --test pinned_urls --
//! --ignored`).

use std::time::Duration;

use crate::manifest::{RuntimePlatform, WHISPER, WhisperPin, runtime_manifest};

/// Every pinned URL of every platform, once each.
pub fn all_pinned_urls() -> Vec<String> {
    let mut manifests: Vec<crate::Manifest> = WHISPER.iter().map(WhisperPin::manifest).collect();
    manifests.push(crate::manifest::vad_manifest());
    manifests.push(crate::manifest::siglip2_manifest());
    manifests.extend([RuntimePlatform::WindowsX64, RuntimePlatform::MacosArm64].map(runtime_manifest));
    let mut urls: Vec<String> = Vec::new();
    for m in &manifests {
        for f in &m.files {
            if !urls.contains(&f.url) {
                urls.push(f.url.clone());
            }
        }
    }
    urls
}

/// The outcome of one HEAD request: the final status (after redirects), or why there was none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadResult {
    pub url: String,
    pub outcome: Result<u16, String>,
}

impl HeadResult {
    pub fn ok(&self) -> bool {
        matches!(self.outcome, Ok(200..=299))
    }
}

/// Sends a HEAD request to each URL. A 404 (or any other error status, or no answer) is a failed result.
pub fn head_all(urls: &[String]) -> Vec<HeadResult> {
    let agent = ove_guard::http::agent_builder()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(30))
        .build();
    urls.iter()
        .map(|url| {
            let outcome = match agent.head(url).call() {
                Ok(r) => Ok(r.status()),
                Err(ureq::Error::Status(code, _)) => Ok(code),
                Err(e) => Err(e.to_string()),
            };
            HeadResult { url: url.clone(), outcome }
        })
        .collect()
}
