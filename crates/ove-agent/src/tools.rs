//! The assistant's tools: definitions (JSON Schema), input validation, a host trait the app implements, and
//! implementations over an open [`Project`] shared by the app and the tests.

use std::collections::BTreeSet;

use ove_core::revision::Source;
pub use ove_projection::textedit::TextEdit;
use ove_projection::textedit::apply_edits;
use ove_projection::verify::{Context, Level, Registry};
use ove_projection::{ApplyInput, BaseError, Prefer};
use ove_store::media::{MediaState, media_status};
use ove_store::project::Project;
use serde::Deserialize;
use serde_json::{Value, json};

/// Where tools run. All results are text (usually JSON) handed back to the model; `Err` becomes an error result.
pub trait ToolHost: Send {
    fn print_timeline(&mut self, from: Option<usize>, lines: Option<usize>) -> Result<String, String>;
    fn apply_timeline(&mut self, req: ApplyRequest) -> Result<String, String>;
    fn verify_timeline(&mut self) -> Result<String, String>;
    fn list_media(&mut self) -> Result<String, String>;
    fn undo_last(&mut self) -> Result<String, String>;
    fn import_media(&mut self, _paths: Vec<String>) -> Result<String, String> {
        Err("Importing files is not available here; ask the user to import them.".into())
    }
    fn seek_viewer(&mut self, _seconds: f64) -> Result<String, String> {
        Err("The viewer is not available here.".into())
    }
    /// Shows a question card and blocks until the user answers (or the turn is stopped).
    fn ask_user(&mut self, question_id: &str, question: &str, options: &[String]) -> Result<String, String>;
    /// Checks a code item's composition (`codeComposition`). Hosts with a project resolve its aspect and assets.
    fn check_code(&mut self, composition: &Value, duration: f64) -> Result<String, String> {
        Ok(check_code(None, composition, duration))
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyRequest {
    /// The `rev` the edits were made against (from the header of the printed timeline).
    #[serde(default)]
    pub rev: Option<i64>,
    #[serde(default)]
    pub edits: Option<Vec<TextEdit>>,
    /// A whole replacement projection instead of edits.
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub prefer: Option<String>,
}

/// Tool definitions, in a fixed order (the request prefix must stay byte-stable for prompt caching).
pub fn definitions() -> Vec<Value> {
    let tools = vec![
        json!({
            "name": "print_timeline",
            "description": "Print the timeline of the open project as its literal TSX projection (header with media table, pending count and rev, then <track> and item elements). Always print before editing; use from/lines to read part of a long timeline.",
            "input_schema": {"type": "object", "properties": {
                "from": {"type": "integer", "minimum": 1, "description": "First line (1-based)."},
                "lines": {"type": "integer", "minimum": 1, "description": "Number of lines."}
            }, "additionalProperties": false}
        }),
        json!({
            "name": "apply_timeline",
            "description": "Apply an edit to the timeline as one undoable revision. Give `edits` (each oldText must occur exactly once in the latest printed timeline; newText replaces it; empty newText deletes) or a whole replacement `source`. Pass the `rev` from the header you edited. Returns what changed, conflicts and warnings, or the diagnostics that refused the edit.",
            "input_schema": {"type": "object", "properties": {
                "rev": {"type": "integer", "description": "The rev from the printed header."},
                "edits": {"type": "array", "items": {"type": "object", "properties": {
                    "oldText": {"type": "string"}, "newText": {"type": "string"}
                }, "required": ["oldText", "newText"], "additionalProperties": false}},
                "source": {"type": "string", "description": "Complete replacement projection text."},
                "prefer": {"type": "string", "enum": ["file", "editor"], "description": "Which side wins a conflict with edits the user made meanwhile (default file)."}
            }, "additionalProperties": false}
        }),
        json!({
            "name": "verify_timeline",
            "description": "Check the timeline for problems: overlapping items, gaps on the main track, clips running past their source, missing or offline media, pending placeholders.",
            "input_schema": {"type": "object", "properties": {}, "additionalProperties": false}
        }),
        json!({
            "name": "list_media",
            "description": "List the project's media: name (usable as src), id, type, duration, resolution, and whether the file is online.",
            "input_schema": {"type": "object", "properties": {}, "additionalProperties": false}
        }),
        json!({
            "name": "undo_last",
            "description": "Undo the most recent change you applied (only if it is still the latest change in the project).",
            "input_schema": {"type": "object", "properties": {}, "additionalProperties": false}
        }),
        json!({
            "name": "import_media",
            "description": "Import media files into the project by absolute path. Each becomes a media item and a clip appended to the main track.",
            "input_schema": {"type": "object", "properties": {
                "paths": {"type": "array", "items": {"type": "string"}, "minItems": 1}
            }, "required": ["paths"], "additionalProperties": false}
        }),
        json!({
            "name": "seek_viewer",
            "description": "Move the editor's playhead to a timeline time (seconds) so the user sees that moment.",
            "input_schema": {"type": "object", "properties": {
                "time": {"type": "number", "minimum": 0}
            }, "required": ["time"], "additionalProperties": false}
        }),
        json!({
            "name": "check_code",
            "description": "Check a code item's composition before (or after) applying it: transpiles the TSX, reports syntax errors, imports and unavailable globals with line:column, then renders frames at the start, middle and end and reports runtime errors, endless loops, unsupported elements and styles, console output and the slowest frame time. Pass the codeComposition object ({source, props?, assets?} or {template, props}).",
            "input_schema": {"type": "object", "properties": {
                "composition": {"type": "object", "description": "The codeComposition value."},
                "duration": {"type": "number", "exclusiveMinimum": 0, "description": "Item duration in seconds (default 4)."}
            }, "required": ["composition"], "additionalProperties": false}
        }),
        json!({
            "name": "ask_user",
            "description": "Ask the user a question when a choice is genuinely theirs (style, which take, what to cut). Offer 2-4 short options when you can. Blocks until they answer.",
            "input_schema": {"type": "object", "properties": {
                "question": {"type": "string"},
                "options": {"type": "array", "items": {"type": "string"}, "maxItems": 4}
            }, "required": ["question"], "additionalProperties": false}
        }),
    ];
    // Streamed tool inputs arrive as they are generated; inputs are validated client-side before running.
    tools
        .into_iter()
        .map(|mut t| {
            t["eager_input_streaming"] = json!(true);
            t
        })
        .collect()
}

/// A short human label for a tool call, shown as a chip in the chat.
pub fn label(name: &str, input: &Value) -> String {
    match name {
        "print_timeline" => "Reading the timeline".into(),
        "apply_timeline" => {
            let n = input["edits"].as_array().map(|a| a.len()).unwrap_or(0);
            if n > 0 {
                format!("Editing the timeline ({n} change{})", if n == 1 { "" } else { "s" })
            } else {
                "Rewriting the timeline".into()
            }
        }
        "verify_timeline" => "Checking the timeline".into(),
        "list_media" => "Looking at the media".into(),
        "undo_last" => "Undoing the last change".into(),
        "import_media" => "Importing media".into(),
        "seek_viewer" => format!("Jumping to {:.1}s", input["time"].as_f64().unwrap_or(0.0)),
        "ask_user" => "Asking you".into(),
        "check_code" => "Checking motion graphics code".into(),
        other => other.to_string(),
    }
}

fn opt_usize(input: &Value, key: &str) -> Result<Option<usize>, String> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .filter(|n| *n >= 1)
            .map(|n| Some(n as usize))
            .ok_or_else(|| format!("`{key}` must be a positive integer")),
    }
}

