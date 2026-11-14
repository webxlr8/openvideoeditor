//! Endpoint overrides (`OVE_ANTHROPIC_BASE_URL`, `OVE_FAL_QUEUE_URL`, `OVE_OPENAI_URL`, `OVE_ELEVENLABS_URL`) point a
//! client at a local mock in tests and development. Whoever sets one receives the API key with every request, so:
//!
//! - they are honoured only in debug builds (tests, `cargo run`), or when [`DEV_FLAG`] is `1`;
//! - the URL must be `https`, or `http` to the local machine (`localhost`, `127.0.0.1`, `[::1]`), with no
//!   user-info part.
//!
//! Anything else is ignored with a warning and the provider's real endpoint is used.

/// Set to `1` to allow endpoint overrides in a release build (local end-to-end testing).
pub const DEV_FLAG: &str = "OVE_DEV_ENDPOINTS";

/// Scheme, host and port of a URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    /// `http` or `https`, lower case.
    pub scheme: String,
    /// Lower case, without brackets for IPv6.
    pub host: String,
    pub port: Option<u16>,
}

impl Origin {
    /// The port, or the scheme's default.
    pub fn effective_port(&self) -> u16 {
        self.port.unwrap_or(if self.scheme == "https" { 443 } else { 80 })
    }

    /// Same scheme, host and port.
    pub fn same_as(&self, other: &Origin) -> bool {
        self.scheme == other.scheme && self.host == other.host && self.effective_port() == other.effective_port()
    }

    /// `host` is `suffix` or a subdomain of it.
    pub fn host_within(&self, suffix: &str) -> bool {
        self.host == suffix
            || (self.host.len() > suffix.len()
                && self.host.ends_with(suffix)
                && self.host.as_bytes()[self.host.len() - suffix.len() - 1] == b'.')
    }
}

/// The origin of an `http`/`https` URL; `None` for other schemes, user-info (`user@host`), or anything malformed.
pub fn origin(url: &str) -> Option<Origin> {
    let (scheme, rest) = url.trim().split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains(['@', '\\']) || authority.chars().any(char::is_whitespace) {
        return None;
    }
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (h, after) = v6.split_once(']')?;
        let port = match after {
            "" => None,
            p => Some(p.strip_prefix(':')?.parse::<u16>().ok()?),
        };
        (h.to_string(), port)
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), Some(p.parse::<u16>().ok()?)),
            None => (authority.to_string(), None),
        }
    };
    let host = host.to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    Some(Origin { scheme, host, port })
}

/// `localhost`, `127.0.0.1` or `::1`.
pub fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

/// `https` anywhere, or `http` to this machine.
pub fn is_acceptable_base(url: &str) -> bool {
    match origin(url) {
        Some(o) if o.scheme == "https" => true,
        Some(o) => is_loopback_host(&o.host),
        None => false,
    }
}

/// Whether this build accepts endpoint overrides.
pub fn overrides_allowed() -> bool {
    cfg!(debug_assertions) || std::env::var(DEV_FLAG).is_ok_and(|v| v.trim() == "1")
}

/// The base URL for a service: `default`, or the override in environment variable `var` when this build allows
/// overrides and the URL is acceptable. No trailing slash.
pub fn base_url(var: &str, default: &str) -> String {
    let value = std::env::var(var).ok();
    resolve(var, value.as_deref(), default, overrides_allowed())
}

/// [`base_url`] with the environment value and the build policy passed in.
pub fn resolve(var: &str, value: Option<&str>, default: &str, allowed: bool) -> String {
    let default = default.trim_end_matches('/').to_string();
    let Some(v) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return default;
    };
    if !allowed {
        tracing::warn!(var, "ignoring the endpoint override: release builds accept it only with {DEV_FLAG}=1");
        return default;
    }
    if !is_acceptable_base(v) {
        tracing::warn!(var, "ignoring the endpoint override: it must be an https URL (http only to localhost)");
        return default;
    }
    v.trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_origins() {
        let o = origin("https://Queue.Fal.Run/fal-ai/flux?x=1").unwrap();
        assert_eq!((o.scheme.as_str(), o.host.as_str(), o.port), ("https", "queue.fal.run", None));
        assert_eq!(o.effective_port(), 443);
        assert_eq!(origin("http://127.0.0.1:8787").unwrap().port, Some(8787));
        assert_eq!(origin("http://[::1]:9/x").unwrap().host, "::1");
        assert!(origin("https://user:pw@fal.run/x").is_none());
        assert!(origin("https://fal.run@evil.example/x").is_none());
        assert!(origin("ftp://fal.run").is_none());
        assert!(origin("https://:443").is_none());
        assert!(origin("https://fal.run:notaport").is_none());
        assert!(origin("fal.run/x").is_none());
    }

    #[test]
    fn host_suffixes() {
        let h = |u: &str| origin(u).unwrap();
        assert!(h("https://fal.run").host_within("fal.run"));
        assert!(h("https://queue.fal.run").host_within("fal.run"));
        assert!(!h("https://evilfal.run").host_within("fal.run"));
        assert!(!h("https://fal.run.evil.example").host_within("fal.run"));
    }

    #[test]
    fn override_policy() {
        let d = "https://api.example.com/";
        assert_eq!(resolve("V", None, d, true), "https://api.example.com");
        assert_eq!(resolve("V", Some(""), d, true), "https://api.example.com");
        assert_eq!(resolve("V", Some("http://127.0.0.1:9/"), d, true), "http://127.0.0.1:9");
        assert_eq!(resolve("V", Some("http://localhost:9"), d, true), "http://localhost:9");
        assert_eq!(resolve("V", Some("https://staging.example.com"), d, true), "https://staging.example.com");
        // Plain http to another machine, odd schemes and user-info are refused.
        assert_eq!(resolve("V", Some("http://10.0.0.5:8080"), d, true), "https://api.example.com");
        assert_eq!(resolve("V", Some("http://localhost.evil.example"), d, true), "https://api.example.com");
        assert_eq!(resolve("V", Some("https://a@evil.example"), d, true), "https://api.example.com");
        assert_eq!(resolve("V", Some("file:///etc"), d, true), "https://api.example.com");
        // Release builds without the flag ignore overrides altogether.
        assert_eq!(resolve("V", Some("https://evil.example"), d, false), "https://api.example.com");
        assert_eq!(resolve("V", Some("http://127.0.0.1:9"), d, false), "https://api.example.com");
    }
}
