//! What an agent session keeps between calls (change unify-agent-tool-layer, D7 and D8): generation quotes and the
//! spend the person confirmed, the names of the last tools called and the last error code (both go into feedback
//! reports), and the feedback reports the session filed.
//!
//! A [`Session`] is a cheap handle: clones share one state. Each `ove mcp` process has one; the app keeps one per
//! chat, because its tool context is built anew for every call.
//!
//! The spend cap is a setting of the person's, read from `<config>/generation.json` on every check, so a change in
//! Settings applies to running sessions at once.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use ove_core::Clock;
use ove_generate::keys::KeyStore;
use serde_json::{Value, json};

use crate::fail::Fail;

/// How long a quote stays valid.
pub const QUOTE_TTL_MS: i64 = 10 * 60_000;
/// The spend cap per session when the person set none (USD).
pub const DEFAULT_CAP_USD: f64 = 5.0;
/// Tool names a feedback report lists.
pub const RECENT_TOOLS: usize = 15;
/// Feedback reports one session may file.
pub const FEEDBACK_PER_SESSION: usize = 8;
/// The settings file in the configuration folder.
pub const SETTINGS_FILE: &str = "generation.json";

/// The app's configuration folder: `OVE_CONFIG_DIR`, else the app data folder (`%APPDATA%` on Windows,
/// `~/Library/Application Support` on macOS, plus the app identifier). The app, `ove` and `ove mcp` use the same.
pub fn config_dir() -> Option<PathBuf> {
    std::env::var_os("OVE_CONFIG_DIR").filter(|v| !v.is_empty()).map(PathBuf::from).or_else(ove_models::app_data_dir)
}

/// The per-session spend cap in `config` (USD).
pub fn spend_cap_usd(config: Option<&Path>) -> f64 {
    config
        .and_then(|d| std::fs::read(d.join(SETTINGS_FILE)).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.get("sessionCapUsd").and_then(Value::as_f64))
        .filter(|v| v.is_finite() && *v >= 0.0)
        .unwrap_or(DEFAULT_CAP_USD)
}

/// Saves the per-session spend cap (keeps the file's other settings).
pub fn set_spend_cap_usd(config: &Path, usd: f64) -> Result<f64, Fail> {
    if !(usd.is_finite() && (0.0..=10_000.0).contains(&usd)) {
        return Err(Fail::new("argument.invalid", "the spend cap is an amount from $0 to $10,000"));
    }
    let usd = (usd * 100.0).round() / 100.0;
    let path = config.join(SETTINGS_FILE);
    let mut v = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    v["sessionCapUsd"] = json!(usd);
    std::fs::create_dir_all(config).map_err(|e| Fail::new("file.unwritable", e.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&v).unwrap_or_default())
        .and_then(|()| std::fs::rename(&tmp, &path))
        .map_err(|e| Fail::new("file.unwritable", e.to_string()))?;
    Ok(usd)
}

/// A priced request waiting for a person's approval.
#[derive(Debug, Clone)]
pub struct Quote {
    pub id: String,
    /// Hash of the request it priced (tool, project and arguments).
    pub hash: String,
    pub estimate_usd: f64,
    pub created_ms: i64,
    /// What the person is shown.
    pub card: Value,
    used: bool,
}

impl Quote {
    pub fn expires_ms(&self) -> i64 {
        self.created_ms + QUOTE_TTL_MS
    }
}

#[derive(Default)]
struct State {
    quotes: BTreeMap<String, Quote>,
    spent_usd: f64,
    recent: VecDeque<String>,
    last_error: Option<String>,
    /// Fingerprints of the feedback reports filed, in order.
    feedback: Vec<String>,
    counter: u64,
}

/// One agent session's state (see the module documentation).
#[derive(Clone)]
pub struct Session {
    state: Arc<Mutex<State>>,
    clock: Arc<dyn Clock>,
    config: Option<PathBuf>,
    keys: Arc<dyn KeyStore>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").field("config", &self.config).finish()
    }
}