fn only_keys(input: &Value, allowed: &[&str]) -> Result<(), String> {
    let o = input.as_object().ok_or("the input must be an object")?;
    match o.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(format!("unknown field `{k}`")),
        None => Ok(()),
    }
}

/// Validates a tool input against its schema and runs it on the host. `Err` is an error result for the model.
pub fn dispatch(host: &mut dyn ToolHost, tool_use_id: &str, name: &str, input: &Value) -> Result<String, String> {
    match name {
        "print_timeline" => {
            only_keys(input, &["from", "lines"])?;
            host.print_timeline(opt_usize(input, "from")?, opt_usize(input, "lines")?)
        }
        "apply_timeline" => {
            let req: ApplyRequest = serde_json::from_value(input.clone()).map_err(|e| format!("invalid input: {e}"))?;
            if req.edits.is_none() == req.source.is_none() {
                return Err("give exactly one of `edits` or `source`".into());
            }
            if let Some(p) = &req.prefer
                && p != "file"
                && p != "editor"
            {
                return Err("`prefer` must be \"file\" or \"editor\"".into());
            }
            host.apply_timeline(req)
        }
        "verify_timeline" => {
            only_keys(input, &[])?;
            host.verify_timeline()
        }
        "list_media" => {
            only_keys(input, &[])?;
            host.list_media()
        }
        "undo_last" => {
            only_keys(input, &[])?;
            host.undo_last()
        }
        "import_media" => {
            only_keys(input, &["paths"])?;
            let paths: Vec<String> = input["paths"]
                .as_array()
                .ok_or("`paths` must be an array of strings")?
                .iter()
                .map(|p| p.as_str().map(str::to_string).ok_or("`paths` must be an array of strings"))
                .collect::<Result<_, _>>()?;
            if paths.is_empty() {
                return Err("`paths` must not be empty".into());
            }
            host.import_media(paths)
        }
        "seek_viewer" => {
            only_keys(input, &["time"])?;
            let t = input["time"]
                .as_f64()
                .filter(|t| *t >= 0.0 && t.is_finite())
                .ok_or("`time` must be a non-negative number")?;
            host.seek_viewer(t)
        }
        "ask_user" => {
            only_keys(input, &["question", "options"])?;
            let q = input["question"].as_str().filter(|s| !s.trim().is_empty()).ok_or("`question` is required")?;
            let options: Vec<String> = match input.get("options") {
                None | Some(Value::Null) => vec![],
                Some(v) => v
                    .as_array()
                    .ok_or("`options` must be an array of strings")?
                    .iter()
                    .map(|o| o.as_str().map(str::to_string).ok_or("`options` must be an array of strings"))
                    .collect::<Result<_, _>>()?,
            };
            host.ask_user(tool_use_id, q, &options)
        }
        "check_code" => {
            only_keys(input, &["composition", "duration"])?;
            let c = input.get("composition").filter(|c| c.is_object()).ok_or("`composition` must be an object")?;
            let d = match input.get("duration") {
                None | Some(Value::Null) => 4.0,
                Some(v) => {
                    v.as_f64().filter(|d| *d > 0.0 && d.is_finite()).ok_or("`duration` must be a positive number")?
                }
            };
            host.check_code(c, d)
        }
        other => Err(format!("unknown tool `{other}`")),
    }
}

