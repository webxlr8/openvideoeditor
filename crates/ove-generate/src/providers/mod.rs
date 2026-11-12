//! Provider adapters. Each turns a request into output bytes, polling where the provider is asynchronous and
//! stopping when cancelled. Base URLs can be overridden for tests (`OVE_FAL_QUEUE_URL`, `OVE_OPENAI_URL`,
//! `OVE_ELEVENLABS_URL`) in debug builds, or in release builds with `OVE_DEV_ENDPOINTS=1` (see [`base`]).

pub mod elevenlabs;
pub mod fal;
pub mod openai;

use std::io::Read;
use std::time::Duration;

use serde_json::Value;

use crate::catalog::{Model, Provider};
use crate::job::Request;

/// A refusal or failure, with a stable code.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct GenError {
    pub code: String,
    pub message: String,
}

impl GenError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        GenError { code: code.into(), message: message.into() }
    }
}

pub type Result<T> = std::result::Result<T, GenError>;

/// What a provider returned.
#[derive(Debug, Clone)]
pub struct Output {
    pub bytes: Vec<u8>,
    /// File extension: png, jpg, mp4, mp3, …
    pub ext: String,
    pub details: Value,
}

/// Progress callbacks and the cancel check for one run.
pub struct Run<'a> {
    pub key: &'a str,
    pub cancelled: &'a dyn Fn() -> bool,
    /// (progress 0–1 if known, provider request id once known)
    pub progress: &'a mut dyn FnMut(Option<f64>, Option<&str>),
}

pub fn poll_interval() -> Duration {
    Duration::from_millis(std::env::var("OVE_GENERATE_POLL_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(1500))
}

/// A provider's API root: `default`, or the override in `var` where the build allows it (debug builds, or
/// `OVE_DEV_ENDPOINTS=1`; https, or http to localhost only). See [`ove_guard::endpoint`].
pub fn base(var: &str, default: &str) -> String {
    ove_guard::endpoint::base_url(var, default)
}

/// The HTTP agent for calls that carry a key. Redirects are not followed: ureq drops `Authorization` on a redirect to
/// another host but forwards custom key headers (`xi-api-key`), and no provider API redirects.
pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(300))
        .redirects(0)
        .build()
}

/// The HTTP agent for downloading outputs (no key is ever set on it; CDNs redirect).
fn download_agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(15)).timeout_read(Duration::from_secs(300)).build()
}

/// Maps an HTTP error to a refusal with the provider's message.
pub fn http_error(provider: &str, e: ureq::Error) -> GenError {
    match e {
        ureq::Error::Status(code, r) => {
            let body = r.into_string().unwrap_or_default();
            let detail = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| {
                    v.pointer("/error/message")
                        .or_else(|| v.pointer("/detail/message"))
                        .or_else(|| v.get("detail"))
                        .or_else(|| v.get("message"))
                        .or_else(|| v.get("error"))
                        .map(|d| d.as_str().map(str::to_string).unwrap_or_else(|| d.to_string()))
                })
                .unwrap_or_else(|| body.chars().take(300).collect());
            let code_s = match code {
                401 | 403 => "provider.auth",
                402 => "provider.credits",
                429 => "provider.rateLimited",
                400..=499 => "provider.refused",
                _ => "provider.failed",
            };
            let hint = match code {
                401 | 403 => " — check the API key in Settings",
                402 => " — the provider account has no credits",
                _ => "",
            };
            GenError::new(code_s, format!("{provider}: HTTP {code}: {detail}{hint}"))
        }
        ureq::Error::Transport(t) => GenError::new("network", format!("{provider}: {t}")),
    }
}

pub fn read_bytes(r: ureq::Response, limit: u64) -> Result<Vec<u8>> {
    let mut v = Vec::new();
    r.into_reader().take(limit).read_to_end(&mut v).map_err(|e| GenError::new("network", e.to_string()))?;
    Ok(v)
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn b64_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(B64[(n >> 18) as usize & 63] as char);
        s.push(B64[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    s
}

pub fn b64_decode(s: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for ch in s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        let v = match ch {
            b'A'..=b'Z' => ch - b'A',
            b'a'..=b'z' => ch - b'a' + 26,
            b'0'..=b'9' => ch - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return Err(GenError::new("provider.failed", "bad base64 in the response")),
        } as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

/// Downloads a result URL (or decodes a `data:` URI).
pub fn fetch(url: &str) -> Result<(Vec<u8>, String)> {
    if let Some(rest) = url.strip_prefix("data:") {
        let (meta, data) = rest.split_once(',').ok_or_else(|| GenError::new("provider.failed", "bad data URI"))?;
        let mime = meta.split(';').next().unwrap_or("").to_string();
        return Ok((b64_decode(data)?, mime));
    }
    let r = download_agent().get(url).call().map_err(|e| http_error("download", e))?;
    let mime = r.content_type().to_string();
    Ok((read_bytes(r, 2 << 30)?, mime))
}

pub fn ext_for(mime: &str, url: &str, fallback: &str) -> String {
    let from_mime = match mime {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/webp" => Some("webp"),
        "video/mp4" => Some("mp4"),
        "video/quicktime" => Some("mov"),
        "video/webm" => Some("webm"),
        "audio/mpeg" => Some("mp3"),
        "audio/wav" | "audio/x-wav" => Some("wav"),
        _ => None,
    };
    if let Some(e) = from_mime {
        return e.into();
    }
    let path = url.split(['?', '#']).next().unwrap_or("");
    match path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()) {
        Some(e) if e.len() <= 4 && e.chars().all(|c| c.is_ascii_alphanumeric()) => e,
        _ => fallback.into(),
    }
}

/// The image as a `data:` URI (inputs such as a first frame).
pub fn data_uri(path: &std::path::Path) -> Result<String> {
    let bytes =
        std::fs::read(path).map_err(|e| GenError::new("file.unreadable", format!("{}: {e}", path.display())))?;
    let mime = match path.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        _ => "image/jpeg",
    };
    Ok(format!("data:{mime};base64,{}", b64_encode(&bytes)))
}

/// Runs one request on its provider.
pub fn run(model: &Model, req: &Request, aspect: &str, r: &mut Run) -> Result<Output> {
    ove_guard::register_secret(r.key);
    match model.provider {
        Provider::Fal => fal::run(model, req, aspect, r),
        Provider::OpenAi => openai::run(model, req, aspect, r),
        Provider::ElevenLabs => elevenlabs::run(model, req, r),
    }
}