impl Default for Session {
    fn default() -> Self {
        Session::new()
    }
}

impl Session {
    /// A session with the system clock, the usual configuration folder and the system credential store.
    pub fn new() -> Session {
        Session::with(config_dir(), Arc::new(ove_core::SystemClock), Arc::new(ove_generate::keys::OsKeyStore))
    }

    pub fn with(config: Option<PathBuf>, clock: Arc<dyn Clock>, keys: Arc<dyn KeyStore>) -> Session {
        Session { state: Arc::default(), clock, config, keys }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn now(&self) -> i64 {
        self.clock.now_ms()
    }

    pub fn config(&self) -> Option<&Path> {
        self.config.as_deref()
    }

    /// Provider keys (only checked for presence here; the job's worker reads them itself).
    pub fn keys(&self) -> &dyn KeyStore {
        self.keys.as_ref()
    }

    /// The person's spend cap for a session (USD).
    pub fn cap_usd(&self) -> f64 {
        spend_cap_usd(self.config.as_deref())
    }

    /// What the person confirmed so far in this session (USD).
    pub fn spent_usd(&self) -> f64 {
        self.lock().spent_usd
    }

    fn cap_check(spent: f64, estimate: f64, cap: f64) -> Result<(), Fail> {
        if spent + estimate > cap + 1e-9 {
            return Err(Fail::new(
                "spend.cap",
                format!(
                    "this would take the session's spend to ${:.2}, above its cap of ${cap:.2} (Settings → Generation); nothing was spent",
                    spent + estimate
                ),
            )
            .details(json!({"capUsd": cap, "spentUsd": spent, "estimateUsd": estimate})));
        }
        Ok(())
    }

    /// Refused with `spend.cap` when `estimate` would take the session above its cap.
    pub fn check_cap(&self, estimate: f64) -> Result<(), Fail> {
        let cap = self.cap_usd();
        Self::cap_check(self.spent_usd(), estimate, cap)
    }

    /// Records a new quote and returns it. Expired quotes are dropped.
    pub fn quote(&self, hash: String, estimate_usd: f64, card: Value) -> Quote {
        let now = self.now();
        let mut s = self.lock();
        s.quotes.retain(|_, q| now < q.expires_ms() && !q.used);
        s.counter += 1;
        // The job id's random part makes quote ids hard to guess.
        let random = ove_generate::job::new_id(now);
        let id = format!("q_{now:x}_{}{}", s.counter, random.rsplit('_').next().unwrap_or_default());
        let q = Quote { id: id.clone(), hash, estimate_usd, created_ms: now, card, used: false };
        s.quotes.insert(id, q.clone());
        q
    }

    /// The quote `id` when it is valid for the request with `hash`: refused with `quote.unknown`, `quote.used`,
    /// `quote.expired` or `quote.mismatch`.
    pub fn redeem(&self, id: &str, hash: &str) -> Result<Quote, Fail> {
        let now = self.now();
        let s = self.lock();
        let q = s.quotes.get(id).ok_or_else(|| {
            Fail::new(
                "quote.unknown",
                format!("there is no quote {id} in this session; call the tool without `confirm` to get one"),
            )
        })?;
        if q.used {
            return Err(Fail::new("quote.used", format!("quote {id} was already spent; ask for a new quote")));
        }
        if now >= q.expires_ms() {
            return Err(Fail::new(
                "quote.expired",
                format!("quote {id} expired (quotes last {} minutes); ask for a new one", QUOTE_TTL_MS / 60_000),
            ));
        }
        if q.hash != hash {
            return Err(Fail::new(
                "quote.mismatch",
                format!("quote {id} priced a different request; a quote is valid only for the exact request it priced"),
            ));
        }
        Ok(q.clone())
    }

    /// Spends a quote the person approved: marks it used and adds its estimate to the session's spend. Checks the
    /// quote and the cap again (another call may have spent meanwhile).
    pub fn commit(&self, id: &str) -> Result<f64, Fail> {
        let now = self.now();
        let cap = self.cap_usd();
        let mut s = self.lock();
        let spent = s.spent_usd;
        let q = s.quotes.get_mut(id).ok_or_else(|| Fail::new("quote.unknown", format!("there is no quote {id}")))?;
        if q.used {
            return Err(Fail::new("quote.used", format!("quote {id} was already spent")));
        }
        if now >= q.expires_ms() {
            return Err(Fail::new(
                "quote.expired",
                format!("quote {id} expired before it was approved; ask for a new one"),
            ));
        }
        Self::cap_check(spent, q.estimate_usd, cap)?;
        q.used = true;
        let estimate = q.estimate_usd;
        s.spent_usd += estimate;
        Ok(s.spent_usd)
    }

    /// Takes back a committed quote's estimate when its job could not start (nothing was spent). The quote stays used.
    pub fn refund(&self, id: &str) {
        let mut s = self.lock();
        if let Some(est) = s.quotes.get(id).filter(|q| q.used).map(|q| q.estimate_usd) {
            s.spent_usd = (s.spent_usd - est).max(0.0);
        }
    }

    /// Records a finished call (for feedback reports).
    pub fn note_call(&self, tool: &str, error: Option<&str>) {
        let mut s = self.lock();
        if s.recent.len() == RECENT_TOOLS {
            s.recent.pop_front();
        }
        s.recent.push_back(tool.to_string());
        if let Some(e) = error {
            s.last_error = Some(e.to_string());
        }
    }

    /// The last tools called, oldest first.
    pub fn recent_tools(&self) -> Vec<String> {
        self.lock().recent.iter().cloned().collect()
    }

    /// The code of the last refused call.
    pub fn last_error(&self) -> Option<String> {
        self.lock().last_error.clone()
    }

    /// Feedback reports filed by this session.
    pub fn feedback_filed(&self) -> usize {
        self.lock().feedback.len()
    }

    /// True when this session already filed a report with `fingerprint`.
    pub fn filed(&self, fingerprint: &str) -> bool {
        self.lock().feedback.iter().any(|f| f == fingerprint)
    }

    /// Records a filed report.
    pub fn note_feedback(&self, fingerprint: String) {
        self.lock().feedback.push(fingerprint);
    }
}

/// FNV-1a (64-bit) of `text`, as hex: binds a quote to the request it priced (not a secret).
pub fn fnv1a(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// `v` as JSON text with object keys sorted at every level (the same request always gives the same text).
pub fn canonical(v: &Value) -> String {
    fn sort(v: &Value) -> Value {
        match v {
            Value::Object(m) => {
                let sorted: BTreeMap<&String, Value> = m.iter().map(|(k, v)| (k, sort(v))).collect();
                let mut out = serde_json::Map::new();
                for (k, v) in sorted {
                    out.insert(k.clone(), v);
                }
                Value::Object(out)
            }
            Value::Array(a) => Value::Array(a.iter().map(sort).collect()),
            other => other.clone(),
        }
    }
    sort(v).to_string()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, Ordering};

    use super::*;

    struct Manual(AtomicI64);

    impl Clock for Manual {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    fn session(dir: &Path) -> (Session, Arc<Manual>) {
        let clock = Arc::new(Manual(AtomicI64::new(1_000_000)));
        let s = Session::with(
            Some(dir.to_path_buf()),
            clock.clone(),
            Arc::new(ove_generate::keys::MemoryKeyStore::default()),
        );
        (s, clock)
    }

    #[test]
    fn quotes_expire_bind_to_their_request_and_spend_once() {
        let d = tempfile::tempdir().unwrap();
        let (s, clock) = session(d.path());
        let q = s.quote("h1".into(), 0.5, json!({}));
        assert_eq!(s.redeem(&q.id, "h2").unwrap_err().code, "quote.mismatch");
        assert_eq!(s.redeem("q_nope", "h1").unwrap_err().code, "quote.unknown");
        assert_eq!(s.redeem(&q.id, "h1").unwrap().estimate_usd, 0.5);
        assert_eq!(s.commit(&q.id).unwrap(), 0.5);
        assert_eq!(s.redeem(&q.id, "h1").unwrap_err().code, "quote.used");
        assert_eq!(s.commit(&q.id).unwrap_err().code, "quote.used");

        let q = s.quote("h3".into(), 0.5, json!({}));
        clock.0.fetch_add(QUOTE_TTL_MS - 1, Ordering::SeqCst);
        assert!(s.redeem(&q.id, "h3").is_ok());
        clock.0.fetch_add(1, Ordering::SeqCst);
        assert_eq!(s.redeem(&q.id, "h3").unwrap_err().code, "quote.expired");
        assert_eq!(s.commit(&q.id).unwrap_err().code, "quote.expired");
        assert_eq!(s.spent_usd(), 0.5);
    }

    #[test]
    fn the_cap_comes_from_settings_and_counts_confirmed_spend() {
        let d = tempfile::tempdir().unwrap();
        let (s, _) = session(d.path());
        assert_eq!(s.cap_usd(), DEFAULT_CAP_USD);
        set_spend_cap_usd(d.path(), 1.0).unwrap();
        assert_eq!(s.cap_usd(), 1.0, "a changed setting applies to a running session");
        let a = s.quote("a".into(), 0.75, json!({}));
        s.commit(&a.id).unwrap();
        assert_eq!(s.check_cap(0.25), Ok(()));
        let f = s.check_cap(0.3).unwrap_err();
        assert_eq!(f.code, "spend.cap");
        assert_eq!(f.details["capUsd"], 1.0);
        let b = s.quote("b".into(), 0.3, json!({}));
        assert_eq!(s.commit(&b.id).unwrap_err().code, "spend.cap", "checked again when spending");
        assert_eq!(s.spent_usd(), 0.75);
        // A raised cap lets it through.
        set_spend_cap_usd(d.path(), 2.0).unwrap();
        assert_eq!(s.commit(&b.id).unwrap(), 1.05);
        assert!(set_spend_cap_usd(d.path(), -1.0).is_err());
        assert!(set_spend_cap_usd(d.path(), f64::NAN).is_err());
        // Other settings in the file are kept.
        std::fs::write(d.path().join(SETTINGS_FILE), r#"{"other": 1, "sessionCapUsd": 3}"#).unwrap();
        set_spend_cap_usd(d.path(), 4.0).unwrap();
        let v: Value = serde_json::from_slice(&std::fs::read(d.path().join(SETTINGS_FILE)).unwrap()).unwrap();
        assert_eq!(v, json!({"other": 1, "sessionCapUsd": 4.0}));
    }

    #[test]
    fn recent_tools_keep_the_last_fifteen() {
        let d = tempfile::tempdir().unwrap();
        let (s, _) = session(d.path());
        for n in 0..20 {
            s.note_call(&format!("t{n}"), if n == 7 { Some("edit.anchor") } else { None });
        }
        let r = s.recent_tools();
        assert_eq!(r.len(), RECENT_TOOLS);
        assert_eq!((r[0].as_str(), r[14].as_str()), ("t5", "t19"));
        assert_eq!(s.last_error().as_deref(), Some("edit.anchor"));
    }

    #[test]
    fn canonical_text_ignores_key_order() {
        let a = json!({"b": 1, "a": {"y": [1, {"d": 2, "c": 3}], "x": null}});
        let b = json!({"a": {"x": null, "y": [1, {"c": 3, "d": 2}]}, "b": 1});
        assert_eq!(canonical(&a), canonical(&b));
        assert_eq!(fnv1a(&canonical(&a)), fnv1a(&canonical(&b)));
        assert_ne!(fnv1a("a"), fnv1a("b"));
    }
}
