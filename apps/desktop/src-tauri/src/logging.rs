//! Rotating file logs and the process-wide panic hook (design D7).
//!
//! API keys must never reach a log file, whatever `OVE_LOG` says. Two independent guards:
//!
//! - HTTP, TLS and credential-store internals (`ureq`, `rustls`, `keyring`, …) are capped at `warn`. `ureq` dumps
//!   every request it sends at `debug`, headers included, and hides only `Authorization` and `Cookie`, not
//!   `x-api-key` / `xi-api-key`.
//! - Both outputs (file and stderr) write through [`ove_guard::RedactingWriter`], which replaces every registered key
//!   (keys register themselves when loaded) and any value after an `authorization` / `api-key` header name.

use std::path::{Path, PathBuf};

use ove_guard::RedactingWriter;
use tracing::{Level, Metadata};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::{FilterFn, filter_fn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Days of logs kept.
pub const MAX_LOG_FILES: usize = 7;

/// Log targets of HTTP, TLS and credential-store internals: never more verbose than `warn` (their debug output
/// includes request headers).
pub const QUIET_TARGETS: &[&str] = &[
    "ureq",
    "rustls",
    "hyper",
    "hyper_util",
    "h2",
    "reqwest",
    "tungstenite",
    "tokio_tungstenite",
    "keyring",
    "secret_service",
    "dbus_secret_service",
    "dbus",
];

fn is_quiet_target(target: &str) -> bool {
    QUIET_TARGETS.iter().any(|t| target.strip_prefix(t).is_some_and(|rest| rest.is_empty() || rest.starts_with("::")))
}

/// The level filter: `spec` (`OVE_LOG`; default `info`) with a `warn` directive for each of [`QUIET_TARGETS`]
/// appended (replacing the same directive in `spec`).
pub fn env_filter(spec: Option<&str>) -> EnvFilter {
    let mut filter = spec.and_then(|s| EnvFilter::try_new(s).ok()).unwrap_or_else(|| EnvFilter::new("info"));
    for t in QUIET_TARGETS {
        if let Ok(d) = format!("{t}=warn").parse() {
            filter = filter.add_directive(d);
        }
    }
    filter
}

/// A hard cap on [`QUIET_TARGETS`] that a more specific directive (`OVE_LOG=ureq::unit=debug`) cannot lift.
pub fn quiet_http_internals() -> FilterFn<impl Fn(&Metadata<'_>) -> bool> {
    filter_fn(|meta: &Metadata<'_>| !(is_quiet_target(meta.target()) && *meta.level() > Level::WARN))
}

/// Keeps the background log writer alive; dropping it flushes.
pub struct LogGuard {
    _guard: Option<WorkerGuard>,
    pub dir: Option<PathBuf>,
}

/// Starts logging to `<dir>/ove.<date>.log` (daily rotation, 7 files) and stderr.
/// Falls back to stderr only if the directory cannot be created.
pub fn init(dir: &Path) -> LogGuard {
    let filter = env_filter(std::env::var("OVE_LOG").ok().as_deref());
    let stderr = tracing_subscriber::fmt::layer().with_writer(|| RedactingWriter::new(std::io::stderr()));
    let file = std::fs::create_dir_all(dir).ok().and_then(|_| {
        Builder::new()
            .rotation(Rotation::DAILY)
            .filename_prefix("ove")
            .filename_suffix("log")
            .max_log_files(MAX_LOG_FILES)
            .build(dir)
            .ok()
    });
    let base = tracing_subscriber::registry().with(filter).with(quiet_http_internals());
    let guard = match file {
        Some(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let file_layer = tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(move || RedactingWriter::new(writer.clone()));
            let _ = base.with(stderr).with(file_layer).try_init();
            LogGuard { _guard: Some(guard), dir: Some(dir.to_path_buf()) }
        }
        None => {
            let _ = base.with(stderr).try_init();
            tracing::warn!(dir = %dir.display(), "log directory unavailable; logging to stderr only");
            LogGuard { _guard: None, dir: None }
        }
    };
    let _ = CRASH_DIR.set(dir.join("crash-reports"));
    install_panic_hook();
    guard
}

/// Where panic reports are written (set once logging starts).
pub static CRASH_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Writes a self-contained report for a panic: version, OS, thread, message, location, backtrace. Kept on this
/// computer only (Settings → Privacy shows them).
fn write_crash_report(thread: &str, message: &str, location: &str, backtrace: &str) {
    let Some(dir) = CRASH_DIR.get() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let os = os_info::get();
    let text = format!(
        "OpenVideoEditor {}\nOS: {} {} ({})\nArch: {}\nTime (ms since epoch): {now}\nThread: {thread}\nLocation: {location}\nMessage: {message}\n\nBacktrace:\n{backtrace}\n",
        env!("CARGO_PKG_VERSION"),
        os.os_type(),
        os.version(),
        os.bitness(),
        std::env::consts::ARCH,
    );
    let _ = std::fs::write(dir.join(format!("crash-{now}.txt")), text);
    // Keep the newest 20.
    if let Ok(rd) = std::fs::read_dir(dir) {
        let mut files: Vec<PathBuf> =
            rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "txt")).collect();
        files.sort();
        let excess = files.len().saturating_sub(20);
        for f in files.into_iter().take(excess) {
            let _ = std::fs::remove_file(f);
        }
    }
}

