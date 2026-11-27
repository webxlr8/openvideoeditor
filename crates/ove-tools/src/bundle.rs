//! Projects on disk: [`BundleCtx`] works on bundles headless (it takes each project's write lock), and [`LinkCtx`]
//! does the same but, when the running app holds a project open, sends the edit to the app over the control link
//! so it becomes a revision of the app's session and the editor updates at once.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ove_core::ops::Op;
use ove_core::revision::{Actor, Source};
use ove_store::Library;
use ove_store::project::{Mode, Project};
use serde_json::{Value, json};

use crate::ctx::{Editor, LocalEditor, Surface, ToolCtx, confirm_required};
use crate::fail::Fail;
use crate::session::Session;

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
    /// Recorded on every revision made here.
    pub actor: Option<Actor>,
    /// Quotes, spend and recent calls of this session.
    pub session: Session,
    /// Asks the person to approve a spend when the app cannot (the MCP client's confirmation prompt).
    pub confirm: Option<Confirm>,
    /// Starts the generation worker for a saved job.
    pub runner: Option<Runner>,
    /// When set, a project opened for an edit stays open (with its write lock) between calls, and is closed once no
    /// call used it for this long ([`BundleCtx::release_if_idle`]). `ove mcp` sets it; one-shot commands do not.
    pub hold: Option<Duration>,
    held: Option<Held>,
    /// Project names and ids this session resolved, used only for the held project (it cannot change underneath
    /// while held) so calls in a burst skip the library scan. Forgotten when the project is released.
    resolved: Vec<(String, PathBuf)>,
}

/// How long a writer waits for another `ove` process (an MCP session keeping the project open between its calls)
/// to let go of a project; such a session releases it after [`BundleCtx::hold`] without calls.
///
/// This must be at least the default hold of `ove mcp` (`HOLD` in `ove-cli/src/mcp.rs`, 2 s) plus 5 s of margin:
/// the holder only notices it is idle when its read of the next call times out, and on a loaded machine that
/// wake-up and the close can be late. With a shorter wait a second agent gets `project.locked` although the first
/// one is about to let go. A test in `ove-cli` (which sees both constants) checks the relation.
pub const WAIT_FOR_SESSION: Duration = Duration::from_secs(7);

