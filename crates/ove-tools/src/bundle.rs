//! Projects on disk: [`BundleCtx`] works on bundles headless (it takes each project's write lock), and [`LinkCtx`]
//! does the same but, when the running app holds a project open, sends the edit to the app over the control link
//! so it becomes a revision of the app's session and the editor updates at once.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_store::Library;
use ove_store::project::{Mode, Project};
use serde_json::{Value, json};

use crate::ctx::{Editor, LocalEditor, Surface, ToolCtx};
use crate::fail::Fail;

pub fn clock() -> Arc<dyn ove_core::Clock> {
    Arc::new(ove_core::SystemClock)
}

/// True when two paths name the same folder.
pub fn same_bundle(a: &Path, b: &Path) -> bool {
    let c = |p: &Path| dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    c(a) == c(b)
}

/// The project a session works on when a call names none, and whether it may name others.
#[derive(Debug, Clone, Default)]
pub struct Binding {
    /// The default project's bundle.
    pub project: Option<PathBuf>,
    /// Bound for the whole session (`ove mcp --project`): calls naming another project are refused.
    pub pinned: bool,
}

/// Projects in a library on disk, opened headless.
pub struct BundleCtx {
    pub lib: Library,
    pub root: PathBuf,
    pub surface: Surface,
    pub source: Source,
    pub binding: Binding,
}

impl BundleCtx {
    /// Opens the library at `library` (`None`: `OVE_LIBRARY`, then the platform default).
    pub fn open(library: Option<&Path>, surface: Surface, source: Source) -> Result<BundleCtx, Fail> {
        let root = match library {
            Some(p) => {
                ove_store::library::validate_library(p)?;
                p.to_path_buf()
            }
            None => ove_store::library::resolve_library(None)?,
        };
        Ok(BundleCtx { lib: Library::open(&root, clock())?, root, surface, source, binding: Binding::default() })
    }

    /// The bundle of a project by name, id or path.
    pub fn find(&self, project: &str) -> Result<PathBuf, Fail> {
        Ok(self.lib.find(project)?)
    }

    /// Opens a project read-only.
    pub fn open_read(&self, bundle: &Path) -> Result<Project, Fail> {
        Ok(Project::open(bundle, Mode::ReadOnly, clock())?)
    }

    fn resolve(&self, project: Option<&str>) -> Result<PathBuf, Fail> {
        match (project, &self.binding.project) {
            (None, Some(b)) => Ok(b.clone()),
            (None, None) => Err(Fail::new(
                "argument.missing",
                "this call needs `project` (a project name, id or bundle path; `use_project` sets a default)",
            )),
            (Some(q), bound) => {
                let b = self.find(q)?;
                if self.binding.pinned
                    && let Some(bound) = bound
                    && !same_bundle(bound, &b)
                {
                    return Err(Fail::new(
                        "project.pinned",
                        format!(
                            "this session works on {} only; start another session for {q}",
                            bound.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                        ),
                    ));
                }
                Ok(b)
            }
        }
    }

    fn edit_local(
        &mut self,
        bundle: &Path,
        f: &mut dyn FnMut(&mut dyn Editor) -> Result<(), Fail>,
    ) -> Result<(), Fail> {
        let mut p = Project::open(bundle, Mode::Write, clock())?;
        f(&mut LocalEditor { project: &mut p, source: self.source })?;
        p.flush_projection();
        p.close()?;
        Ok(())
    }
}

impl ToolCtx for BundleCtx {
    fn surface(&self) -> Surface {
        self.surface
    }
    fn source(&self) -> Source {
        self.source
    }
    fn bundle(&mut self, project: Option<&str>) -> Result<PathBuf, Fail> {
        self.resolve(project)
    }
    fn read(&mut self, bundle: &Path, f: &mut dyn FnMut(&Project) -> Result<(), Fail>) -> Result<(), Fail> {
        f(&self.open_read(bundle)?)
    }
    fn edit(&mut self, bundle: &Path, f: &mut dyn FnMut(&mut dyn Editor) -> Result<(), Fail>) -> Result<(), Fail> {
        self.edit_local(bundle, f)
    }
    fn app(&mut self, _method: &str, _params: Value) -> Result<Value, Fail> {
        Err(ove_control::CallError::NotRunning.into())
    }
    fn library(&mut self) -> Result<&Library, Fail> {
        Ok(&self.lib)
    }
}

/// An edit sent to the running app, which holds the project open.
struct AppEditor {
    view: Project,
    root: PathBuf,
    source: Source,
}

impl AppEditor {
    fn reload(&mut self) -> Result<(), Fail> {
        self.view = Project::open(self.view.bundle(), Mode::ReadOnly, clock())?;
        Ok(())
    }
}

