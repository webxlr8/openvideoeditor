//! Tool definitions: one name, description and input schema per tool, its annotations, and the surfaces it appears
//! on. Schemas are written by hand from the argument tables (better descriptions for agents than derived ones).

use serde_json::{Map, Value, json};

use crate::ctx::{Surface, ToolCtx};
use crate::fail::Fail;
use crate::receipt::Receipt;

/// The JSON type of an argument.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Str,
    Num,
    Int,
    Bool,
    /// A non-empty list of strings.
    List,
    /// Any JSON value matching the given JSON Schema text (checked by the handler).
    Json(&'static str),
}

/// One argument.
#[derive(Debug, Clone, Copy)]
pub struct Arg {
    pub name: &'static str,
    pub kind: Kind,
    pub required: bool,
    pub about: &'static str,
    /// Only the command line passes it (output paths and such); agents never see it.
    pub cli_only: bool,
}

pub const fn arg(name: &'static str, kind: Kind, about: &'static str) -> Arg {
    Arg { name, kind, required: false, about, cli_only: false }
}

pub const fn req(name: &'static str, kind: Kind, about: &'static str) -> Arg {
    Arg { name, kind, required: true, about, cli_only: false }
}

pub const fn cli(name: &'static str, kind: Kind, about: &'static str) -> Arg {
    Arg { name, kind, required: false, about, cli_only: true }
}

/// Hints for clients and for the app's confirmation rules.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Annotations {
    /// Changes nothing.
    pub read_only: bool,
    /// Removes or reverts work.
    pub destructive: bool,
    /// Can spend the person's money.
    pub spends: bool,
    /// Needs the running app.
    pub needs_app: bool,
}

pub const READ: Annotations = Annotations { read_only: true, destructive: false, spends: false, needs_app: false };
pub const WRITE: Annotations = Annotations { read_only: false, destructive: false, spends: false, needs_app: false };
pub const DESTRUCTIVE: Annotations =
    Annotations { read_only: false, destructive: true, spends: false, needs_app: false };
pub const APP_READ: Annotations = Annotations { read_only: true, destructive: false, spends: false, needs_app: true };
pub const APP_WRITE: Annotations = Annotations { read_only: false, destructive: false, spends: false, needs_app: true };

/// Surfaces a tool appears on (the command line reaches every tool).
pub const IN_APP: u8 = 1;
pub const MCP: u8 = 2;
pub const PINNED: u8 = 4;
pub const AGENTS: u8 = IN_APP | MCP | PINNED;
pub const SERVERS: u8 = MCP | PINNED;

pub type Handler = fn(&mut dyn ToolCtx, &Args) -> Result<Receipt, Fail>;

/// How a receipt reads for a model.
#[derive(Clone, Copy)]
pub enum ModelText {
    /// The JSON, compact.
    Json,
    /// The receipt's text.
    Text,
    /// A tool-specific rendering.
    With(fn(&Receipt) -> String),
}

/// One tool.
pub struct ToolSpec {
    pub name: &'static str,
    /// Former names, accepted for one release.
    pub aliases: &'static [&'static str],
    pub about: &'static str,
    pub args: &'static [Arg],
    pub annotations: Annotations,
    pub surfaces: u8,
    pub run: Handler,
    pub model: ModelText,
}

impl std::fmt::Debug for ToolSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolSpec").field("name", &self.name).finish()
    }
}

impl ToolSpec {
    /// True when the tool appears on `s`.
    pub fn on(&self, s: Surface) -> bool {
        match s {
            Surface::InApp => self.surfaces & IN_APP != 0,
            Surface::Mcp => self.surfaces & MCP != 0,
            Surface::Pinned => self.surfaces & PINNED != 0,
            Surface::Cli => true,
        }
    }

    /// The input schema agents see (command-line-only arguments left out).
    pub fn schema(&self) -> Value {
        let mut props = Map::new();
        let mut required = Vec::new();
        for a in self.args.iter().filter(|a| !a.cli_only) {
            let mut ty = match a.kind {
                Kind::Str => json!({"type": "string"}),
                Kind::Num => json!({"type": "number"}),
                Kind::Int => json!({"type": "integer"}),
                Kind::Bool => json!({"type": "boolean"}),
                Kind::List => json!({"type": "array", "items": {"type": "string"}, "minItems": 1}),
                Kind::Json(s) => serde_json::from_str(s).unwrap_or_else(|_| json!({})),
            };
            if let Some(o) = ty.as_object_mut()
                && !a.about.is_empty()
            {
                o.insert("description".into(), a.about.into());
            }
            props.insert(a.name.into(), ty);
            if a.required {
                required.push(a.name);
            }
        }
        json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
    }

