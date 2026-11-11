//! Shared plumbing: refusals, the library, and the write path (local, or through the running app when it has the
//! project open).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ove_core::model::Document;
use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_projection::Refusal;
use ove_store::project::{Mode, Project};
use ove_store::{Library, StoreError};
use serde_json::{Value, json};

/// A refusal: code, message and optional details for JSON.
#[derive(Debug)]
pub struct Fail {
    pub code: String,
    pub message: String,
    pub details: Value,
}

impl Fail {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Fail {
        Fail { code: code.into(), message: message.into(), details: Value::Null }
    }

    pub fn details(mut self, v: Value) -> Fail {
        self.details = v;
        self
    }
}

impl From<StoreError> for Fail {
    fn from(e: StoreError) -> Self {
        Fail::new(e.code(), e.to_string())
    }
}

impl From<Refusal> for Fail {
    fn from(r: Refusal) -> Self {
        Fail { code: r.code.clone(), message: r.to_string(), details: json!({"diagnostics": r.diagnostics}) }
    }
}

impl From<ove_control::CallError> for Fail {
    fn from(e: ove_control::CallError) -> Self {
        let message = match &e {
            ove_control::CallError::NotRunning => {
                "the app is not running (start OpenVideoEditor, or use a command that works on files)".to_string()
            }
            other => other.to_string(),
        };
        Fail::new(e.code(), message)
    }
}

pub type Out = Result<(Value, String), Fail>;

/// False while serving MCP: progress must not reach stdout.
static PROGRESS: AtomicBool = AtomicBool::new(true);

pub fn set_progress(on: bool) {
    PROGRESS.store(on, Ordering::SeqCst);
}

pub fn progress_enabled() -> bool {
    PROGRESS.load(Ordering::SeqCst)
}

pub fn clock() -> Arc<dyn ove_core::Clock> {
    Arc::new(ove_core::SystemClock)
}

pub fn now_ms() -> i64 {
    ove_core::Clock::now_ms(&ove_core::SystemClock)
}

/// What every command gets: the library, where edits come from, and the output mode.
pub struct Ctx {
    pub lib: Library,
    pub root: PathBuf,
    pub source: Source,
    pub json: bool,
}

impl Ctx {
    pub fn open(library: Option<&Path>, source: Source, json: bool) -> Result<Ctx, Fail> {
        let root = match library {
            Some(p) => {
                ove_store::library::validate_library(p)?;
                p.to_path_buf()
            }
            None => ove_store::library::resolve_library(None)?,
        };
        Ok(Ctx { lib: Library::open(&root, clock())?, root, source, json })
    }

    pub fn find(&self, project: &str) -> Result<PathBuf, Fail> {
        Ok(self.lib.find(project)?)
    }

    pub fn read(&self, project: &str) -> Result<Project, Fail> {
        Ok(Project::open(&self.find(project)?, Mode::ReadOnly, clock())?)
    }

    /// Calls the running app.
    pub fn app(&self, method: &str, params: Value) -> Result<Value, Fail> {
        Ok(ove_control::call(&self.root, method, params, Duration::from_secs(20))?)
    }
}

fn same_bundle(a: &Path, b: &Path) -> bool {
    let c = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    c(a) == c(b)
}

/// Where edits go: the bundle itself (we hold the write lock), or the running app, which has the project open and
/// applies them as revisions in its session (the editor updates at once).
pub enum Writer {
    Local(Project),
    App { view: Project, root: PathBuf },
}

