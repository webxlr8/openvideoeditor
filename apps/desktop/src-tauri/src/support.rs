//! Distribution support (roadmap change 16): crash reports kept on this computer, and the update check.
//!
//! Updates: a build made with `OVE_UPDATE_URL` set checks that URL for a JSON manifest
//! `{"version": "0.2.0", "notes": "…", "url": "https://…/download"}` and offers the download page. Installing in
//! place (Tauri's signed updater) needs the owner's signing keys and is enabled with them; see docs/03 §6.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult, ErrorCode};
use crate::worker;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CrashReport {
    pub path: String,
    /// Milliseconds since the epoch.
    pub at: f64,
    /// First line of the message.
    pub summary: String,
    pub seen: bool,
}

fn crash_dir() -> Option<PathBuf> {
    crate::logging::CRASH_DIR.get().cloned()
}

fn seen_marker() -> Option<PathBuf> {
    crash_dir().map(|d| d.join(".seen"))
}

/// Reports on this computer, newest first.
#[tauri::command]
#[specta::specta]
pub async fn crash_reports() -> Vec<CrashReport> {
    // An unreadable folder (or an internal error) just shows no reports.
    worker::blocking("crash_reports", || Ok(reports())).await.unwrap_or_default()
}

fn reports() -> Vec<CrashReport> {
    let Some(dir) = crash_dir() else { return vec![] };
    let seen_until: f64 =
        seen_marker().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| s.trim().parse().ok()).unwrap_or(0.0);
    let mut v: Vec<CrashReport> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let p = e.path();
                    let at: f64 = p.file_stem()?.to_str()?.strip_prefix("crash-")?.parse().ok()?;
                    let text = std::fs::read_to_string(&p).ok()?;
                    let summary = text
                        .lines()
                        .find_map(|l| l.strip_prefix("Message: "))
                        .unwrap_or("")
                        .chars()
                        .take(200)
                        .collect();
                    Some(CrashReport { path: p.display().to_string(), at, summary, seen: at <= seen_until })
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| b.at.total_cmp(&a.at));
    v
}

#[tauri::command]
#[specta::specta]
pub async fn mark_crash_reports_seen() -> AppResult<()> {
    worker::blocking("mark_crash_reports_seen", move || {
        let newest = reports().first().map(|r| r.at).unwrap_or(0.0);
        if let Some(p) = seen_marker() {
            std::fs::write(p, format!("{newest}")).map_err(|e| AppError::new(ErrorCode::Io, e.to_string()))?;
        }
        Ok(())
    })
    .await
}

/// The text of one report (to copy into feedback).
#[tauri::command]
#[specta::specta]
pub async fn read_crash_report(path: String) -> AppResult<String> {
    worker::blocking("read_crash_report", move || {
        let dir = crash_dir().ok_or_else(|| AppError::new(ErrorCode::NotFound, "No crash reports."))?;
        let p = PathBuf::from(&path);
        if p.parent() != Some(dir.as_path()) {
            return Err(AppError::new(ErrorCode::InvalidArgument, "Not a crash report."));
        }
        std::fs::read_to_string(p).map_err(|e| AppError::new(ErrorCode::Io, e.to_string()))
    })
    .await
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current: String,
    pub latest: Option<String>,
    pub available: bool,
    pub notes: Option<String>,
    pub url: Option<String>,
    /// Why there is no answer (not configured, offline, …).
    pub message: Option<String>,
}

/// `a > b` for dotted numeric versions (pre-release suffixes ignored).
pub fn newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v').split(['.', '-', '+']).take(3).map(|x| x.parse().unwrap_or(0)).collect()
    };
    parse(a) > parse(b)
}

pub fn update_url() -> Option<String> {
    std::env::var("OVE_UPDATE_URL")
        .ok()
        .or_else(|| option_env!("OVE_UPDATE_URL").map(str::to_string))
        .filter(|u| u.starts_with("https://") || u.starts_with("http://127.0.0.1"))
}

