//! Keeping secrets out of log output.
//!
//! Two nets, both applied by [`redact`]:
//!
//! 1. A process-wide registry of secret values. API keys call [`register_secret`] when they are loaded (credential
//!    store, environment) or handed to a client, and every registered value is replaced wherever it appears.
//! 2. Header-shaped text: the value after `authorization`, `x-api-key`, `xi-api-key`, `api-key` or `api_key` followed
//!    by `:` or `=` is replaced, so a key that was never registered (an HTTP library's debug dump of a request it is
//!    about to send, say) is still caught.
//!
//! [`RedactingWriter`] applies both to everything written through it; the app's log layers (file and stderr) write
//! through one.

use std::borrow::Cow;
use std::io::{self, Write};
use std::sync::RwLock;

/// What a secret is replaced with.
pub const REDACTED: &str = "[redacted]";

/// Shorter values are not registered: they would match ordinary text.
const MIN_SECRET_LEN: usize = 8;
/// Bound on the registry (keys are few; this only stops unbounded growth from a caller registering in a loop).
const MAX_SECRETS: usize = 256;

static SECRETS: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// Header and parameter names whose value is a credential (matched case-insensitively, anywhere in a word, so
/// `x-api-key` and `xi-api-key` are covered by `api-key`).
const SENSITIVE_NAMES: &[&str] = &["authorization", "api-key", "api_key"];

/// Remembers `secret` so [`redact`] replaces it from now on (for the life of the process). Values shorter than 8
/// characters are ignored.
pub fn register_secret(secret: &str) {
    let s = secret.trim();
    if s.len() < MIN_SECRET_LEN {
        return;
    }
    let mut v = SECRETS.write().unwrap_or_else(|e| e.into_inner());
    if v.iter().any(|x| x == s) {
        return;
    }
    if v.len() >= MAX_SECRETS {
        v.remove(0);
    }
    v.push(s.to_string());
}

/// `text` with every registered secret and every credential-header value replaced by [`REDACTED`].
pub fn redact(text: &str) -> Cow<'_, str> {
    let mut out = Cow::Borrowed(text);
    {
        let secrets = SECRETS.read().unwrap_or_else(|e| e.into_inner());
        // Longest first, so a key that contains another registered value is replaced whole.
        let mut ordered: Vec<&String> = secrets.iter().filter(|s| out.contains(s.as_str())).collect();
        ordered.sort_by_key(|s| std::cmp::Reverse(s.len()));
        for s in ordered {
            if out.contains(s.as_str()) {
                out = Cow::Owned(out.replace(s.as_str(), REDACTED));
            }
        }
    }
    match redact_headers(&out) {
        Some(s) => Cow::Owned(s),
        None => out,
    }
}

/// Characters that end a header value: line ends, quotes, an escaped character (JSON-formatted logs) and separators
/// between fields or query parameters. Spaces do not (`Authorization: Bearer …`, `Key …`).
fn ends_value(b: u8) -> bool {
    matches!(b, b'\r' | b'\n' | b'"' | b'\'' | b'\\' | b',' | b';' | b'&' | b'}' | b']' | b')' | b'>')
}

/// Replaces the values of credential headers; `None` when there is nothing to replace.
fn redact_headers(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase(); // same byte offsets as `text`
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut copied = 0; // bytes of `text` already in `out`
    let mut pos = 0;
    let mut changed = false;
    while pos < bytes.len() {
        // The nearest sensitive name from `pos`.
        let Some((at, name)) =
            SENSITIVE_NAMES.iter().filter_map(|n| lower[pos..].find(n).map(|i| (pos + i, n.len()))).min()
        else {
            break;
        };
        let mut i = at + name;
        // `"x-api-key": "…"`, `x-api-key: …`, `api_key=…`
        if i < bytes.len() && matches!(bytes[i], b'"' | b'\'') {
            i += 1;
        }
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
            i += 1;
        }
        if i >= bytes.len() || !matches!(bytes[i], b':' | b'=') {
            pos = at + name;
            continue;
        }
        i += 1;
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'"' | b'\'') {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && !ends_value(bytes[i]) {
            i += 1;
        }
        let value = text[start..i].trim_end();
        let end = start + value.len();
        if !value.is_empty() && value != REDACTED && value != "***" {
            out.push_str(&text[copied..start]);
            out.push_str(REDACTED);
            copied = end;
            changed = true;
        }
        pos = end.max(at + name);
    }
    if !changed {
        return None;
    }
    out.push_str(&text[copied..]);
    Some(out)
}

