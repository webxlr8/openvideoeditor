//! The agent tool registry (change unify-agent-tool-layer, design D1): every tool an agent can call is described
//! once — name, description, input schema, annotations and the surfaces it appears on — and has one handler. The
//! in-app assistant, `ove mcp` and the pinned server list and call tools through this crate, and the `ove` command
//! line calls the same handlers, so the surfaces cannot drift apart.
//!
//! A handler sees only a [`ToolCtx`]: [`BundleCtx`] (projects on disk), [`LinkCtx`] (on disk, through the running
//! app when it holds the project) or [`AppCtx`] (the project open in the desktop app).
#![forbid(unsafe_code)]

pub mod app;
pub mod bundle;
pub mod ctx;
pub mod fail;
pub mod receipt;
pub mod registry;
pub mod spec;
pub mod tools;

use serde_json::{Map, Value, json};

pub use app::{AppCtx, AppHost, ProjectHost};
pub use bundle::{Binding, BundleCtx, LinkCtx, ProgressStyle};
pub use ctx::{Editor, Surface, ToolCtx};
pub use fail::Fail;
pub use receipt::{Png, Receipt};
pub use registry::TOOLS;
pub use spec::{Args, ToolSpec};

/// A tool by name or former name.
pub fn find(name: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|t| t.name == name).or_else(|| TOOLS.iter().find(|t| t.aliases.contains(&name)))
}

/// The tools that appear on a surface, in registry order.
pub fn tools_on(surface: Surface) -> impl Iterator<Item = &'static ToolSpec> {
    TOOLS.iter().filter(move |t| t.on(surface))
}

fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != *cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

/// Up to three tool names on `surface` closest to `name` (shared words count, then edit distance).
pub fn suggest(name: &str, surface: Surface) -> Vec<&'static str> {
    let words: Vec<&str> = name.split('_').filter(|w| !w.is_empty()).collect();
    let mut scored: Vec<(usize, &'static str)> = tools_on(surface)
        .map(|t| {
            let shared = t.name.split('_').filter(|w| words.contains(w)).count();
            (distance(name, t.name).saturating_sub(shared * 4), t.name)
        })
        .collect();
    scored.sort();
    scored.into_iter().take(3).map(|(_, n)| n).collect()
}

/// The refusal for a tool name that does not exist on `surface`.
pub fn unknown(name: &str, surface: Surface) -> Fail {
    let close = suggest(name, surface);
    Fail::new("tool.unknown", format!("there is no tool `{name}`; closest: {}", close.join(", ")))
        .details(json!({"suggestions": close}))
}

/// Validates the arguments and runs a tool.
pub fn call(ctx: &mut dyn ToolCtx, name: &str, args: &Map<String, Value>) -> Result<Receipt, Fail> {
    let surface = ctx.surface();
    let t = find(name).filter(|t| t.on(surface)).ok_or_else(|| unknown(name, surface))?;
    spec::validate(t, args, surface)?;
    (t.run)(ctx, &Args(args))
}

/// The MCP `tools/list` entries for a surface.
pub fn mcp_list(surface: Surface) -> Value {
    let tools: Vec<Value> = tools_on(surface)
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.about,
                "inputSchema": t.schema(),
                "annotations": {
                    "readOnlyHint": t.annotations.read_only,
                    "destructiveHint": t.annotations.destructive,
                    "openWorldHint": t.annotations.spends,
                },
            })
        })
        .collect();
    json!({"tools": tools})
}

/// The in-app assistant's tool definitions (Messages API shape), in registry order. Inputs stream as they are
/// generated and are validated here before running.
pub fn anthropic_list() -> Vec<Value> {
    tools_on(Surface::InApp)
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.about,
                "input_schema": t.schema(),
                "eager_input_streaming": true,
            })
        })
        .collect()
}

/// The tool table of docs/cli/tools.md (kept in step with the registry by a test).
pub fn docs_table() -> String {
    let mut s = String::from("| Tool | Former names | Surfaces | Kind | Arguments |\n|---|---|---|---|---|\n");
    for t in TOOLS {
        let mut on = Vec::new();
        for (bit, name) in [(spec::IN_APP, "in-app"), (spec::MCP, "mcp"), (spec::PINNED, "pinned")] {
            if t.surfaces & bit != 0 {
                on.push(name);
            }
        }
        let a = t.annotations;
        let kind = match (a.read_only, a.destructive, a.spends, a.needs_app) {
            (_, _, true, _) => "spends",
            (_, true, _, _) => "destructive",
            (true, _, _, true) => "read, app",
            (true, ..) => "read",
            (false, _, _, true) => "write, app",
            _ => "write",
        };
        let args: Vec<String> = t
            .args
            .iter()
            .filter(|x| !x.cli_only)
            .map(|x| if x.required { format!("`{}`", x.name) } else { format!("`{}`?", x.name) })
            .collect();
        s.push_str(&format!(
            "| `{}` | {} | {} | {} | {} |\n",
            t.name,
            t.aliases.iter().map(|x| format!("`{x}`")).collect::<Vec<_>>().join(", "),
            on.join(", "),
            kind,
            args.join(" ")
        ));
    }
    s
}
