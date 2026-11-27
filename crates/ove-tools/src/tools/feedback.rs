//! `send_feedback` (change unify-agent-tool-layer, D8): an agent drafts a report about the app — a bug, something
//! confusing or missing, an idea — and it is stored on this computer only, in `<config>/feedback/<id>.json`. Nothing
//! is ever sent: the person reviews each draft in Help → Feedback, edits or deletes it, and copies the report to send
//! it themselves. A session may file 8 reports; an exact duplicate (same category, severity and summary) is dropped.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::ctx::ToolCtx;
use crate::fail::{Fail, now_ms};
use crate::receipt::Receipt;
use crate::session::{FEEDBACK_PER_SESSION, canonical, config_dir, fnv1a};
use crate::spec::Args;

/// The queue's folder in the configuration folder.
pub const DIR: &str = "feedback";
/// Longest summary accepted (characters).
pub const MAX_SUMMARY: usize = 2000;
pub const CATEGORIES: [&str; 4] = ["bug", "confusing", "missing", "idea"];
pub const SEVERITIES: [&str; 3] = ["low", "medium", "high"];

/// One queued report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub id: String,
    /// Milliseconds since the epoch.
    pub created_at: i64,
    pub category: String,
    pub severity: String,
    pub summary: String,
    pub app_version: String,
    pub os: String,
    /// Where the agent ran: `inApp`, `mcp`, `pinned`.
    pub surface: String,
    #[serde(default)]
    pub client: Option<String>,
    #[serde(default)]
    pub session: Option<String>,
    /// The session's last tool calls, oldest first.
    #[serde(default)]
    pub recent_tools: Vec<String>,
    /// The code of the session's last refused call.
    #[serde(default)]
    pub last_error: Option<String>,
    /// When the person last changed the summary.
    #[serde(default)]
    pub edited_at: Option<i64>,
}

impl Draft {
    /// What makes two reports the same: category, severity and the summary's words.
    pub fn fingerprint(&self) -> String {
        fingerprint(&self.category, &self.severity, &self.summary)
    }
}

fn fingerprint(category: &str, severity: &str, summary: &str) -> String {
    let words: Vec<&str> = summary.split_whitespace().collect();
    fnv1a(&canonical(&json!([category, severity, words.join(" ")])))
}

pub fn queue_dir(config: &Path) -> PathBuf {
    config.join(DIR)
}

fn no_config() -> Fail {
    Fail::new("feedback.unavailable", "there is no configuration folder on this computer to keep feedback in")
}

fn io(e: std::io::Error) -> Fail {
    Fail::new("file.unwritable", e.to_string())
}

/// The queued reports, newest first. Unreadable files are skipped.
pub fn list(config: &Path) -> Vec<Draft> {
    let mut v: Vec<Draft> = std::fs::read_dir(queue_dir(config))
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let p = e.path();
                    let stem = p.file_stem()?.to_str()?.to_string();
                    if p.extension()? != "json" || !ove_guard::is_safe_id(&stem) {
                        return None;
                    }
                    let d: Draft = serde_json::from_slice(&std::fs::read(&p).ok()?).ok()?;
                    (d.id == stem).then_some(d)
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| b.created_at.cmp(&a.created_at).then_with(|| b.id.cmp(&a.id)));
    v
}

fn path_of(config: &Path, id: &str) -> Result<PathBuf, Fail> {
    if !ove_guard::is_safe_id(id) {
        return Err(Fail::new("argument.invalid", format!("{id:?} is not a feedback id")));
    }
    Ok(queue_dir(config).join(format!("{id}.json")))
}

/// Writes a report (atomically).
pub fn save(config: &Path, d: &Draft) -> Result<(), Fail> {
    let path = path_of(config, &d.id)?;
    std::fs::create_dir_all(queue_dir(config)).map_err(io)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(d).unwrap_or_default()).map_err(io)?;
    std::fs::rename(&tmp, &path).map_err(io)
}

/// One report by id.
pub fn load(config: &Path, id: &str) -> Result<Draft, Fail> {
    let path = path_of(config, id)?;
    let d: Draft = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| Fail::new("feedback.notFound", format!("no feedback report {id}")))?;
    if d.id != id {
        return Err(Fail::new("feedback.notFound", format!("no feedback report {id}")));
    }
    Ok(d)
}

/// The person's edit of a report's summary.
pub fn update_summary(config: &Path, id: &str, summary: &str) -> Result<Draft, Fail> {
    let summary = check_summary(summary)?;
    let mut d = load(config, id)?;
    d.summary = summary;
    d.edited_at = Some(now_ms());
    save(config, &d)?;
    Ok(d)
}

/// Removes a report.
pub fn delete(config: &Path, id: &str) -> Result<(), Fail> {
    let path = path_of(config, id)?;
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(Fail::new("feedback.notFound", format!("no feedback report {id}")))
        }
        Err(e) => Err(io(e)),
    }
}

/// The report as the person copies it.
pub fn report_text(d: &Draft) -> String {
    let when = d.created_at;
    let mut s = format!("[{} · {}] {}\n\n", d.category, d.severity, d.summary.trim());
    s.push_str("--- Environment ---\n");
    s.push_str(&format!("app {} · {}\n", d.app_version, d.os));
    let from = d.client.as_deref().map(|c| format!("{} ({c})", d.surface)).unwrap_or_else(|| d.surface.clone());
    s.push_str(&format!("drafted by an agent over {from} at {when} ms since the epoch\n"));
    if !d.recent_tools.is_empty() {
        s.push_str(&format!("recent tools: {}\n", d.recent_tools.join(", ")));
    }
    if let Some(e) = &d.last_error {
        s.push_str(&format!("last error: {e}\n"));
    }
    s
}

