//! Shared plumbing: refusals, the library, the write path, and running registry tools for commands (every command
//! that is also an agent tool runs that tool's handler, so the command line and the tools agree by construction).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ove_core::revision::Source;
use ove_store::Library;
use ove_store::project::{Mode, Project};
use ove_tools::{LinkCtx, ProgressStyle, Surface};
use serde_json::Value;

pub use ove_tools::Fail;
pub use ove_tools::ctx::{Editor, via};
pub use ove_tools::fail::{now_ms, secs};

pub type Out = Result<(Value, String), Fail>;

pub fn clock() -> Arc<dyn ove_core::Clock> {
    Arc::new(ove_core::SystemClock)
}

/// What every command gets: the library and the tool context (which records where edits come from).
pub struct Ctx {
    pub lib: Library,
    pub root: PathBuf,
    pub tools: RefCell<LinkCtx>,
}

impl Ctx {
    pub fn open(library: Option<&Path>, source: Source, json: bool, progress: bool) -> Result<Ctx, Fail> {
        let mut tools = LinkCtx::open(library, Surface::Cli, source)?;
        tools.inner.actor = Some(ove_core::revision::Actor::new(ove_core::revision::ActorKind::Cli));
        if progress {
            tools.progress = Some(if json { ProgressStyle::JsonLines } else { ProgressStyle::Text });
        }
        let root = tools.root().to_path_buf();
        Ok(Ctx { lib: Library::open(&root, clock())?, root, tools: RefCell::new(tools) })
    }

    pub fn find(&self, project: &str) -> Result<PathBuf, Fail> {
        Ok(self.lib.find(project)?)
    }

    pub fn read(&self, project: &str) -> Result<Project, Fail> {
        Ok(Project::open(&self.find(project)?, Mode::ReadOnly, clock())?)
    }

    /// Runs a registry tool with JSON arguments (nulls mean "not given").
    pub fn tool(&self, name: &str, args: Value) -> Out {
        let args = match args {
            Value::Object(m) => m,
            _ => Default::default(),
        };
        let r = ove_tools::call(&mut *self.tools.borrow_mut(), name, &args)?;
        Ok((r.json, r.text))
    }
}

/// Runs an edit on a project, through the running app when it holds the project (retrying when the app's project
/// moved on between reading and applying).
pub fn edit<T>(ctx: &Ctx, project: &str, f: impl FnMut(&mut dyn Editor) -> Result<T, Fail>) -> Result<T, Fail> {
    let bundle = ctx.find(project)?;
    ove_tools::ctx::edit(&mut *ctx.tools.borrow_mut(), &bundle, f)
}
