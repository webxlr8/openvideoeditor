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
    /// Draft a skill from a finished project with the configured assistant: from the final timeline, the edit
    /// history by actor, the presets in use and, with --with-chat, the latest chat. Saves nothing to the store:
    /// review the draft, then `ove skills add <file>`.
    Draft {
        #[arg(long = "from-project", value_name = "PROJECT")]
        from_project: String,
        /// Include the project's latest chat with the assistant.
        #[arg(long)]
        with_chat: bool,
        /// Write the draft here (default: print it).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Also save the context pack here (works without an assistant).
        #[arg(long)]
        pack_out: Option<PathBuf>,
        /// The model to draft with (default: the assistant's default model).
        #[arg(long)]
        model: Option<String>,
    },
    /// Save a skill file you reviewed (a draft, or one you wrote) into the store.
    Add {
        file: PathBuf,
        /// The id (default: from its name).
        #[arg(long)]
        id: Option<String>,
    },
}

/// Drafts a skill from `pack` with `client` (none: no assistant is configured), writing the pack to `pack_out` and
/// the draft to `out` or the output. Nothing is saved to the store.
pub fn draft(
    pack: &ove_tools::skills::draft::Pack,
    out: Option<&Path>,
    pack_out: Option<&Path>,
    client: Option<ove_agent::Client>,
    model: &str,
) -> Out {
    let write = |p: &Path, text: &str| {
        std::fs::write(p, text).map_err(|e| Fail::new("file.unwritable", format!("{}: {e}", p.display())))
    };
    if let Some(p) = pack_out {
        write(p, &ove_tools::skills::draft::pack_text(pack))?;
    }
    let Some(client) = client else {
        let saved = match pack_out {
            Some(p) => format!("The context pack was saved to {}.", p.display()),
            None => "You can still save the context pack with --pack-out <file>.".into(),
        };
        return Err(Fail::new(
            "draft.noAssistant",
            format!(
                "Drafting needs the assistant: add an Anthropic API key in Settings → Assistant (or set ANTHROPIC_API_KEY). {saved}"
            ),
        )
        .details(json!({"packSaved": pack_out})));
    };
    let d = ove_agent::draft::draft_skill(&client, model, pack, &std::sync::atomic::AtomicBool::new(false))
        .map_err(|e| Fail::new(e.code, e.message))?;
    let review = "Nothing was saved. Review the draft, then save it with `ove skills add <file>`.";
    let v = json!({"draft": d.text, "out": out, "packOut": pack_out, "costUsd": d.cost_usd});
    match out {
        Some(p) => {
            write(p, &d.text)?;
            Ok((v, format!("Wrote the draft to {}. {review}", p.display())))
        }
        None => Ok((v, format!("{}\n\n{review}", d.text.trim_end()))),
    }
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
        SkillsCmd::Draft { from_project, with_chat, out, pack_out, model } => {
            let lib = ove_store::Library::open(library(library_flag)?, crate::common::clock())?;
            let bundle = lib.find(from_project)?;
            let p =
                ove_store::project::Project::open(&bundle, ove_store::project::Mode::ReadOnly, crate::common::clock())?;
            let chat = with_chat
                .then(|| {
                    let chats = ove_agent::ChatStore::for_bundle(&bundle);
                    chats.list().first().map(|c| {
                        ove_agent::chat::transcript(&chats.records(&c.id), ove_tools::skills::draft::MAX_CHAT_CHARS)
                    })
                })
                .flatten();
            let pack = ove_tools::skills::draft::pack(&p, chat)?;
            use ove_agent::KeyStore;
            let client = ove_agent::OsKeyStore.get().map(|(key, _)| ove_agent::Client::new(key));
            let model = model.clone().unwrap_or_else(|| ove_agent::MODELS[0].id.to_string());
            draft(&pack, out.as_deref(), pack_out.as_deref(), client, &model)
        }
        SkillsCmd::Add { file, id } => {
            let store = store(library_flag)?;
            let text = std::fs::read_to_string(file)
                .map_err(|e| Fail::new("file.unreadable", format!("{}: {e}", file.display())))?;
            let fm = ove_tools::skills::validate_text(&text)
                .map_err(|why| Fail::new("skill.invalid", format!("not saved: {why}")))?;
            let id = match id {
                Some(i) => i.clone(),
                None => store.free_id(&ove_tools::skills::id_from_name(fm.name.as_deref().unwrap_or("skill"))),
            };
            if std::fs::symlink_metadata(store.dir(&id)).is_ok() {
                return Err(Fail::new("skill.exists", format!("a skill `{id}` already exists; pass another --id")));
            }
            let origin = if fm.source.as_deref() == Some("drafted") {
                ove_tools::skills::Origin::Drafted
            } else {
                ove_tools::skills::Origin::Local
            };
            let s = store.write(&id, &text, origin, None)?;
            Ok((
                serde_json::to_value(&s).unwrap_or_default(),
                format!("Saved skill `{}` ({}).", s.id, s.origin.label()),
            ))
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

#[cfg(test)]
mod tests;