// ------------------------------------------------------------------------------------------------------------------
// Implementations over an open project

/// `print_timeline`.
pub fn print(p: &Project, from: Option<usize>, lines: Option<usize>) -> String {
    let text = p.projection();
    if from.is_none() && lines.is_none() {
        return text;
    }
    let total = text.lines().count();
    let part = ove_projection::print::slice_lines(&text, from.unwrap_or(1), lines);
    format!(
        "{part}\n[lines {}–{} of {total}]",
        from.unwrap_or(1),
        (from.unwrap_or(1) + part.lines().count()).saturating_sub(1)
    )
}

/// `apply_timeline`: plans the edit with the 3-way merge and applies it as one agent revision.
pub fn apply(p: &mut Project, req: ApplyRequest) -> Result<String, String> {
    let current_rev = p.rev();
    let text = match (&req.edits, &req.source) {
        (Some(edits), _) => {
            if let Some(r) = req.rev
                && r != current_rev
            {
                return Err(format!(
                    "The timeline changed since rev {r} (it is now rev {current_rev}). Print it again and redo your edits against the new text."
                ));
            }
            apply_edits(&p.projection(), edits)
                .map_err(|e| format!("{} ({}). Print the timeline again if it changed.", e.message, e.code))?
        }
        (None, Some(source)) => source.clone(),
        (None, None) => return Err("give `edits` or `source`".into()),
    };
    let prefer = if req.prefer.as_deref() == Some("editor") { Prefer::Editor } else { Prefer::File };
    let plan = {
        let pr: &Project = p;
        let base_for =
            |rev: i64| pr.doc_at(rev).map_err(|e| BaseError { code: e.code().to_string(), message: e.to_string() });
        ove_projection::plan(&ApplyInput {
            text: &text,
            current: pr.doc(),
            base_for: &base_for,
            bundle: Some(pr.bundle()),
            prefer,
            now_ms: ove_core::Clock::now_ms(&ove_core::SystemClock),
        })
    };
    let plan = plan.map_err(|r| {
        let mut s = format!("Refused ({}): {}", r.code, r.message);
        for d in r.diagnostics.iter().take(20) {
            s.push_str(&format!("\n  {d}"));
        }
        s
    })?;
    let changed = !plan.ops.is_empty();
    if changed {
        p.apply(plan.ops.clone(), Source::Agent, Some(format!("assistant: {}", plan.summary())))
            .map_err(|e| e.to_string())?;
    }
    let _ = p.set_diagnostics(plan.warnings.iter().map(|w| w.to_string()).collect());
    Ok(json!({
        "changed": changed,
        "rev": p.rev(),
        "summary": plan.summary(),
        "added": plan.counts.added,
        "updated": plan.counts.updated,
        "removed": plan.removed,
        "conflicts": plan.conflicts,
        "warnings": plan.warnings.iter().map(|w| w.to_string()).collect::<Vec<_>>(),
    })
    .to_string())
}