fn check_summary(summary: &str) -> Result<String, Fail> {
    let t = summary.trim();
    if t.is_empty() {
        return Err(Fail::new("argument.invalid", "the summary is empty"));
    }
    if t.chars().count() > MAX_SUMMARY {
        return Err(Fail::new("argument.invalid", format!("the summary is longer than {MAX_SUMMARY} characters")));
    }
    Ok(t.to_string())
}

fn one_of(a: &Args, k: &str, allowed: &[&str]) -> Result<String, Fail> {
    let v = a.req_str(k)?;
    if allowed.contains(&v) {
        Ok(v.to_string())
    } else {
        Err(Fail::new("argument.invalid", format!("`{k}` is one of {}", allowed.join(", "))))
    }
}

fn new_id(now: i64) -> String {
    // The job id's random part keeps two reports in the same millisecond apart.
    let random = ove_generate::job::new_id(now);
    format!("fb_{now:x}_{}", random.rsplit('_').next().unwrap_or_default())
}

/// `send_feedback`.
pub fn send_feedback(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let category = one_of(a, "category", &CATEGORIES)?;
    let severity = one_of(a, "severity", &SEVERITIES)?;
    let summary = check_summary(a.req_str("summary")?)?;
    let session =
        ctx.session().ok_or_else(|| Fail::new("tool.unavailable", "feedback is not available in this context"))?;
    let config = session.config().map(Path::to_path_buf).or_else(config_dir).ok_or_else(no_config)?;
    let fp = fingerprint(&category, &severity, &summary);
    let queued = list(&config);
    if let Some(same) = queued.iter().find(|d| d.fingerprint() == fp) {
        return Ok(Receipt::new(
            json!({"stored": false, "duplicate": true, "id": same.id, "sent": false}),
            format!("the same report is already queued ({}); nothing new stored", same.id),
        ));
    }
    if session.filed(&fp) {
        return Ok(Receipt::new(
            json!({"stored": false, "duplicate": true, "sent": false}),
            "this session already filed the same report; nothing new stored",
        ));
    }
    if session.feedback_filed() >= FEEDBACK_PER_SESSION {
        return Err(Fail::new(
            "feedback.limit",
            format!("a session can file {FEEDBACK_PER_SESSION} reports; nothing was stored"),
        ));
    }
    let actor = ctx.actor();
    let now = now_ms();
    let mut recent = session.recent_tools();
    recent.retain(|t| t != "send_feedback");
    let d = Draft {
        id: new_id(now),
        created_at: now,
        category,
        severity,
        summary,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        surface: ctx.surface().as_str().to_string(),
        client: actor.as_ref().and_then(|x| x.client.clone()),
        session: actor.as_ref().and_then(|x| x.session.clone()),
        recent_tools: recent,
        last_error: session.last_error(),
        edited_at: None,
    };
    save(&config, &d)?;
    session.note_feedback(fp);
    let left = FEEDBACK_PER_SESSION - session.feedback_filed();
    Ok(Receipt::new(
        json!({"stored": true, "duplicate": false, "id": d.id, "sent": false, "left": left}),
        format!(
            "stored on this computer as {}; the person reviews it in Help → Feedback and decides whether to send it ({left} more allowed this session)",
            d.id
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(id: &str, at: i64, summary: &str) -> Draft {
        Draft {
            id: id.into(),
            created_at: at,
            category: "bug".into(),
            severity: "low".into(),
            summary: summary.into(),
            app_version: "0".into(),
            os: "test".into(),
            surface: "mcp".into(),
            client: None,
            session: None,
            recent_tools: vec![],
            last_error: None,
            edited_at: None,
        }
    }

    #[test]
    fn the_queue_lists_newest_first_edits_and_deletes() {
        let d = tempfile::tempdir().unwrap();
        save(d.path(), &draft("fb_1", 1, "one")).unwrap();
        save(d.path(), &draft("fb_2", 2, "two")).unwrap();
        std::fs::write(queue_dir(d.path()).join("junk.json"), b"{").unwrap();
        std::fs::write(queue_dir(d.path()).join("fb_3.json"), serde_json::to_vec(&draft("other", 3, "x")).unwrap())
            .unwrap();
        let ids: Vec<String> = list(d.path()).into_iter().map(|x| x.id).collect();
        assert_eq!(ids, ["fb_2", "fb_1"]);
        let e = update_summary(d.path(), "fb_1", "  one, reworded  ").unwrap();
        assert_eq!(e.summary, "one, reworded");
        assert!(e.edited_at.is_some());
        assert_eq!(load(d.path(), "fb_1").unwrap().summary, "one, reworded");
        assert_eq!(update_summary(d.path(), "fb_1", "  ").unwrap_err().code, "argument.invalid");
        delete(d.path(), "fb_2").unwrap();
        assert_eq!(delete(d.path(), "fb_2").unwrap_err().code, "feedback.notFound");
        assert_eq!(delete(d.path(), "../x").unwrap_err().code, "argument.invalid");
        assert_eq!(list(d.path()).len(), 1);
    }

    #[test]
    fn duplicates_ignore_spacing_only() {
        let a = draft("a", 0, "Export  fails\non long clips");
        let b = draft("b", 0, "Export fails on long clips");
        let c = draft("c", 0, "Export fails on short clips");
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert_ne!(a.fingerprint(), c.fingerprint());
        let text = report_text(&a);
        assert!(text.starts_with("[bug · low] Export  fails\non long clips"), "{text}");
    }
}
