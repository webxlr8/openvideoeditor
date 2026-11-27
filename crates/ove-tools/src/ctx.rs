//! Where a tool runs. A handler sees only [`ToolCtx`]: it names a project, reads it, edits it through an [`Editor`],
//! and calls the running app. Three contexts implement it: [`crate::BundleCtx`] (a project on disk, headless),
//! [`crate::LinkCtx`] (on disk, forwarding edits to the running app when it has the project open) and
//! [`crate::AppCtx`] (the project open in the desktop app).

use std::path::{Path, PathBuf};

use ove_core::model::Document;
use ove_core::ops::Op;
use ove_core::revision::{Actor, Source};
use ove_store::Library;
use ove_store::project::Project;
use serde_json::Value;

use crate::fail::Fail;
use crate::session::Session;

/// The agent surface a call arrives on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// The in-app assistant.
    InApp,
    /// `ove mcp`.
    Mcp,
    /// `ove mcp --project` (the pinned server for local agents).
    Pinned,
    /// The `ove` command line (every tool, plus options only people use, such as output paths).
    Cli,
}

impl Surface {
    pub fn as_str(self) -> &'static str {
        match self {
            Surface::InApp => "inApp",
            Surface::Mcp => "mcp",
            Surface::Pinned => "pinned",
            Surface::Cli => "cli",
        }
    }

    /// True for the surfaces agents use (everything but the command line).
    pub fn is_agent(self) -> bool {
        self != Surface::Cli
    }
}

/// A project open for one edit. Changes are recorded with the context's source.
pub trait Editor {
    fn project(&self) -> &Project;
    /// Applies one batch as one revision; returns the new revision.
    fn apply(&mut self, ops: Vec<Op>, summary: String) -> Result<i64, Fail>;
    /// Returns to the state of revision `target` as a new revision (source `undo`): `Some((rev, summary))`, or `None`
    /// when already there.
    fn restore(&mut self, target: i64) -> Result<Option<(i64, String)>, Fail>;
    /// [`Editor::apply`] for a ripple edit, recording how it moved time per track so projections printed before it
    /// rebase onto it. Through the running app the time map is not carried over (the app records the batch).
    fn apply_mapped(
        &mut self,
        ops: Vec<Op>,
        summary: String,
        _time_map: ove_core::timemap::TimeMap,
    ) -> Result<i64, Fail> {
        self.apply(ops, summary)
    }
    /// Reverts the revisions `revs` as one new revision (source `undo`), keeping every other change; refused with
    /// `undo.conflict` when later changes touched the same fields.
    fn undo_selected(&mut self, revs: &[i64], summary: String) -> Result<i64, Fail>;
    /// [`Editor::apply`] recorded as `actor`'s change instead of the context's (a generation job filling the
    /// placeholder its agent asked for). Editors that cannot record another actor apply it as their own.
    fn apply_by(&mut self, ops: Vec<Op>, summary: String, _actor: Option<Actor>) -> Result<i64, Fail> {
        self.apply(ops, summary)
    }
    /// Records the warnings listed in the next printed header.
    fn set_diagnostics(&mut self, lines: Vec<String>) -> Result<(), Fail>;
    /// True when the edit goes through the running app.
    fn via_app(&self) -> bool {
        false
    }
    fn doc(&self) -> &Document {
        self.project().doc()
    }
    fn rev(&self) -> i64 {
        self.project().rev()
    }
    fn bundle(&self) -> &Path {
        self.project().bundle()
    }
}