/// `verify_timeline`.
pub fn verify(p: &Project) -> String {
    let offline: BTreeSet<String> = media_status(p.bundle(), p.doc())
        .into_iter()
        .filter(|m| m.state == MediaState::Offline)
        .map(|m| m.media_id)
        .collect();
    let findings = Registry::default().run(&Context { doc: p.doc(), offline: &offline });
    if findings.is_empty() {
        return "No problems found.".into();
    }
    findings
        .iter()
        .map(|f| format!("{} [{}] {}", if f.severity == Level::Error { "error" } else { "warning" }, f.lint, f.message))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `list_media`.
pub fn list_media(p: &Project) -> String {
    let names = ove_projection::print::media_names(p.doc());
    let status = media_status(p.bundle(), p.doc());
    let rows: Vec<Value> = p
        .doc()
        .project
        .media
        .values()
        .map(|m| {
            let d = m.data();
            let st = status.iter().find(|s| s.media_id == m.id);
            json!({
                "name": names.get(&m.id),
                "id": m.id,
                "type": d.and_then(|d| d.get("type")).cloned(),
                "durationSeconds": d.and_then(|d| d.get("duration")).cloned(),
                "width": d.and_then(|d| d.get("width")).cloned(),
                "height": d.and_then(|d| d.get("height")).cloned(),
                "online": st.map(|s| s.state != MediaState::Offline).unwrap_or(false),
            })
        })
        .collect();
    json!(rows).to_string()
}

/// `check_code`: the motion-graphics check as compact JSON. With a project, the frame size follows its aspect ratio
/// and image assets (media ids or names) load from its media.
pub fn check_code(p: Option<&Project>, composition: &Value, duration: f64) -> String {
    let Some(c) = ove_graphics::CodeComposition::from_value(composition) else {
        return json!({"ok": false, "diagnostics": ["error [shape] the composition must be an object"]}).to_string();
    };
    let size = match p.map(|p| p.doc().project.aspect_ratio.as_str()) {
        Some("9:16") => (1080, 1920),
        Some("1:1") => (1080, 1080),
        Some("4:5") => (1080, 1350),
        Some("4:3") => (1440, 1080),
        Some("3:4") => (1080, 1440),
        _ => (1920, 1080),
    };
    let load = |id: &str| {
        let p = p?;
        let doc = p.doc();
        let id = if doc.registry.contains_key(id) {
            id.to_string()
        } else {
            ove_projection::print::media_names(doc).into_iter().find(|(_, n)| n == id).map(|(k, _)| k)?
        };
        let native = std::path::PathBuf::from(&doc.registry.get(&id)?.native_path);
        let path = if native.is_absolute() { native } else { p.bundle().join(native) };
        std::fs::read(path).ok().map(std::sync::Arc::new)
    };
    let r = ove_graphics::check(&c, size, duration, &load);
    let fmt = |d: &ove_graphics::Diagnostic| {
        let at = match (d.line, d.column) {
            (Some(l), Some(c)) => format!(" {l}:{c}"),
            _ => String::new(),
        };
        let sev = if d.is_error() { "error" } else { "warning" };
        format!("{sev} [{}]{at} {}", d.code, d.message)
    };
    json!({
        "ok": r.ok,
        "diagnostics": r.diagnostics.iter().map(fmt).collect::<Vec<_>>(),
        "logs": r.logs,
        "slowestFrameMs": (r.frame_ms * 10.0).round() / 10.0,
        "size": format!("{}x{}", r.width, r.height),
    })
    .to_string()
}

/// `undo_last`: reverts the latest revision if it was the assistant's.
pub fn undo_last(p: &mut Project) -> Result<String, String> {
    let latest = p.latest_record().map_err(|e| e.to_string())?.filter(|r| r.rev == p.rev());
    match latest {
        Some(r) if r.source == Source::Agent => {
            let undone = if p.session().can_undo() && p.session().undo_summary() == Some(r.summary.as_str()) {
                p.undo().map_err(|e| e.to_string())?
            } else {
                p.restore_as(r.parent, Source::Undo).map_err(|e| e.to_string())?
            };
            Ok(match undone {
                Some(c) => format!("Undid \"{}\" (now rev {}).", r.summary, c.record.rev),
                None => "Nothing changed.".into(),
            })
        }
        Some(r) => Err(format!("The latest change (\"{}\") was not made by you; not undoing it.", r.summary)),
        None => Err("There is nothing to undo.".into()),
    }
}