impl Editor for AppEditor {
    fn project(&self) -> &Project {
        &self.view
    }
    fn apply(&mut self, ops: Vec<Op>, summary: String) -> Result<i64, Fail> {
        let params = json!({
            "bundle": self.view.bundle(),
            "expectRev": self.view.rev(),
            "ops": ops,
            "summary": summary,
            "source": self.source,
        });
        let v = ove_control::call(&self.root, "apply", params, Duration::from_secs(30))?;
        self.reload()?;
        Ok(v.get("rev").and_then(Value::as_i64).unwrap_or_else(|| self.view.rev()))
    }
    fn restore(&mut self, target: i64) -> Result<Option<(i64, String)>, Fail> {
        let params = json!({"bundle": self.view.bundle(), "rev": target, "expectRev": self.view.rev()});
        let v = ove_control::call(&self.root, "restore", params, Duration::from_secs(30))?;
        self.reload()?;
        Ok(v.get("rev")
            .and_then(Value::as_i64)
            .map(|r| (r, v.get("summary").and_then(Value::as_str).unwrap_or_default().to_string())))
    }
    fn set_diagnostics(&mut self, _lines: Vec<String>) -> Result<(), Fail> {
        // The app keeps its own diagnostics.
        Ok(())
    }
    fn via_app(&self) -> bool {
        true
    }
}

/// [`BundleCtx`] plus the control link: edits to a project the running app holds go through the app, and app
/// calls reach it. Used by the command line and `ove mcp`.
pub struct LinkCtx {
    pub inner: BundleCtx,
    /// Prints progress (command line without `--json` prints to stderr, with it JSON lines to stdout).
    pub progress: Option<ProgressStyle>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressStyle {
    Text,
    JsonLines,
}

impl LinkCtx {
    pub fn open(library: Option<&Path>, surface: Surface, source: Source) -> Result<LinkCtx, Fail> {
        Ok(LinkCtx { inner: BundleCtx::open(library, surface, source)?, progress: None })
    }

    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    /// The bundle the running app has open, when it is `bundle`.
    fn app_holds(&self, bundle: &Path) -> bool {
        let v = ove_control::call(&self.inner.root, "context", json!({}), Duration::from_secs(5));
        let open = v.ok().and_then(|v| v.pointer("/project/bundle").and_then(Value::as_str).map(PathBuf::from));
        open.is_some_and(|b| same_bundle(&b, bundle))
    }
}

impl ToolCtx for LinkCtx {
    fn surface(&self) -> Surface {
        self.inner.surface
    }
    fn source(&self) -> Source {
        self.inner.source
    }
    fn bundle(&mut self, project: Option<&str>) -> Result<PathBuf, Fail> {
        self.inner.resolve(project)
    }
    fn read(&mut self, bundle: &Path, f: &mut dyn FnMut(&Project) -> Result<(), Fail>) -> Result<(), Fail> {
        self.inner.read(bundle, f)
    }
    /// Retries (up to 3 times) when the app's project moved on between reading and applying.
    fn edit(&mut self, bundle: &Path, f: &mut dyn FnMut(&mut dyn Editor) -> Result<(), Fail>) -> Result<(), Fail> {
        let mut attempt = 0;
        loop {
            let r = match Project::open(bundle, Mode::Write, clock()) {
                Ok(mut p) => {
                    let r = f(&mut LocalEditor { project: &mut p, source: self.inner.source });
                    if r.is_ok() {
                        p.flush_projection();
                        p.close()?;
                    }
                    r
                }
                Err(e) if e.code() == "project.locked" && self.app_holds(bundle) => {
                    let mut w = AppEditor {
                        view: Project::open(bundle, Mode::ReadOnly, clock())?,
                        root: self.inner.root.clone(),
                        source: self.inner.source,
                    };
                    f(&mut w)
                }
                Err(e) => return Err(e.into()),
            };
            match r {
                Err(e) if e.code == "rev.changed" && attempt < 3 => attempt += 1,
                other => return other,
            }
        }
    }
    fn app(&mut self, method: &str, params: Value) -> Result<Value, Fail> {
        Ok(ove_control::call(&self.inner.root, method, params, Duration::from_secs(20))?)
    }
    fn library(&mut self) -> Result<&Library, Fail> {
        Ok(&self.inner.lib)
    }
    fn progress(&mut self, percent: i64, detail: &Value) {
        use std::io::Write;
        match self.progress {
            Some(ProgressStyle::JsonLines) => println!("{}", json!({"progress": detail})),
            Some(ProgressStyle::Text) => eprint!("\rexporting {percent}%"),
            None => return,
        }
        let _ = std::io::stdout().flush();
    }
    fn progress_done(&mut self) {
        if self.progress == Some(ProgressStyle::Text) {
            eprintln!();
        }
    }
}
