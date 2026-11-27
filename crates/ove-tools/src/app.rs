//! [`AppCtx`]: tools over the project open in the desktop app. The app implements [`AppHost`]; [`ProjectHost`] is a
//! host over one project held by the caller (tests, and the assistant's own helpers).

use std::path::{Path, PathBuf};

use ove_core::revision::{Actor, ActorKind, Source};
use ove_store::Library;
use ove_store::project::Project;
use serde_json::Value;

use crate::bundle::same_bundle;
use crate::ctx::{Editor, LocalEditor, Surface, ToolCtx, confirm_required};
use crate::fail::Fail;
use crate::session::Session;

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
    /// The session (the chat) the call belongs to; see [`ToolCtx::session`].
    fn session(&self) -> Option<Session> {
        None
    }
    /// Asks the person to approve a priced request; see [`ToolCtx::ask_spend`].
    fn ask_spend(&mut self, _card: &Value) -> Result<bool, Fail> {
        Err(confirm_required())
    }
    /// Starts the app's generation worker for a saved job; see [`ToolCtx::start_generation`].
    fn start_generation(&mut self, _bundle: &Path, _job: &str) -> Result<(), Fail> {
        Err(Fail::new("generate.unavailable", "generation jobs cannot be started here"))
    }
}

/// Tools over the project open in the app. Edits are revisions of the app's session, recorded with `source`.
pub struct AppCtx<'a> {
    pub host: &'a mut dyn AppHost,
    pub source: Source,
    pub surface: Surface,
    /// Recorded on every revision made here.
    pub actor: Option<Actor>,
}

impl<'a> AppCtx<'a> {
    /// The in-app assistant's context (source `agent`).
    pub fn assistant(host: &'a mut dyn AppHost) -> AppCtx<'a> {
        AppCtx { host, source: Source::Agent, surface: Surface::InApp, actor: None }
    }

    /// The in-app assistant's context in a chat (`session`) and turn.
    pub fn assistant_in(host: &'a mut dyn AppHost, session: &str, turn: &str) -> AppCtx<'a> {
        let actor = Actor {
            kind: ActorKind::Assistant,
            session: Some(session.to_string()),
            turn: Some(turn.to_string()),
            client: None,
        };
        AppCtx { host, source: Source::Agent, surface: Surface::InApp, actor: Some(actor) }
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
    fn actor(&self) -> Option<Actor> {
        self.actor.clone()
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
        let (source, actor) = (self.source, self.actor.clone());
        self.host.with_open(true, &mut |p| f(&mut LocalEditor { project: p, source, actor: actor.clone() }))
    }
    fn app(&mut self, method: &str, params: Value) -> Result<Value, Fail> {
        self.host.app(method, params)
    }
    fn library(&mut self) -> Result<&Library, Fail> {
        self.host.library().ok_or_else(|| Fail::new("library.unavailable", "the project library is not available here"))
    }
    fn session(&self) -> Option<Session> {
        self.host.session()
    }
    fn ask_spend(&mut self, card: &Value) -> Result<bool, Fail> {
        self.host.ask_spend(card)
    }
    fn start_generation(&mut self, bundle: &Path, job: &str) -> Result<(), Fail> {
        self.host.start_generation(bundle, job)
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
