//! `docs`: the built-in documentation (the pages in docs/cli).

use serde_json::json;

use crate::ctx::ToolCtx;
use crate::fail::Fail;
use crate::receipt::Receipt;
use crate::spec::Args;

/// Built-in documentation pages: name, one-line description, text.
pub const TOPICS: &[(&str, &str, &str)] = &[
    ("agents", "How an agent edits a project with ove (start here)", include_str!("../../../../docs/cli/agents.md")),
    (
        "timeline",
        "source print / check / apply, verify, history, undo",
        include_str!("../../../../docs/cli/timeline.md"),
    ),
    (
        "edit",
        "edit verbs: split, trim, ripple, insert, overwrite, lift, extract, slip, slide, roll",
        include_str!("../../../../docs/cli/edit.md"),
    ),
    (
        "project",
        "project list / create / show / rename / duplicate / remove / open / reveal",
        include_str!("../../../../docs/cli/project.md"),
    ),
    ("media", "media list / add / remove / relink / probe", include_str!("../../../../docs/cli/media.md")),
    (
        "app",
        "app context / open / seek / play / pause and how edits reach the running app",
        include_str!("../../../../docs/cli/app.md"),
    ),
    ("export", "export to MP4", include_str!("../../../../docs/cli/export.md")),
    (
        "interchange",
        "timeline export / import (FCPXML, XML, EDL, OTIO)",
        include_str!("../../../../docs/cli/interchange.md"),
    ),
    ("mcp", "the MCP server and agent setup", include_str!("../../../../docs/cli/mcp.md")),
    (
        "tools",
        "every agent tool, its arguments and the surfaces it appears on",
        include_str!("../../../../docs/cli/tools.md"),
    ),
];

/// The topic list, or one page.
pub fn page(topic: Option<&str>) -> Result<Receipt, Fail> {
    match topic {
        None => {
            let text = TOPICS.iter().map(|(n, about, _)| format!("{n}\t{about}")).collect::<Vec<_>>().join("\n");
            let v = json!({"topics": TOPICS.iter().map(|(n, about, _)| json!({"name": n, "about": about})).collect::<Vec<_>>()});
            Ok(Receipt::new(v, format!("{text}\n\n`ove docs <topic>` prints one.")))
        }
        Some(t) => {
            let (n, _, body) = TOPICS
                .iter()
                .find(|(n, ..)| n.eq_ignore_ascii_case(t))
                .ok_or_else(|| Fail::new("docs.notFound", format!("no topic {t}; `ove docs` lists them")))?;
            Ok(Receipt::new(json!({"topic": n, "text": body}), body.trim_end().to_string()))
        }
    }
}

pub fn docs(_: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    page(a.str("topic"))
}