    /// The receipt as text for a model.
    pub fn model_text(&self, r: &Receipt) -> String {
        match self.model {
            ModelText::Json => r.json.to_string(),
            ModelText::Text => r.text.clone(),
            ModelText::With(f) => f(r),
        }
    }
}

/// Validated call arguments.
#[derive(Debug, Clone, Copy)]
pub struct Args<'a>(pub &'a Map<String, Value>);

impl<'a> Args<'a> {
    fn get(&self, k: &str) -> Option<&'a Value> {
        self.0.get(k).filter(|v| !v.is_null())
    }
    pub fn has(&self, k: &str) -> bool {
        self.get(k).is_some()
    }
    pub fn str(&self, k: &str) -> Option<&'a str> {
        self.get(k).and_then(Value::as_str)
    }
    pub fn req_str(&self, k: &str) -> Result<&'a str, Fail> {
        self.str(k).ok_or_else(|| Fail::new("argument.missing", format!("`{k}` is required")))
    }
    pub fn f64(&self, k: &str) -> Option<f64> {
        self.get(k).and_then(Value::as_f64)
    }
    pub fn i64(&self, k: &str) -> Option<i64> {
        self.get(k).and_then(Value::as_i64)
    }
    pub fn u32_or(&self, k: &str, default: u32) -> u32 {
        self.i64(k).map(|v| v.clamp(0, u32::MAX as i64) as u32).unwrap_or(default)
    }
    pub fn bool(&self, k: &str) -> bool {
        self.get(k).and_then(Value::as_bool).unwrap_or(false)
    }
    pub fn strings(&self, k: &str) -> Vec<String> {
        self.get(k)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    }
    pub fn value(&self, k: &str) -> Option<&'a Value> {
        self.get(k)
    }
    /// The `project` argument.
    pub fn project(&self) -> Option<&'a str> {
        self.str("project")
    }
}

/// Checks arguments against a tool's table: no unknown names (command-line-only ones only from the command line),
/// required ones present, types right.
pub fn validate(t: &ToolSpec, args: &Map<String, Value>, surface: Surface) -> Result<(), Fail> {
    for k in args.keys() {
        match t.args.iter().find(|a| a.name == k) {
            Some(a) if !a.cli_only || surface == Surface::Cli => {}
            _ => return Err(Fail::new("argument.unknown", format!("{} has no argument `{k}`", t.name))),
        }
    }
    for a in t.args {
        let Some(v) = args.get(a.name).filter(|v| !v.is_null()) else {
            if a.required {
                return Err(Fail::new("argument.missing", format!("{} needs `{}`", t.name, a.name)));
            }
            continue;
        };
        let ok = match a.kind {
            Kind::Str => v.is_string(),
            Kind::Num => v.as_f64().is_some_and(f64::is_finite),
            Kind::Int => v.is_i64() || v.is_u64(),
            Kind::Bool => v.is_boolean(),
            Kind::List => v.as_array().is_some_and(|l| !l.is_empty() && l.iter().all(Value::is_string)),
            Kind::Json(s) => {
                match serde_json::from_str::<Value>(s).ok().and_then(|s| s["type"].as_str().map(str::to_string)) {
                    Some(t) if t == "object" => v.is_object(),
                    Some(t) if t == "array" => v.is_array(),
                    Some(t) if t == "string" => v.is_string(),
                    _ => true,
                }
            }
        };
        if !ok {
            let want = match a.kind {
                Kind::Str => "a string",
                Kind::Num => "a number",
                Kind::Int => "an integer",
                Kind::Bool => "true or false",
                Kind::List => "a non-empty list of strings",
                Kind::Json(_) => "of the type its schema gives",
            };
            return Err(Fail::new("argument.invalid", format!("`{}` must be {want}", a.name)));
        }
    }
    Ok(())
}