/// What a handler can do.
pub trait ToolCtx {
    fn surface(&self) -> Surface;
    /// The revision source of edits made here.
    fn source(&self) -> Source;
    /// Who makes the edits made here (recorded on each revision).
    fn actor(&self) -> Option<Actor> {
        None
    }
    /// The client's name, from MCP `initialize`.
    fn set_client(&mut self, _name: &str) {}
    /// Makes `bundle` the session's project for calls that name none (`use_project`).
    fn use_project(&mut self, _bundle: &Path) -> Result<(), Fail> {
        Err(Fail::new("tool.unavailable", "this session's project is the one open in the app"))
    }
    /// The bundle of the project a call names; `None` means the session's project (the open one in the app, or the
    /// one a session is bound to). Refused when there is none or when the session may not use that project.
    fn bundle(&mut self, project: Option<&str>) -> Result<PathBuf, Fail>;
    /// Runs `f` on the project, read-only.
    fn read(&mut self, bundle: &Path, f: &mut dyn FnMut(&Project) -> Result<(), Fail>) -> Result<(), Fail>;
    /// Runs `f` with the project open for editing. `f` may run again when the project moved on meanwhile (an
    /// attempt refused with `rev.changed`).
    fn edit(&mut self, bundle: &Path, f: &mut dyn FnMut(&mut dyn Editor) -> Result<(), Fail>) -> Result<(), Fail>;
    /// Calls the running app (`context`, `open`, `seek`, `play`, `pause`).
    fn app(&mut self, method: &str, params: Value) -> Result<Value, Fail>;
    /// The project library (listing and creating projects).
    fn library(&mut self) -> Result<&Library, Fail>;
    /// Progress of a long call (export), 0–100, with the details as JSON. Only the command line shows it.
    fn progress(&mut self, _percent: i64, _detail: &Value) {}
    /// A long call that reported progress finished.
    fn progress_done(&mut self) {}
    /// The skill store: the library's `.skills` folder.
    fn skills(&mut self) -> Result<crate::skills::SkillStore, Fail> {
        Ok(crate::skills::SkillStore::of_library(&self.library()?.root))
    }
    /// The session's state kept between calls (quotes, spend, recent tools). `None`: this context has no session, so
    /// generation and feedback are not available.
    fn session(&self) -> Option<Session> {
        None
    }
    /// Shows the person a priced request (`card`) and waits for their answer: `Ok(true)` approved, `Ok(false)`
    /// declined. Only a person answers: the running app's price card, the in-app question card, or the MCP client's
    /// confirmation prompt. Refused with `confirm.required` when no person can be asked here.
    fn ask_spend(&mut self, _card: &Value) -> Result<bool, Fail> {
        Err(confirm_required())
    }
    /// Starts the generation worker for a saved job (`ove-generate` job file in `bundle`).
    fn start_generation(&mut self, _bundle: &Path, _job: &str) -> Result<(), Fail> {
        Err(Fail::new("generate.unavailable", "generation jobs cannot be started here"))
    }
}

/// The refusal when spending needs a person and none can be asked.
pub fn confirm_required() -> Fail {
    Fail::new(
        "confirm.required",
        "Spending needs a person's approval, and none could be asked: open OpenVideoEditor (it shows a price card), \
         or use an MCP client that supports confirmation prompts (elicitation). Nothing was spent.",
    )
}

fn internal() -> Fail {
    Fail::new("internal", "the tool context did not run the handler")
}

/// Runs `f` on the project read-only and returns its result.
pub fn read<T>(ctx: &mut dyn ToolCtx, bundle: &Path, f: impl FnOnce(&Project) -> Result<T, Fail>) -> Result<T, Fail> {
    let mut f = Some(f);
    let mut out = None;
    ctx.read(bundle, &mut |p| {
        if let Some(f) = f.take() {
            out = Some(f(p)?);
        }
        Ok(())
    })?;
    out.ok_or_else(internal)
}

/// A copy of the project's document with its bundle and revision, read without holding the project while the
/// caller works (decoding frames can take seconds).
pub fn snapshot(ctx: &mut dyn ToolCtx, bundle: &Path) -> Result<(Document, PathBuf, i64), Fail> {
    read(ctx, bundle, |p| Ok((p.doc().clone(), p.bundle().to_path_buf(), p.rev())))
}