/// Opens `bundle` for writing. When another `ove` process holds it (a session between calls), waits up to
/// [`WAIT_FOR_SESSION`] for it to be released; any other holder (the app) is reported at once.
pub fn open_write_waiting(bundle: &Path) -> Result<Project, Fail> {
    let t0 = Instant::now();
    loop {
        match Project::open(bundle, Mode::Write, clock()) {
            Ok(p) => return Ok(p),
            Err(e) if e.code() == "project.locked" && held_by_ove(bundle) && t0.elapsed() < WAIT_FOR_SESSION => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// True when the project's lock holder is another `ove` command-line process.
fn held_by_ove(bundle: &Path) -> bool {
    let holder: Option<ove_store::lock::Holder> =
        std::fs::read(bundle.join(".lock.holder")).ok().and_then(|b| serde_json::from_slice(&b).ok());
    holder.is_some_and(|h| h.process == "ove" && h.pid != std::process::id())
}

/// A project the session keeps open for writing between calls.
struct Held {
    bundle: PathBuf,
    project: Project,
    last: Instant,
}

impl Drop for BundleCtx {
    fn drop(&mut self) {
        if let Err(e) = self.release() {
            tracing::warn!("closing the held project failed: {}", e.message);
        }
    }
}

/// Asks a person to approve a priced request (see [`ToolCtx::ask_spend`]).
pub type Confirm = Box<dyn FnMut(&Value) -> Result<bool, Fail>>;
/// Starts the generation worker for job `id` of `bundle` (see [`ToolCtx::start_generation`]).
pub type Runner = Arc<dyn Fn(&Path, &str) -> Result<(), Fail> + Send + Sync>;

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
        Ok(BundleCtx {
            lib: Library::open(&root, clock())?,
            root,
            surface,
            source,
            binding: Binding::default(),
            actor: None,
            session: Session::new(),
            confirm: None,
            runner: None,
            hold: None,
            held: None,
            resolved: Vec::new(),
        })
    }

    /// The bundle this session holds open, if any.
    pub fn held_bundle(&self) -> Option<&Path> {
        self.held.as_ref().map(|h| h.bundle.as_path())
    }

    /// Closes the held project (its projection and snapshot are written and its lock released).
    pub fn release(&mut self) -> Result<(), Fail> {
        match self.held.take() {
            Some(h) => {
                self.resolved.clear();
                h.project.flush_projection();
                h.project.close()?;
                Ok(())
            }
            None => Ok(()),
        }
    }

    /// Releases the held project when no call used it for the hold time. Returns true when it was released.
    pub fn release_if_idle(&mut self) -> Result<bool, Fail> {
        let idle = match (&self.held, self.hold) {
            (Some(h), Some(hold)) => h.last.elapsed() >= hold,
            _ => false,
        };
        if idle {
            self.release()?;
        }
        Ok(idle)
    }

    /// The time until the held project goes idle (`None` when nothing is held).
    pub fn idle_in(&self) -> Option<Duration> {
        let (h, hold) = (self.held.as_ref()?, self.hold?);
        Some(hold.saturating_sub(h.last.elapsed()))
    }

    fn holds(&self, bundle: &Path) -> bool {
        self.held.as_ref().is_some_and(|h| h.bundle == bundle || same_bundle(&h.bundle, bundle))
    }

    /// The project open for writing: the held one when it is `bundle`, else a newly opened one (another held
    /// project is closed first).
    fn take_writable(&mut self, bundle: &Path) -> Result<Project, Fail> {
        if self.holds(bundle)
            && let Some(h) = self.held.take()
        {
            return Ok(h.project);
        }
        self.release()?;
        open_write_waiting(bundle)
    }

    /// After a call: keeps the project open when the session holds projects, else writes it and closes it.
    fn put_back(&mut self, bundle: &Path, p: Project, ok: bool) -> Result<(), Fail> {
        if self.hold.is_some() {
            self.held = Some(Held { bundle: bundle.to_path_buf(), project: p, last: Instant::now() });
        } else if ok {
            p.flush_projection();
            p.close()?;
        }
        Ok(())
    }

    /// The bundle of a project by name, id or path.
    pub fn find(&self, project: &str) -> Result<PathBuf, Fail> {
        Ok(self.lib.find(project)?)
    }

    /// Opens a project read-only.
    pub fn open_read(&self, bundle: &Path) -> Result<Project, Fail> {
        Ok(Project::open(bundle, Mode::ReadOnly, clock())?)
    }

    fn resolve(&mut self, project: Option<&str>) -> Result<PathBuf, Fail> {
        match (project, &self.binding.project) {
            (None, Some(b)) => Ok(b.clone()),
            (None, None) => Err(Fail::new(
                "argument.missing",
                "this call needs `project` (a project name, id or bundle path; `use_project` sets a default)",
            )),
            (Some(q), bound) => {
                let memo = self
                    .held
                    .as_ref()
                    .and_then(|h| self.resolved.iter().find(|(k, b)| k == q && *b == h.bundle).map(|(_, b)| b.clone()));
                let b = match memo {
                    Some(b) => b,
                    None => {
                        let b = self.find(q)?;
                        if self.hold.is_some() && !self.resolved.iter().any(|(k, _)| k == q) {
                            self.resolved.push((q.to_string(), b.clone()));
                        }
                        b
                    }
                };
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
        let mut p = self.take_writable(bundle)?;
        let r = f(&mut LocalEditor { project: &mut p, source: self.source, actor: self.actor.clone() });
        self.put_back(bundle, p, r.is_ok())?;
        r
    }

    /// Reads the held project when it is `bundle`, else opens it read-only.
    fn read_local(&mut self, bundle: &Path, f: &mut dyn FnMut(&Project) -> Result<(), Fail>) -> Result<(), Fail> {
        if self.holds(bundle)
            && let Some(h) = self.held.as_mut()
        {
            h.last = Instant::now();
            return f(&h.project);
        }
        f(&self.open_read(bundle)?)
    }
}

impl ToolCtx for BundleCtx {
    fn surface(&self) -> Surface {
        self.surface
    }
    fn source(&self) -> Source {
        self.source
    }
    fn actor(&self) -> Option<Actor> {
        self.actor.clone()
    }
    fn set_client(&mut self, name: &str) {
        if let Some(a) = &mut self.actor {
            a.client = Some(name.to_string());
        }
    }
    fn bundle(&mut self, project: Option<&str>) -> Result<PathBuf, Fail> {
        self.resolve(project)
    }
    fn use_project(&mut self, bundle: &Path) -> Result<(), Fail> {
        self.binding.project = Some(bundle.to_path_buf());
        Ok(())
    }
    fn read(&mut self, bundle: &Path, f: &mut dyn FnMut(&Project) -> Result<(), Fail>) -> Result<(), Fail> {
        self.read_local(bundle, f)
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
    fn session(&self) -> Option<Session> {
        Some(self.session.clone())
    }
    fn ask_spend(&mut self, card: &Value) -> Result<bool, Fail> {
        if self.confirm.is_none() {
            return Err(confirm_required());
        }
        // The person may take minutes: the project is not kept locked meanwhile.
        self.release()?;
        match &mut self.confirm {
            Some(ask) => ask(card),
            None => Err(confirm_required()),
        }
    }
    fn start_generation(&mut self, bundle: &Path, job: &str) -> Result<(), Fail> {
        // The worker, another process, writes the project when the job finishes.
        self.release()?;
        match &self.runner {
            Some(run) => run(bundle, job),
            None => Err(Fail::new("generate.unavailable", "generation jobs cannot be started here")),
        }
    }
}

/// An edit sent to the running app, which holds the project open.
struct AppEditor {
    view: Project,
    root: PathBuf,
    source: Source,
    actor: Option<Actor>,
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
        let actor = self.actor.clone();
        self.apply_by(ops, summary, actor)
    }
    fn apply_by(&mut self, ops: Vec<Op>, summary: String, actor: Option<Actor>) -> Result<i64, Fail> {
        let params = json!({
            "bundle": self.view.bundle(),
            "expectRev": self.view.rev(),
            "ops": ops,
            "summary": summary,
            "source": self.source,
            "actor": actor.or_else(|| self.actor.clone()),
        });
        let v = ove_control::call(&self.root, "apply", params, Duration::from_secs(30))?;
        self.reload()?;
        Ok(v.get("rev").and_then(Value::as_i64).unwrap_or_else(|| self.view.rev()))
    }
    fn undo_selected(&mut self, revs: &[i64], summary: String) -> Result<i64, Fail> {
        let params = json!({"bundle": self.view.bundle(), "revs": revs, "summary": summary, "actor": self.actor});
        let v = ove_control::call(&self.root, "undo", params, Duration::from_secs(30))?;
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
    fn actor(&self) -> Option<Actor> {
        self.inner.actor.clone()
    }
    fn set_client(&mut self, name: &str) {
        self.inner.set_client(name);
    }
    fn bundle(&mut self, project: Option<&str>) -> Result<PathBuf, Fail> {
        self.inner.resolve(project)
    }
    fn use_project(&mut self, bundle: &Path) -> Result<(), Fail> {
        self.inner.use_project(bundle)
    }
    fn read(&mut self, bundle: &Path, f: &mut dyn FnMut(&Project) -> Result<(), Fail>) -> Result<(), Fail> {
        self.inner.read(bundle, f)
    }
    /// Retries (up to 3 times) when the app's project moved on between reading and applying.
    fn edit(&mut self, bundle: &Path, f: &mut dyn FnMut(&mut dyn Editor) -> Result<(), Fail>) -> Result<(), Fail> {
        let mut attempt = 0;
        loop {
            let r = match self.inner.take_writable(bundle) {
                Ok(mut p) => {
                    let r = f(&mut LocalEditor {
                        project: &mut p,
                        source: self.inner.source,
                        actor: self.inner.actor.clone(),
                    });
                    self.inner.put_back(bundle, p, r.is_ok())?;
                    r
                }
                Err(e) if e.code == "project.locked" && self.app_holds(bundle) => {
                    let mut w = AppEditor {
                        view: Project::open(bundle, Mode::ReadOnly, clock())?,
                        root: self.inner.root.clone(),
                        source: self.inner.source,
                        actor: self.inner.actor.clone(),
                    };
                    f(&mut w)
                }
                Err(e) => return Err(e),
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
    fn session(&self) -> Option<Session> {
        self.inner.session()
    }
    /// The running app's price card first; without the app, the MCP client's prompt (when it has one).
    fn ask_spend(&mut self, card: &Value) -> Result<bool, Fail> {
        let left = card.get("expiresAt").and_then(Value::as_i64).map(|t| t - crate::fail::now_ms()).unwrap_or(0);
        let wait = Duration::from_millis(left.clamp(1_000, crate::session::QUOTE_TTL_MS) as u64 + 5_000);
        // The person may take minutes: the project is not kept locked meanwhile.
        self.inner.release()?;
        match ove_control::call(&self.inner.root, "confirm_spend", card.clone(), wait) {
            Ok(v) => {
                // The answer must be for this quote.
                if v.get("quoteId") != card.get("quoteId") {
                    return Err(Fail::new("confirm.invalid", "the app answered for another quote; nothing was spent"));
                }
                Ok(v.get("approved").and_then(Value::as_bool) == Some(true))
            }
            // No app, or one too old to show price cards.
            Err(ove_control::CallError::NotRunning) => self.inner.ask_spend(card),
            Err(ove_control::CallError::Refused(r)) if r.code == "method.unknown" => self.inner.ask_spend(card),
            Err(e) => Err(e.into()),
        }
    }
    fn start_generation(&mut self, bundle: &Path, job: &str) -> Result<(), Fail> {
        self.inner.start_generation(bundle, job)
    }
}