impl Writer {
    pub fn open(ctx: &Ctx, bundle: &Path) -> Result<Writer, Fail> {
        match Project::open(bundle, Mode::Write, clock()) {
            Ok(p) => Ok(Writer::Local(p)),
            Err(e) if e.code() == "project.locked" => {
                // Held by the app with this project open? Then edit through it.
                let ctx_v = ove_control::call(&ctx.root, "context", json!({}), Duration::from_secs(5));
                let open_bundle =
                    ctx_v.ok().and_then(|v| v.pointer("/project/bundle").and_then(Value::as_str).map(PathBuf::from));
                match open_bundle {
                    Some(b) if same_bundle(&b, bundle) => Ok(Writer::App {
                        view: Project::open(bundle, Mode::ReadOnly, clock())?,
                        root: ctx.root.clone(),
                    }),
                    _ => Err(e.into()),
                }
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn project(&self) -> &Project {
        match self {
            Writer::Local(p) => p,
            Writer::App { view, .. } => view,
        }
    }

    pub fn doc(&self) -> &Document {
        self.project().doc()
    }

    pub fn rev(&self) -> i64 {
        self.project().rev()
    }

    pub fn bundle(&self) -> &Path {
        self.project().bundle()
    }

    pub fn via_app(&self) -> bool {
        matches!(self, Writer::App { .. })
    }

    fn reload(&mut self) -> Result<(), Fail> {
        if let Writer::App { view, .. } = self {
            *view = Project::open(view.bundle(), Mode::ReadOnly, clock())?;
        }
        Ok(())
    }

    /// Applies one batch as one revision; returns the new revision.
    pub fn apply(&mut self, ops: Vec<Op>, source: Source, summary: String) -> Result<i64, Fail> {
        match self {
            Writer::Local(p) => {
                p.apply(ops, source, Some(summary))?;
                Ok(p.rev())
            }
            Writer::App { view, root } => {
                let params = json!({
                    "bundle": view.bundle(),
                    "expectRev": view.rev(),
                    "ops": ops,
                    "summary": summary,
                    "source": source,
                });
                let v = ove_control::call(root, "apply", params, Duration::from_secs(30))?;
                self.reload()?;
                Ok(v.get("rev").and_then(Value::as_i64).unwrap_or_else(|| self.rev()))
            }
        }
    }

    /// Returns to the state of revision `target` as a new revision: `Some((rev, summary))`, or `None` when already
    /// there.
    pub fn restore(&mut self, target: i64, source: Source) -> Result<Option<(i64, String)>, Fail> {
        match self {
            Writer::Local(p) => Ok(p.restore_as(target, source)?.map(|c| (c.record.rev, c.record.summary))),
            Writer::App { view, root } => {
                let params = json!({"bundle": view.bundle(), "rev": target, "expectRev": view.rev()});
                let v = ove_control::call(root, "restore", params, Duration::from_secs(30))?;
                self.reload()?;
                Ok(v.get("rev")
                    .and_then(Value::as_i64)
                    .map(|r| (r, v.get("summary").and_then(Value::as_str).unwrap_or_default().to_string())))
            }
        }
    }

    pub fn set_diagnostics(&mut self, lines: Vec<String>) -> Result<(), Fail> {
        if let Writer::Local(p) = self {
            p.set_diagnostics(lines)?;
        }
        Ok(())
    }

    /// Writes the projection and releases the lock.
    pub fn finish(self) -> Result<(), Fail> {
        if let Writer::Local(p) = self {
            p.flush_projection();
            p.close()?;
        }
        Ok(())
    }
}

/// Runs an edit, retrying (up to 3 times) when the app's project moved on between reading and applying.
pub fn edit<T>(ctx: &Ctx, project: &str, mut f: impl FnMut(&mut Writer) -> Result<T, Fail>) -> Result<T, Fail> {
    let bundle = ctx.find(project)?;
    let mut attempt = 0;
    loop {
        let mut w = Writer::open(ctx, &bundle)?;
        match f(&mut w) {
            Err(e) if e.code == "rev.changed" && attempt < 3 => attempt += 1,
            Err(e) => return Err(e),
            Ok(v) => {
                w.finish()?;
                return Ok(v);
            }
        }
    }
}

/// `" (in the app)"` when the edit went through the running app.
pub fn via(w: &Writer) -> &'static str {
    if w.via_app() { " (in the app)" } else { "" }
}

pub fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

/// Seconds → µs, refusing negatives and non-numbers.
pub fn secs(s: f64, what: &str) -> Result<i64, Fail> {
    ove_core::time::secs_to_micros(s)
        .filter(|v| *v >= 0)
        .ok_or_else(|| Fail::new("argument.invalid", format!("{what} must be a number of seconds ≥ 0")))
}