/// Runs an edit and returns its result. `f` may run more than once (see [`ToolCtx::edit`]).
pub fn edit<T>(
    ctx: &mut dyn ToolCtx,
    bundle: &Path,
    mut f: impl FnMut(&mut dyn Editor) -> Result<T, Fail>,
) -> Result<T, Fail> {
    let mut out = None;
    ctx.edit(bundle, &mut |w| {
        out = Some(f(w)?);
        Ok(())
    })?;
    out.ok_or_else(internal)
}

/// `" (in the app)"` when the edit went through the running app.
pub fn via(w: &dyn Editor) -> &'static str {
    if w.via_app() { " (in the app)" } else { "" }
}

/// An editor over a project this process holds open for writing.
pub struct LocalEditor<'a> {
    pub project: &'a mut Project,
    pub source: Source,
    pub actor: Option<Actor>,
}

impl Editor for LocalEditor<'_> {
    fn project(&self) -> &Project {
        self.project
    }
    fn apply(&mut self, ops: Vec<Op>, summary: String) -> Result<i64, Fail> {
        self.project.apply_as(ops, self.source, Some(summary), self.actor.clone())?;
        Ok(self.project.rev())
    }
    fn apply_by(&mut self, ops: Vec<Op>, summary: String, actor: Option<Actor>) -> Result<i64, Fail> {
        self.project.apply_as(ops, self.source, Some(summary), actor.or_else(|| self.actor.clone()))?;
        Ok(self.project.rev())
    }
    fn apply_mapped(
        &mut self,
        ops: Vec<Op>,
        summary: String,
        time_map: ove_core::timemap::TimeMap,
    ) -> Result<i64, Fail> {
        self.project.apply_mapped_as(ops, self.source, Some(summary), time_map, self.actor.clone())?;
        Ok(self.project.rev())
    }
    fn undo_selected(&mut self, revs: &[i64], summary: String) -> Result<i64, Fail> {
        undo_on(self.project, revs, summary, self.actor.clone())
    }
    fn restore(&mut self, target: i64) -> Result<Option<(i64, String)>, Fail> {
        Ok(self.project.restore_as(target, Source::Undo)?.map(|c| (c.record.rev, c.record.summary)))
    }
    fn set_diagnostics(&mut self, lines: Vec<String>) -> Result<(), Fail> {
        // Unchanged diagnostics (the usual case) are not written again: that saves a file write and a second
        // projection print on every edit.
        let on_disk = std::fs::read(self.project.bundle().join("cache/diagnostics.json")).ok();
        let unchanged = match on_disk {
            Some(bytes) => serde_json::from_slice::<Vec<String>>(&bytes).is_ok_and(|old| old == lines),
            None => lines.is_empty(),
        };
        if unchanged {
            return Ok(());
        }
        Ok(self.project.set_diagnostics(lines)?)
    }
}

/// Selective undo on a project this process holds, with the refusal agents see.
pub fn undo_on(p: &mut Project, revs: &[i64], summary: String, actor: Option<Actor>) -> Result<i64, Fail> {
    use ove_store::undo::UndoError;
    match p.undo_selected(revs, Source::Undo, summary, actor) {
        Ok(c) => Ok(c.record.rev),
        Err(UndoError::Nothing) => Err(Fail::new("undo.nothing", "There is nothing to undo.")),
        Err(UndoError::Store(e)) => Err(e.into()),
        Err(UndoError::Conflict(c)) => {
            let items: Vec<String> = c
                .iter()
                .map(|x| {
                    let what = x.item.as_deref().unwrap_or(&x.object);
                    format!("{what} {} (changed later in rev {}: {})", x.field, x.rev, x.summary)
                })
                .collect();
            Err(Fail::new(
                "undo.conflict",
                format!("Not undone: later changes touched the same fields: {}", items.join("; ")),
            )
            .details(serde_json::json!({"conflicts": c})))
        }
    }
}