#[tauri::command]
#[specta::specta]
pub async fn check_for_updates() -> AppResult<UpdateInfo> {
    worker::blocking("check_for_updates", move || {
        let current = env!("CARGO_PKG_VERSION").to_string();
        let Some(url) = update_url() else {
            return Ok(UpdateInfo {
                current,
                latest: None,
                available: false,
                notes: None,
                url: None,
                message: Some(
                    "This build has no update channel; download new versions from where you got this one.".into(),
                ),
            });
        };
        {
            let agent = ove_guard::http::agent_builder().timeout(std::time::Duration::from_secs(10)).build();
            let res = agent
                .get(&url)
                .call()
                .map_err(|e| e.to_string())
                .and_then(|r| r.into_string().map_err(|e| e.to_string()));
            Ok(match res.and_then(|t| serde_json::from_str::<serde_json::Value>(&t).map_err(|e| e.to_string())) {
                Ok(v) => {
                    let latest = v.get("version").and_then(|x| x.as_str()).map(str::to_string);
                    let available = latest.as_deref().is_some_and(|l| newer(l, &current));
                    UpdateInfo {
                        current,
                        available,
                        notes: v.get("notes").and_then(|x| x.as_str()).map(str::to_string),
                        url: v
                            .get("url")
                            .and_then(|x| x.as_str())
                            .filter(|u| u.starts_with("https://"))
                            .map(str::to_string),
                        latest,
                        message: None,
                    }
                }
                Err(e) => UpdateInfo {
                    current,
                    latest: None,
                    available: false,
                    notes: None,
                    url: None,
                    message: Some(format!("Couldn't check for updates: {e}")),
                },
            })
        }
    })
    .await
}

/// What "Send feedback" puts in a report. Nothing is sent from here: the person copies it.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackInfo {
    /// The log's startup line: app version, OS, CPU architecture, GPU and decoder.
    pub environment: String,
    /// The page testers send reports to, when the build has one (`OVE_FEEDBACK_URL`).
    pub feedback_url: Option<String>,
}

/// The feedback page set for this build, like `OVE_UPDATE_URL`: at run time or when the app was built. https only
/// (`open_url` opens nothing else).
pub fn feedback_url() -> Option<String> {
    pick_feedback_url(std::env::var("OVE_FEEDBACK_URL").ok(), option_env!("OVE_FEEDBACK_URL"))
}

fn pick_feedback_url(runtime: Option<String>, built: Option<&str>) -> Option<String> {
    runtime.or_else(|| built.map(str::to_string)).filter(|u| u.starts_with("https://"))
}

/// The environment block for a feedback report, and where to send it.
#[tauri::command]
#[specta::specta]
pub async fn feedback_info(app: AppHandle) -> AppResult<FeedbackInfo> {
    worker::blocking("feedback_info", move || {
        let state = app.state::<crate::AppState>();
        let gpu = worker::relock(&state.gpu_summary).clone().unwrap_or_else(|| "unknown".into());
        Ok(FeedbackInfo {
            environment: crate::logging::startup_line(&gpu, crate::DECODE_CAPABILITIES),
            feedback_url: feedback_url(),
        })
    })
    .await
}

/// Opens a web page in the default browser (download page from the update check).
#[tauri::command]
#[specta::specta]
pub fn open_url(url: String) -> AppResult<()> {
    if !url.starts_with("https://") {
        return Err(AppError::new(ErrorCode::InvalidArgument, "Only https links open."));
    }
    let r = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(&url).spawn()
    } else if cfg!(windows) {
        std::process::Command::new("explorer").arg(&url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(&url).spawn()
    };
    r.map(|_| ()).map_err(|e| AppError::new(ErrorCode::Io, e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{newer, pick_feedback_url};

    #[test]
    fn feedback_page_is_https_and_run_time_wins() {
        assert_eq!(pick_feedback_url(None, None), None);
        assert_eq!(pick_feedback_url(None, Some("https://a.example/f")).as_deref(), Some("https://a.example/f"));
        assert_eq!(
            pick_feedback_url(Some("https://b.example/f".into()), Some("https://a.example/f")).as_deref(),
            Some("https://b.example/f")
        );
        // `open_url` opens https only, so nothing else is offered.
        assert_eq!(pick_feedback_url(Some("http://a.example/f".into()), None), None);
        assert_eq!(pick_feedback_url(Some("file:///etc/passwd".into()), None), None);
    }

    #[test]
    fn versions() {
        assert!(newer("0.2.0", "0.1.9"));
        assert!(newer("1.0.0", "0.9.9"));
        assert!(newer("v0.1.10", "0.1.9"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0-beta", "0.1.0"));
    }
}
