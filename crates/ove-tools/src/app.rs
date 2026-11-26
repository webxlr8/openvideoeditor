//! [`AppCtx`]: tools over the project open in the desktop app. The app implements [`AppHost`]; [`ProjectHost`] is a
//! host over one project held by the caller (tests, and the assistant's own helpers).

use std::path::{Path, PathBuf};

use ove_core::revision::Source;
use ove_store::Library;
use ove_store::project::Project;
use serde_json::Value;

use crate::bundle::same_bundle;
use crate::ctx::{Editor, LocalEditor, Surface, ToolCtx};
use crate::fail::Fail;

/// What the app provides to tools.
pub trait AppHost {
    /// Runs `f` on the open project, holding it for the call. `edit` is true when `f` may commit revisions; the host
    /// then updates the editor afterwards. Refused when no project is open.
    fn with_open(&mut self, edit: bool, f: &mut dyn FnMut(&mut Project) -> Result<(), Fail>) -> Result<(), Fail>;
    /// App calls (`context`, `seek`, `play`, `pause`, `open`).
    fn app(&mut self, method: &str, _params: Value) -> Result<Value, Fail> {
        Err(Fail::new("app.unavailable", format!("`{method}` is not available here")))
    }
    /// The library, when the host has one.
    fn library(&mut self) -> Option<&Library> {
        None
    }
}

/// Tools over the project open in the app. Edits are revisions of the app's session, recorded with `source`.
pub struct AppCtx<'a> {
    pub host: &'a mut dyn AppHost,
    pub source: Source,
    pub surface: Surface,
}

impl<'a> AppCtx<'a> {
    /// The in-app assistant's context (source `agent`).
    pub fn assistant(host: &'a mut dyn AppHost) -> AppCtx<'a> {
        AppCtx { host, source: Source::Agent, surface: Surface::InApp }
    }

    fn open_bundle(&mut self) -> Result<PathBuf, Fail> {
        let mut b = None;
        self.host.with_open(false, &mut |p| {
            b = Some(p.bundle().to_path_buf());
            Ok(())
        })?;
        b.ok_or_else(no_project)
    }

    fn check(&mut self, bundle: &Path) -> Result<(), Fail> {
        let open = self.open_bundle()?;
        if same_bundle(&open, bundle) {
            Ok(())
        } else {
            Err(Fail::new("project.notOpen", "only the project open in the editor can be used here"))
        }
    }
}

fn no_project() -> Fail {
    Fail::new("app.noProject", "No project is open in the editor anymore.")
}

impl ToolCtx for AppCtx<'_> {
    fn surface(&self) -> Surface {
        self.surface
    }
    fn source(&self) -> Source {
        self.source
    }
    fn bundle(&mut self, project: Option<&str>) -> Result<PathBuf, Fail> {
        let mut found = None;
        self.host.with_open(false, &mut |p| {
            let ok = match project {
                None => true,
                Some(q) => {
                    let d = &p.doc().project;
                    q == d.id || q.eq_ignore_ascii_case(&d.name) || same_bundle(Path::new(q), p.bundle())
                }
            };
            found = Some((ok, p.bundle().to_path_buf()));
            Ok(())
        })?;
        match found {
            Some((true, b)) => Ok(b),
            Some((false, _)) => {
                Err(Fail::new("project.notOpen", "only the project open in the editor can be used here"))
            }
            None => Err(no_project()),
        }
    }
    fn read(&mut self, bundle: &Path, f: &mut dyn FnMut(&Project) -> Result<(), Fail>) -> Result<(), Fail> {
        self.check(bundle)?;
        self.host.with_open(false, &mut |p| f(p))
    }
    fn edit(&mut self, bundle: &Path, f: &mut dyn FnMut(&mut dyn Editor) -> Result<(), Fail>) -> Result<(), Fail> {
        self.check(bundle)?;
        let source = self.source;
        self.host.with_open(true, &mut |p| f(&mut LocalEditor { project: p, source }))
    }
    fn app(&mut self, method: &str, params: Value) -> Result<Value, Fail> {
        self.host.app(method, params)
    }
    fn library(&mut self) -> Result<&Library, Fail> {
        self.host.library().ok_or_else(|| Fail::new("library.unavailable", "the project library is not available here"))
    }
}

/// A host over one project the caller holds open (the "app" is the caller).
pub struct ProjectHost<'a> {
    pub project: &'a mut Project,
}

impl AppHost for ProjectHost<'_> {
    fn with_open(&mut self, _edit: bool, f: &mut dyn FnMut(&mut Project) -> Result<(), Fail>) -> Result<(), Fail> {
        f(self.project)
    }
}
