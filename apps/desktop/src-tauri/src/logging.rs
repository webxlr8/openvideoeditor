//! Rotating file logs and the process-wide panic hook (design D7).

use std::path::{Path, PathBuf};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Days of logs kept.
pub const MAX_LOG_FILES: usize = 7;

/// Keeps the background log writer alive; dropping it flushes.
pub struct LogGuard {
    _guard: Option<WorkerGuard>,
    pub dir: Option<PathBuf>,
}

/// Starts logging to `<dir>/ove.<date>.log` (daily rotation, 7 files) and stderr.
/// Falls back to stderr only if the directory cannot be created.
pub fn init(dir: &Path) -> LogGuard {
    let filter = EnvFilter::try_from_env("OVE_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let stderr = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);
    let file = std::fs::create_dir_all(dir).ok().and_then(|_| {
        Builder::new()
            .rotation(Rotation::DAILY)
            .filename_prefix("ove")
            .filename_suffix("log")
            .max_log_files(MAX_LOG_FILES)
            .build(dir)
            .ok()
    });
    let guard = match file {
        Some(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let file_layer = tracing_subscriber::fmt::layer().with_ansi(false).with_writer(writer);
            let _ = tracing_subscriber::registry().with(filter).with(stderr).with(file_layer).try_init();
            LogGuard { _guard: Some(guard), dir: Some(dir.to_path_buf()) }
        }
        None => {
            let _ = tracing_subscriber::registry().with(filter).with(stderr).try_init();
            tracing::warn!(dir = %dir.display(), "log directory unavailable; logging to stderr only");
            LogGuard { _guard: None, dir: None }
        }
    };
    install_panic_hook();
    guard
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
    #[test]
    fn startup_line_names_os_gpu_and_decode() {
        let line = super::startup_line("TestGPU", "software");
        assert!(line.contains("GPU TestGPU"));
        assert!(line.contains("decode software"));
        assert!(line.contains(std::env::consts::ARCH));
    }
}