/// Logs every panic (any thread) with its location and a captured backtrace.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<unnamed>");
        let message = crate::worker::panic_message(info.payload());
        let location = info.location().map(|l| l.to_string()).unwrap_or_default();
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!(thread = name, %location, "panic: {message}\nbacktrace:\n{backtrace}");
        write_crash_report(name, &message, &location, &backtrace.to_string());
    }));
}

/// One line with what the support team needs first.
pub fn startup_line(gpu: &str, decode: &str) -> String {
    let os = os_info::get();
    format!(
        "startup: app {} | OS {} {} ({}) | arch {} | GPU {} | decode {}",
        env!("CARGO_PKG_VERSION"),
        os.os_type(),
        os.version(),
        os.bitness(),
        std::env::consts::ARCH,
        gpu,
        decode
    )
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::sync::{Arc, Mutex};

    use super::*;

    #[test]
    fn startup_line_names_os_gpu_and_decode() {
        let line = super::startup_line("TestGPU", "software");
        assert!(line.contains("GPU TestGPU"));
        assert!(line.contains("decode software"));
        assert!(line.contains(std::env::consts::ARCH));
    }

    #[test]
    fn quiet_targets_match_whole_crate_names() {
        assert!(is_quiet_target("ureq"));
        assert!(is_quiet_target("ureq::unit"));
        assert!(is_quiet_target("rustls::client::hs"));
        assert!(!is_quiet_target("ureqx"));
        assert!(!is_quiet_target("ove_desktop_lib::generation"));
    }

    /// Log output captured in memory.
    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl Write for Captured {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Captured {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
        }
    }

    /// A one-shot HTTP server that answers `200 {}` and returns the request head it received.
    fn one_shot_server() -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut head = String::new();
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                head.push_str(&line);
                if line.trim().is_empty() {
                    break;
                }
            }
            let mut body = vec![0; len];
            r.read_exact(&mut body).unwrap();
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").unwrap();
            head
        });
        (url, handle)
    }

    /// Sends a request carrying `key` in `x-api-key` and `xi-api-key` while `subscriber` is the default, with `log`
    /// records bridged into tracing as in the app.
    fn request_with_key(subscriber: impl tracing::Subscriber + Send + Sync + 'static, key: &str) {
        let _ = tracing_log::LogTracer::init();
        let (url, server) = one_shot_server();
        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!(target: "ove_test", "about to call {url}");
            let r = ureq::post(&format!("{url}/v1/messages")).set("x-api-key", key).set("xi-api-key", key).call();
            assert!(r.is_ok(), "{r:?}");
        });
        let head = server.join().unwrap();
        assert!(head.contains(key), "the key must still be sent: {head}");
    }

    fn captured_layer<S>(out: &Captured) -> impl tracing_subscriber::Layer<S>
    where
        S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    {
        let w = out.clone();
        tracing_subscriber::fmt::layer().with_ansi(false).with_writer(move || RedactingWriter::new(w.clone()))
    }

    #[test]
    fn debug_logging_never_writes_api_keys() {
        // The app's subscriber with OVE_LOG asking for everything: HTTP internals capped, output redacted.
        let key = "sk-ant-api03-log-leak-test-key-1";
        let out = Captured::default();
        let subscriber = tracing_subscriber::registry()
            .with(env_filter(Some("debug,ureq=trace,ureq::unit=trace")))
            .with(quiet_http_internals())
            .with(captured_layer(&out));
        request_with_key(subscriber, key);
        let text = out.text();
        assert!(text.contains("about to call"), "debug logging is on: {text}");
        assert!(!text.contains(key), "key in the log: {text}");
        assert!(!text.contains("writing prelude"), "ureq's request dump is capped: {text}");
    }

    #[test]
    fn redaction_alone_catches_unregistered_keys_in_request_dumps() {
        // The second guard on its own: no level cap, a key nobody registered, still not written.
        let key = "el-unregistered-log-leak-test-key-2";
        let out = Captured::default();
        let subscriber = tracing_subscriber::registry().with(EnvFilter::new("trace")).with(captured_layer(&out));
        request_with_key(subscriber, key);
        let text = out.text();
        assert!(text.contains("writing prelude"), "ureq's dump reached the formatter: {text}");
        assert!(!text.contains(key), "key in the log: {text}");
        assert!(text.contains("x-api-key: [redacted]"), "{text}");

        // A key registered by a key store is caught anywhere, in messages and fields.
        ove_guard::register_secret("fal-registered-0123456789abcdef");
        let out = Captured::default();
        let subscriber = tracing_subscriber::registry().with(captured_layer(&out));
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(detail = "fal-registered-0123456789abcdef", "provider said fal-registered-0123456789abcdef")
        });
        assert!(out.text().contains("provider said [redacted]"), "{}", out.text());
        assert!(!out.text().contains("fal-registered"), "{}", out.text());
    }
}
