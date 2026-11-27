//! `ove skills`: the skill store in the library (list, show), and the editing guide written into agents' skill
//! folders (`ove skills install` with no id).

use std::path::{Path, PathBuf};

use clap::Subcommand;
use ove_core::revision::Source;
use ove_tools::skills::{Skill, SkillStore};
use serde_json::json;

use crate::common::{Fail, Out};

#[derive(Subcommand)]
pub enum SkillsCmd {
    /// Every skill in the library's store, builtin ones first, with its source and state (invalid ones with why).
    List,
    /// Print one skill's text.
    Show { id: String },
    /// With no id: write the openvideoeditor editing guide into the Claude Code / Codex skills folders (or --dir).
    Install {
        #[arg(long)]
        dir: Vec<PathBuf>,
    },
}

/// The library folder: `--library`, else `OVE_LIBRARY`, else the platform default.
pub fn library(flag: Option<&Path>) -> Result<PathBuf, Fail> {
    match flag {
        Some(p) => ove_store::library::validate_library(p).map(|_| p.to_path_buf()),
        None => ove_store::library::resolve_library(None),
    }
    .map_err(Fail::from)
}

pub fn store(flag: Option<&Path>) -> Result<SkillStore, Fail> {
    Ok(SkillStore::of_library(&library(flag)?))
}

fn state(s: &Skill) -> String {
    match (&s.invalid, s.modified) {
        (Some(why), _) => format!("invalid: {why}"),
        (None, true) => "modified".into(),
        (None, false) => "ok".into(),
    }
}

pub fn run(cmd: &SkillsCmd, library_flag: Option<&Path>, _source: Source) -> Out {
    match cmd {
        SkillsCmd::Install { dir } => crate::docs::install_skills(dir),
        SkillsCmd::List => {
            let store = store(library_flag)?;
            let all = store.list();
            let mut text = String::new();
            for s in &all {
                text.push_str(&format!("{:<24} {:<10} {}\n", s.id, state(s), s.origin.label()));
                if s.is_valid() {
                    text.push_str(&format!("{:<24} {}\n", "", ove_tools::skills::cut(&s.description, 100)));
                }
            }
            text.push_str(&format!("store: {}", store.root.display()));
            Ok((json!({"root": store.root, "skills": all}), text))
        }
        SkillsCmd::Show { id } => {
            let s = store(library_flag)?.get(id)?;
            let mut v = serde_json::to_value(&s).unwrap_or_default();
            v["text"] = s.text.clone().into();
            Ok((v, s.text.trim_end().to_string()))
        }
    }
}
