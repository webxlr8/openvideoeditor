//! `ove skills`: the skill store in the library (list, show), the community catalog (catalog, install, update),
//! export to other agents, and the editing guide written into agents' skill folders (`ove skills install` with no
//! id).
//!
//! Installing and updating from a catalog never happens on its own: without `--yes` the command shows what it would
//! install, verified against its pin, and changes nothing. `--yes` typed at a terminal is the person's confirmation.

use std::path::{Path, PathBuf};

use clap::Subcommand;
use ove_core::revision::Source;
use ove_tools::skills::catalog::{self, Http, Settings, UpdateState};
use ove_tools::skills::export;
use ove_tools::skills::{Skill, SkillStore};
use serde_json::{Value, json};

use crate::common::{Fail, Out};

#[derive(Subcommand)]
pub enum SkillsCmd {
    /// Every skill in the library's store, builtin ones first, with its source and state (invalid ones with why).
    List,
    /// Print one skill's text.
    Show { id: String },
    /// The community catalog: its skills and the state of the ones installed from it. --set / --clear change its
    /// address (HTTPS). With no address set, nothing is fetched.
    Catalog {
        #[arg(long, value_name = "URL", conflicts_with = "clear")]
        set: Option<String>,
        #[arg(long)]
        clear: bool,
    },
    /// With an id: install that catalog skill after checking it against its pinned SHA-256 (shows it and changes
    /// nothing without --yes). With no id: write the openvideoeditor editing guide into the Claude Code / Codex skills
    /// folders (or --dir).
    Install {
        id: Option<String>,
        /// Install under this id instead.
        #[arg(long = "as", value_name = "ID")]
        as_id: Option<String>,
        /// Confirm the install.
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        dir: Vec<PathBuf>,
    },
    /// Updates of skills installed from the catalog. With an id: show the differences, and install them with --yes
    /// (a skill you edited is kept and the update is installed as a copy).
    Update {
        id: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    /// Copy a skill into the skills folders of the agents installed here (Claude, Codex, Cursor) as `ove-<id>`.
    Export {
        id: String,
        /// Only these agents (claude, codex, cursor).
        #[arg(long, value_delimiter = ',')]
        to: Vec<String>,
        /// Replace copies that were changed in the agent's folder.
        #[arg(long)]
        yes: bool,
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

fn home() -> Result<PathBuf, Fail> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| Fail::new("home.unknown", "no home folder (HOME / USERPROFILE)"))
}

fn state(s: &Skill) -> String {
    match (&s.invalid, s.modified) {
        (Some(why), _) => format!("invalid: {why}"),
        (None, true) => "modified".into(),
        (None, false) => "ok".into(),
    }
}

fn confirm(what: String, v: Value) -> Fail {
    Fail::new("skill.confirm", what).details(v)
}

pub fn run(cmd: &SkillsCmd, library_flag: Option<&Path>, source: Source) -> Out {
    // `--yes` counts only when typed at a terminal.
    let person = source == Source::Cli;
    match cmd {
        SkillsCmd::Install { id: None, dir, .. } => crate::docs::install_skills(dir),
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
        SkillsCmd::Catalog { set, clear } => {
            let store = store(library_flag)?;
            if set.is_some() || *clear {
                let url = if *clear { String::new() } else { set.clone().unwrap_or_default() };
                store.save_settings(&Settings { catalog_url: url.clone() })?;
                let text =
                    if url.is_empty() { "Catalog address cleared.".to_string() } else { format!("Catalog: {url}") };
                return Ok((json!({"catalogUrl": url}), text));
            }
            let c = catalog::fetch(&store, &Http)?;
            let states = catalog::updates(&store, &c);
            let mut text = format!("{}\n", c.url);
            for e in &c.entries {
                text.push_str(&format!("{:<24} {}\n", e.id, ove_tools::skills::cut(&e.description, 90)));
            }
            for s in &c.skipped {
                text.push_str(&format!("skipped: {s}\n"));
            }
            Ok((json!({"catalog": c, "installed": states}), text.trim_end().to_string()))
        }
        SkillsCmd::Install { id: Some(id), as_id, yes, .. } => {
            let store = store(library_flag)?;
            let c = catalog::fetch(&store, &Http)?;
            if !(*yes && person) {
                let p = catalog::preview(&store, &c, id, &Http)?;
                let mut msg = format!(
                    "Not installed yet. `{}` from {} matches its pinned SHA-256 {}.\nIt would be installed as `{}`. Read it, then run again with --yes to install it.",
                    p.entry.id, p.catalog_url, p.entry.sha256, p.install_as
                );
                if !p.unknown_tools.is_empty() {
                    msg.push_str(&format!("\nIt names tools that do not exist: {}.", p.unknown_tools.join(", ")));
                }
                msg.push_str(&format!("\n\n{}", p.text.trim_end()));
                return Err(confirm(msg, serde_json::to_value(&p).unwrap_or_default()));
            }
            let s = catalog::install(&store, &c, id, as_id.as_deref(), &Http)?;
            Ok((serde_json::to_value(&s).unwrap_or_default(), format!("Installed `{}` from {}.", s.id, c.url)))
        }
        SkillsCmd::Update { id: None, .. } => {
            let store = store(library_flag)?;
            let c = catalog::fetch(&store, &Http)?;
            let states = catalog::updates(&store, &c);
            let text = if states.is_empty() {
                "No skills are installed from this catalog.".to_string()
            } else {
                states
                    .iter()
                    .map(|s| match s {
                        UpdateState::UpToDate { id } => format!("{id:<24} up to date"),
                        UpdateState::Available { id, .. } => format!("{id:<24} update available"),
                        UpdateState::Modified { id, .. } => {
                            format!("{id:<24} update available; you edited it, so it comes as a copy")
                        }
                        UpdateState::EditedLocally { id } => format!("{id:<24} edited here"),
                        UpdateState::Gone { id } => format!("{id:<24} no longer in the catalog (kept)"),
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            Ok((json!({"catalogUrl": c.url, "skills": states}), text))
        }
        SkillsCmd::Update { id: Some(id), yes } => {
            let store = store(library_flag)?;
            let c = catalog::fetch(&store, &Http)?;
            let o = catalog::offer(&store, &c, id, &Http)?;
            if !(*yes && person) {
                let lines: Vec<String> =
                    o.diff.iter().filter(|l| l.op != " ").map(|l| format!("{}{}", l.op, l.text)).collect();
                let target = if o.as_copy {
                    format!("You edited `{id}`, so it stays; the update would be installed as `{}`.", o.install_as)
                } else {
                    format!("It would replace `{id}`.")
                };
                return Err(confirm(
                    format!(
                        "Not updated yet. {target} Changes:\n{}\nRun again with --yes to install it.",
                        lines.join("\n")
                    ),
                    serde_json::to_value(&o).unwrap_or_default(),
                ));
            }
            let s = catalog::accept(&store, &c, id, &o.sha256, &Http)?;
            let text = if o.as_copy {
                format!("Installed the update as `{}`; your edited `{id}` is unchanged.", s.id)
            } else {
                format!("Updated `{id}`.")
            };
            Ok((serde_json::to_value(&s).unwrap_or_default(), text))
        }
        SkillsCmd::Export { id, to, yes } => {
            let store = store(library_flag)?;
            let home = home()?;
            let all = export::targets(&home);
            if let Some(bad) = to.iter().find(|t| !all.iter().any(|x| x.id == **t)) {
                return Err(Fail::new(
                    "export.target",
                    format!(
                        "no agent {bad:?}; choose from {}",
                        all.iter().map(|t| t.id.as_str()).collect::<Vec<_>>().join(", ")
                    ),
                ));
            }
            let chosen: Vec<&export::Target> =
                all.iter().filter(|t| if to.is_empty() { t.available } else { to.contains(&t.id) }).collect();
            if chosen.is_empty() {
                return Err(Fail::new(
                    "export.unavailable",
                    format!(
                        "none of the agents is installed here: {}",
                        all.iter().filter_map(|t| t.reason.clone()).collect::<Vec<_>>().join("; ")
                    ),
                ));
            }
            let mut done = Vec::new();
            let mut lines = Vec::new();
            let mut first_error = None;
            for t in chosen {
                match export::export(&store, id, t, *yes && person) {
                    Ok(e) => {
                        lines.push(format!("{}: {}", t.name, e.file.display()));
                        done.push(e);
                    }
                    Err(f) => {
                        lines.push(format!("{}: not exported: {}", t.name, f.message));
                        first_error.get_or_insert(f);
                    }
                }
            }
            match first_error {
                Some(f) if done.is_empty() => Err(f),
                Some(f) => Err(Fail::new(f.code, lines.join("\n")).details(json!({"exported": done}))),
                None => Ok((json!({"exported": done, "targets": all}), lines.join("\n"))),
            }
        }
    }
}