/// An `io::Write` that passes text through [`redact`] on its way to `inner`.
///
/// Output is buffered until a newline, a flush or drop, so a secret is never split across two writes to `inner`.
/// Log formatters write one whole event per writer, which this handles in one piece.
#[derive(Debug)]
pub struct RedactingWriter<W: Write> {
    inner: W,
    buf: Vec<u8>,
}

impl<W: Write> RedactingWriter<W> {
    pub fn new(inner: W) -> Self {
        RedactingWriter { inner, buf: Vec::new() }
    }

    fn emit(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let res = {
            let text = String::from_utf8_lossy(&self.buf);
            self.inner.write_all(redact(&text).as_bytes())
        };
        self.buf.clear();
        res
    }
}

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        if self.buf.ends_with(b"\n") {
            self.emit()?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.emit()?;
        self.inner.flush()
    }
}

impl<W: Write> Drop for RedactingWriter<W> {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_secrets_are_replaced() {
        register_secret("sk-ant-registry-test-0123456789");
        register_secret("short"); // ignored
        let r = redact("calling with sk-ant-registry-test-0123456789 now; short stays");
        assert_eq!(r, "calling with [redacted] now; short stays");
        assert!(matches!(redact("nothing secret here"), Cow::Borrowed(_)));
    }

    #[test]
    fn longer_secret_wins_over_a_contained_one() {
        register_secret("overlap-secret-abc");
        register_secret("overlap-secret-abc-and-more");
        assert_eq!(redact("x overlap-secret-abc-and-more y"), "x [redacted] y");
    }

    #[test]
    fn header_values_are_replaced() {
        // ureq's debug dump of a request prelude.
        let prelude = "writing prelude: POST /v1/messages HTTP/1.1\r\nHost: api.anthropic.com\r\nx-api-key: sk-ant-unregistered-1\r\nanthropic-version: 2026-10-21\r\nAuthorization: Key fal:abc/def\r\n";
        let r = redact(prelude);
        assert!(!r.contains("sk-ant-unregistered-1"), "{r}");
        assert!(!r.contains("fal:abc/def"), "{r}");
        assert!(r.contains("x-api-key: [redacted]\r\n"));
        assert!(r.contains("Authorization: [redacted]\r\n"));
        assert!(r.contains("Host: api.anthropic.com"));
        assert!(r.contains("anthropic-version: 2026-10-21"));

        assert_eq!(redact("XI-API-KEY:el_secret_value"), "XI-API-KEY:[redacted]");
        assert_eq!(redact(r#"{"xi-api-key": "el_json_secret", "a": 1}"#), r#"{"xi-api-key": "[redacted]", "a": 1}"#);
        assert_eq!(redact("GET /v1?api_key=abc123&x=1"), "GET /v1?api_key=[redacted]&x=1");
        assert_eq!(redact("authorization: Bearer abc.def\nnext"), "authorization: [redacted]\nnext");
        // Mentions without a value are left alone.
        assert_eq!(redact("the api-key is missing"), "the api-key is missing");
        assert_eq!(redact("Authorization: ***"), "Authorization: ***");
    }

    #[test]
    fn writer_redacts_whole_lines_even_when_split() {
        register_secret("writer-secret-0123456789");
        let mut out = Vec::new();
        {
            let mut w = RedactingWriter::new(&mut out);
            w.write_all(b"a writer-sec").unwrap();
            w.write_all(b"ret-0123456789 b\n").unwrap();
            w.write_all(b"x-api-key: tail-without-newline").unwrap();
        }
        let s = String::from_utf8(out).unwrap();
        assert_eq!(s, "a [redacted] b\nx-api-key: [redacted]");
    }
}
