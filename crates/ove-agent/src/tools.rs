//! The assistant's tools: a thin adapter over the agent tool registry (`ove-tools`). The definitions are the
//! registry's in-app tools; inputs are validated against the registry's schemas; calls run on a [`ToolHost`] the
//! app implements. The project-level helpers here run the registry's handlers on a project the caller holds.

use ove_store::project::Project;
use ove_tools::spec::validate;
use ove_tools::tools::{code, media, timeline};
use ove_tools::{AppCtx, ProjectHost, Surface, ToolSpec};
use serde::Deserialize;
use serde_json::{Map, Value};

pub use ove_projection::textedit::TextEdit;

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
    /// Runs any registry tool with the full arguments (hosts over the app's open project run them through
    /// `ove_tools::AppCtx`). `None`: this host has no generic runner, so the tool's dedicated method above is used.
    fn run_tool(&mut self, _tool: &'static ToolSpec, _args: &Map<String, Value>) -> Option<Result<String, String>> {
        None
    }
    /// A turn starts: the chat (the assistant's session) and turn its edits are recorded under.
    fn begin_turn(&mut self, _chat_id: &str, _turn_id: &str) {}
    /// Project revisions committed by tool calls since the last call (the turn records them so it can be reviewed
    /// and undone as a whole).
    fn take_revisions(&mut self) -> Vec<i64> {
        Vec::new()
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

impl ApplyRequest {
    /// The registry arguments for this request.
    pub fn args(&self) -> Map<String, Value> {
        let mut m = Map::new();
        if let Some(r) = self.rev {
            m.insert("rev".into(), r.into());
        }
        if let Some(e) = &self.edits {
            m.insert("edits".into(), serde_json::to_value(e).unwrap_or_default());
        }
        if let Some(s) = &self.source {
            m.insert("source".into(), s.clone().into());
        }
        if let Some(p) = &self.prefer {
            m.insert("prefer".into(), p.clone().into());
        }
        m
    }
}

/// Tool definitions, in the registry's fixed order (the request prefix must stay byte-stable for prompt caching).
pub fn definitions() -> Vec<Value> {
    ove_tools::anthropic_list()
}

/// A short human label for a tool call, shown as a chip in the chat.
pub fn label(name: &str, input: &Value) -> String {
    let name = ove_tools::find(name).map(|t| t.name).unwrap_or(name);
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
        "undo_timeline" => "Undoing the last change".into(),
        "add_media" => "Importing media".into(),
        "seek" => format!("Jumping to {:.1}s", input["seconds"].as_f64().unwrap_or(0.0)),
        "ask_user" => "Asking you".into(),
        "check_code" => "Checking motion graphics code".into(),
        other => other.to_string(),
    }
}

/// Validates a tool input against the registry and runs it on the host. `Err` is an error result for the model.
pub fn dispatch(host: &mut dyn ToolHost, tool_use_id: &str, name: &str, input: &Value) -> Result<String, String> {
    let spec = ove_tools::find(name)
        .filter(|t| t.on(Surface::InApp))
        .ok_or_else(|| ove_tools::unknown(name, Surface::InApp).model_text())?;
    let args = input.as_object().ok_or("the input must be an object")?;
    validate(spec, args, Surface::InApp).map_err(|f| f.model_text())?;
    let a = ove_tools::Args(args);
    if spec.name == "ask_user" {
        let q = a.str("question").filter(|s| !s.trim().is_empty()).ok_or("`question` is required")?;
        let options: Vec<String> = a
            .value("options")
            .and_then(Value::as_array)
            .map(|o| o.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        return host.ask_user(tool_use_id, q, &options);
    }
    if let Some(r) = host.run_tool(spec, args) {
        return r;
    }
    let positive = |k: &str| -> Result<Option<usize>, String> {
        match a.i64(k) {
            None => Ok(None),
            Some(n) if n >= 1 => Ok(Some(n as usize)),
            Some(_) => Err(format!("`{k}` must be a positive integer")),
        }
    };
    match spec.name {
        "print_timeline" => host.print_timeline(positive("from")?, positive("lines")?),
        "apply_timeline" => {
            let req = ApplyRequest {
                rev: a.i64("rev"),
                edits: match a.value("edits") {
                    None => None,
                    Some(v) => Some(serde_json::from_value(v.clone()).map_err(|e| format!("invalid input: {e}"))?),
                },
                source: a.str("source").map(str::to_string),
                prefer: a.str("prefer").map(str::to_string),
            };
            if req.edits.is_none() == req.source.is_none() {
                return Err("give exactly one of `edits` or `source`".into());
            }
            host.apply_timeline(req)
        }
        "verify_timeline" => host.verify_timeline(),
        "list_media" => host.list_media(),
        "undo_timeline" => host.undo_last(),
        "add_media" => host.import_media(a.strings("paths")),
        "seek" => host.seek_viewer(a.f64("seconds").filter(|t| *t >= 0.0).ok_or("`seconds` must be ≥ 0")?),
        "check_code" => {
            let c = a.value("composition").filter(|c| c.is_object()).ok_or("`composition` must be an object")?;
            let d = match a.value("duration") {
                None => 4.0,
                Some(v) => {
                    v.as_f64().filter(|d| *d > 0.0 && d.is_finite()).ok_or("`duration` must be a positive number")?
                }
            };
            host.check_code(c, d)
        }
        other => Err(format!("`{other}` is not available here")),
    }
}

/// Runs a registry tool on a project the caller holds, as the assistant: the model's text, or the refusal's.
pub fn run_on(p: &mut Project, name: &str, args: &Map<String, Value>) -> Result<String, String> {
    let spec = ove_tools::find(name).ok_or_else(|| format!("unknown tool `{name}`"))?;
    let mut host = ProjectHost { project: p };
    let mut ctx = AppCtx::assistant(&mut host);
    ove_tools::call(&mut ctx, name, args).map(|r| spec.model_text(&r)).map_err(|f| f.model_text())
}

// ------------------------------------------------------------------------------------------------------------------
// The registry's handlers over an open project

/// `print_timeline`.
pub fn print(p: &Project, from: Option<usize>, lines: Option<usize>) -> String {
    timeline::print_model(&timeline::print_receipt(p, from, lines))
}

/// `apply_timeline`: plans the edit with the 3-way merge and applies it as one agent revision.
pub fn apply(p: &mut Project, req: ApplyRequest) -> Result<String, String> {
    run_on(p, "apply_timeline", &req.args())
}

/// `verify_timeline`.
pub fn verify(p: &Project) -> String {
    match timeline::verify_receipt(p) {
        Ok(r) => r.text,
        Err(f) => f.message,
    }
}

/// `list_media`.
pub fn list_media(p: &Project) -> String {
    media::list_receipt(p, false).json.to_string()
}

/// `check_code`: the motion-graphics check as compact JSON. With a project, the frame size follows its aspect ratio
/// and image assets (media ids or names) load from its media.
pub fn check_code(p: Option<&Project>, composition: &Value, duration: f64) -> String {
    code::check(p, composition, duration).to_string()
}

/// `undo_timeline` (formerly `undo_last`): reverts the latest revision if it was the assistant's.
pub fn undo_last(p: &mut Project) -> Result<String, String> {
    run_on(p, "undo_timeline", &Map::new())
}
