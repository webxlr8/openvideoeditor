//! Verify (spec: Verify): a registry of lints over the document. Later changes register more lints (off-screen text,
//! invalid compositions) through [`Registry::register`].

use std::collections::BTreeSet;

use serde::Serialize;
use serde_json::Value;

use ove_core::model::Document;
use ove_core::time::{Micros, json_to_micros};

use crate::format::micros;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub lint: &'static str,
    pub severity: Level,
    pub item_ids: Vec<String>,
    pub message: String,
}

/// What a lint can look at.
pub struct Context<'a> {
    pub doc: &'a Document,
    /// Media ids whose files are offline (from the store's media status).
    pub offline: &'a BTreeSet<String>,
}

pub trait Lint: Send + Sync {
    fn code(&self) -> &'static str;
    fn run(&self, ctx: &Context, out: &mut Vec<Finding>);
}

/// Gaps shorter than this are not reported (sub-millisecond rounding).
const GAP_TOLERANCE: Micros = 1_000;

struct Overlaps;
impl Lint for Overlaps {
    fn code(&self) -> &'static str {
        "overlap"
    }
    fn run(&self, ctx: &Context, out: &mut Vec<Finding>) {
        for t in &ctx.doc.project.tracks {
            let items = ctx.doc.items_on_track(&t.id);
            for (i, a) in items.iter().enumerate() {
                for b in &items[i + 1..] {
                    if b.start >= a.end {
                        break;
                    }
                    out.push(Finding {
                        lint: "overlap",
                        severity: Level::Error,
                        item_ids: vec![a.id.clone(), b.id.clone()],
                        message: format!(
                            "{} and {} overlap on track \"{}\" from {}s to {}s",
                            a.id,
                            b.id,
                            t.name,
                            micros(b.start),
                            micros(a.end.min(b.end))
                        ),
                    });
                }
            }
        }
    }
}

/// Gaps on the main track: the bottom-most video track (the base layer the rest composites over).
struct Gaps;
impl Lint for Gaps {
    fn code(&self) -> &'static str {
        "gap"
    }
    fn run(&self, ctx: &Context, out: &mut Vec<Finding>) {
        let Some(main) = ctx.doc.project.tracks.iter().rev().find(|t| t.kind == "video") else { return };
        let mut cursor: Micros = 0;
        let mut prev: Option<String> = None;
        for it in ctx.doc.items_on_track(&main.id) {
            if it.start > cursor + GAP_TOLERANCE {
                out.push(Finding {
                    lint: "gap",
                    severity: Level::Warning,
                    item_ids: prev.iter().cloned().chain([it.id.clone()]).collect(),
                    message: format!(
                        "gap on track \"{}\" from {}s to {}s (shows black)",
                        main.name,
                        micros(cursor),
                        micros(it.start)
                    ),
                });
            }
            cursor = cursor.max(it.end);
            prev = Some(it.id.clone());
        }
    }
}

struct PastSource;
impl Lint for PastSource {
    fn code(&self) -> &'static str {
        "past-source"
    }
    fn run(&self, ctx: &Context, out: &mut Vec<Finding>) {
        for it in ctx.doc.project.items.values() {
            if !matches!(it.kind.as_str(), "video" | "audio") {
                continue;
            }
            let Some(m) = it.media_id().and_then(|id| ctx.doc.project.media.get(id)) else { continue };
            let Some(dur) = m.data().and_then(|d| d.get("duration")).and_then(json_to_micros) else { continue };
            let speed = it.fields.get("playbackSpeed").and_then(Value::as_f64).unwrap_or(1.0);
            let used = it.time_field("trimStart") + ((it.end - it.start) as f64 * speed).round() as Micros;
            if used > dur + GAP_TOLERANCE {
                let what = if it.kind == "video" { "holds its last frame" } else { "is silent" };
                out.push(Finding {
                    lint: "past-source",
                    severity: Level::Warning,
                    item_ids: vec![it.id.clone()],
                    message: format!(
                        "{} runs {}s past the end of its source and {what} for that time",
                        it.id,
                        micros(used - dur)
                    ),
                });
            }
        }
    }
}

struct MissingMedia;
impl Lint for MissingMedia {
    fn code(&self) -> &'static str {
        "missing-media"
    }
    fn run(&self, ctx: &Context, out: &mut Vec<Finding>) {
        for it in ctx.doc.project.items.values() {
            let Some(mid) = it.media_id() else { continue };
            let reason = if !ctx.doc.project.media.contains_key(mid) {
                Some("is not in the project".to_string())
            } else if ctx.offline.contains(mid) {
                Some("is offline (file moved or drive not connected)".to_string())
            } else if ctx.doc.registry.get(mid).is_some_and(|e| e.probe_status() == "failed") {
                Some("could not be read".to_string())
            } else {
                None
            };
            if let Some(r) = reason {
                out.push(Finding {
                    lint: "missing-media",
                    severity: Level::Error,
                    item_ids: vec![it.id.clone()],
                    message: format!("{}: media {mid} {r}", it.id),
                });
            }
        }
    }
}

struct Pending;
impl Lint for Pending {
    fn code(&self) -> &'static str {
        "pending"
    }
    fn run(&self, ctx: &Context, out: &mut Vec<Finding>) {
        for it in ctx.doc.project.items.values().filter(|i| i.is_pending()) {
            let what = it.fields.get("pendingDescription").and_then(Value::as_str).unwrap_or("no description");
            out.push(Finding {
                lint: "pending",
                severity: Level::Error,
                item_ids: vec![it.id.clone()],
                message: format!("{} is a pending placeholder ({what}); fill it before export", it.id),
            });
        }
    }
}

pub struct Registry {
    lints: Vec<Box<dyn Lint>>,
}

impl Default for Registry {
    /// The structural lints of this change.
    fn default() -> Self {
        Registry {
            lints: vec![
                Box::new(Overlaps),
                Box::new(Gaps),
                Box::new(PastSource),
                Box::new(MissingMedia),
                Box::new(Pending),
            ],
        }
    }
}

impl Registry {
    pub fn register(&mut self, lint: Box<dyn Lint>) {
        self.lints.push(lint);
    }

    pub fn codes(&self) -> Vec<&'static str> {
        self.lints.iter().map(|l| l.code()).collect()
    }

    /// Runs every lint. Findings are sorted by severity, lint, then item ids.
    pub fn run(&self, ctx: &Context) -> Vec<Finding> {
        let mut out = Vec::new();
        for l in &self.lints {
            l.run(ctx, &mut out);
        }
        out.sort_by(|a, b| (a.severity, a.lint, &a.item_ids).cmp(&(b.severity, b.lint, &b.item_ids)));
        out
    }
}

/// True when verify should exit 1.
pub fn has_errors(findings: &[Finding]) -> bool {
    findings.iter().any(|f| f.severity == Level::Error)
}
