//! Settings → Skills (add-agent-skills-and-connectors task 2.5): the library's skill store, the editor, the community
//! catalog with hash-checked installs and visible updates, and export to other agents. Everything here runs on the
//! person's click: the catalog is fetched only when they open or refresh the Community list, and an install or update
//! happens only after they saw it and confirmed.

use std::path::PathBuf;

use ove_tools::skills::catalog::{self, Http, Settings, UpdateState};
use ove_tools::skills::{INDEX_CAP, Origin, Skill, SkillStore, export};
use serde::Serialize;
use specta::Type;
use tauri::AppHandle;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::worker;

/// One skill as the Installed and Local lists show it.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillRow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: Option<String>,
    /// `builtin`, `catalog`, `local`, `drafted` or `assistant`.
    pub kind: String,
    /// Where it came from, as shown (`catalog <url>`, `created by the assistant`, …).
    pub source: String,
    pub builtin: bool,
    /// Why agents are not offered it.
    pub invalid: Option<String>,
    /// Edited since the app installed it.
    pub modified: bool,
    pub path: Option<String>,
    /// Front-matter keys that are ignored.
    pub ignored_keys: Vec<String>,
    /// Tool names in the text that are not agent tools.
    pub unknown_tools: Vec<String>,
}

/// One agent skills can be exported to.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillTarget {
    pub id: String,
    pub name: String,
    pub dir: String,
    pub available: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillsView {
    /// The store folder.
    pub root: String,
    pub skills: Vec<SkillRow>,
    /// The catalog address; empty when none is set.
    pub catalog_url: String,
    /// Valid skills, and the most the assistant's index lists.
    pub valid_count: u32,
    pub index_cap: u32,
    pub targets: Vec<SkillTarget>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntryView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub sha256: String,
}

/// An installed catalog skill and whether an update is offered.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillUpdateRow {
    pub id: String,
    /// `upToDate`, `available`, `modified` (an update comes as a copy), `editedLocally` or `gone`.
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CatalogView {
    pub url: String,
    pub entries: Vec<CatalogEntryView>,
    pub skipped: Vec<String>,
    pub installed: Vec<SkillUpdateRow>,
}

/// What an install would write, verified against its pin, for the person to read and confirm.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallPreview {
    pub id: String,
    pub name: String,
    pub sha256: String,
    pub catalog_url: String,
    pub install_as: String,
    pub text: String,
    pub unknown_tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillDiffLine {
    /// `" "`, `"-"` or `"+"`.
    pub op: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillUpdateOffer {
    pub id: String,
    /// True when the person edited the skill: accepting installs a copy under `install_as`.
    pub as_copy: bool,
    pub install_as: String,
    pub sha256: String,
    pub diff: Vec<SkillDiffLine>,
}

/// The result of exporting to one agent.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillExportOutcome {
    pub target: String,
    pub name: String,
    pub ok: bool,
    pub file: Option<String>,
    /// `export.modified` when the copy there was changed (export again with overwrite to replace it).
    pub code: Option<String>,
    pub message: Option<String>,
}

fn fail(f: ove_tools::Fail) -> AppError {
    let code = match f.code.as_str() {
        "skill.unknown" | "catalog.missing" => ErrorCode::NotFound,
        "skill.io" | "export.io" | "catalog.unreachable" | "catalog.http" => ErrorCode::Io,
        "skill.tampered" => ErrorCode::Corrupt,
        _ => ErrorCode::InvalidArgument,
    };
    AppError::new(code, f.message)
}

fn store(app: &AppHandle) -> AppResult<SkillStore> {
    crate::assistant::skill_store(app).ok_or_else(|| {
        AppError::new(ErrorCode::Io, "The library folder could not be opened, so skills are unavailable.")
    })
}

fn home() -> AppResult<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| AppError::new(ErrorCode::Io, "No home folder was found."))
}

fn kind(o: &Origin) -> &'static str {
    match o {
        Origin::Builtin => "builtin",
        Origin::Catalog { .. } => "catalog",
        Origin::Local => "local",
        Origin::Drafted => "drafted",
        Origin::Assistant => "assistant",
    }
}

pub fn row(s: &Skill) -> SkillRow {
    SkillRow {
        id: s.id.clone(),
        name: s.name.clone(),
        description: s.description.clone(),
        version: s.version.clone(),
        kind: kind(&s.origin).into(),
        source: s.origin.label(),
        builtin: s.builtin,
        invalid: s.invalid.clone(),
        modified: s.modified,
        path: s.path.as_ref().map(|p| p.display().to_string()),
        ignored_keys: s.ignored_keys.clone(),
        unknown_tools: ove_tools::skills::unknown_tool_names(&s.text),
    }
}

fn targets() -> Vec<SkillTarget> {
    let Ok(h) = home() else { return Vec::new() };
    export::targets(&h)
        .into_iter()
        .map(|t| SkillTarget {
            id: t.id,
            name: t.name,
            dir: t.dir.display().to_string(),
            available: t.available,
            reason: t.reason,
        })
        .collect()
}

pub fn view(store: &SkillStore) -> SkillsView {
    let all = store.list();
    SkillsView {
        root: store.root.display().to_string(),
        valid_count: all.iter().filter(|s| s.is_valid()).count() as u32,
        skills: all.iter().map(row).collect(),
        catalog_url: store.catalog_url().unwrap_or_default(),
        index_cap: INDEX_CAP as u32,
        targets: targets(),
    }
}

/// Every skill in the store, with the catalog address and the export targets.
#[tauri::command]
#[specta::specta]
pub async fn skills_view(app: AppHandle) -> AppResult<SkillsView> {
    worker::blocking("skills_view", move || Ok(view(&store(&app)?))).await
}

/// A skill's whole file (also for invalid skills, so the person can fix them).
#[tauri::command]
#[specta::specta]
pub async fn skill_text(app: AppHandle, id: String) -> AppResult<String> {
    worker::blocking("skill_text", move || {
        let store = store(&app)?;
        store
            .list()
            .into_iter()
            .find(|s| s.id == id)
            .map(|s| s.text)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, format!("There is no skill {id}.")))
    })
    .await
}

/// Saves the editor: a new skill (no id: one is made from its name) or the person's edit of one.
#[tauri::command]
#[specta::specta]
pub async fn skill_save(app: AppHandle, id: Option<String>, text: String, drafted: bool) -> AppResult<SkillRow> {
    worker::blocking("skill_save", move || {
        let store = store(&app)?;
        let id = match id.filter(|i| !i.is_empty()) {
            Some(i) => i,
            None => {
                let fm = ove_tools::skills::validate_text(&text)
                    .map_err(|why| AppError::new(ErrorCode::InvalidArgument, format!("Not saved: {why}.")))?;
                store.free_id(&ove_tools::skills::id_from_name(fm.name.as_deref().unwrap_or("skill")))
            }
        };
        let origin = if drafted { Origin::Drafted } else { Origin::Local };
        store.save_edit(&id, &text, origin).map(|s| row(&s)).map_err(fail)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn skill_remove(app: AppHandle, id: String) -> AppResult<()> {
    worker::blocking("skill_remove", move || store(&app)?.remove(&id).map_err(fail)).await
}

/// Sets the catalog address (HTTPS); empty clears it. Fetches nothing.
#[tauri::command]
#[specta::specta]
pub async fn skills_set_catalog(app: AppHandle, url: String) -> AppResult<()> {
    worker::blocking("skills_set_catalog", move || {
        store(&app)?.save_settings(&Settings { catalog_url: url.trim().to_string() }).map_err(fail)
    })
    .await
}

fn state_name(s: &UpdateState) -> (String, &'static str) {
    match s {
        UpdateState::UpToDate { id } => (id.clone(), "upToDate"),
        UpdateState::Available { id, .. } => (id.clone(), "available"),
        UpdateState::Modified { id, .. } => (id.clone(), "modified"),
        UpdateState::EditedLocally { id } => (id.clone(), "editedLocally"),
        UpdateState::Gone { id } => (id.clone(), "gone"),
    }
}

/// Fetches the catalog (the person opened or refreshed the Community list). Refused, with no request, when no
/// address is set.
#[tauri::command]
#[specta::specta]
pub async fn skills_catalog(app: AppHandle) -> AppResult<CatalogView> {
    worker::blocking("skills_catalog", move || {
        let store = store(&app)?;
        let c = catalog::fetch(&store, &Http).map_err(fail)?;
        let installed = catalog::updates(&store, &c)
            .iter()
            .map(|s| {
                let (id, state) = state_name(s);
                SkillUpdateRow { id, state: state.into() }
            })
            .collect();
        Ok(CatalogView {
            entries: c
                .entries
                .iter()
                .map(|e| CatalogEntryView {
                    id: e.id.clone(),
                    name: e.name.clone(),
                    description: e.description.clone(),
                    sha256: e.sha256.clone(),
                })
                .collect(),
            url: c.url,
            skipped: c.skipped,
            installed,
        })
    })
    .await
}

/// Fetches and verifies a catalog skill for the person to read before installing. Installs nothing.
#[tauri::command]
#[specta::specta]
pub async fn skill_install_preview(app: AppHandle, id: String) -> AppResult<SkillInstallPreview> {
    worker::blocking("skill_install_preview", move || {
        let store = store(&app)?;
        let c = catalog::fetch(&store, &Http).map_err(fail)?;
        let p = catalog::preview(&store, &c, &id, &Http).map_err(fail)?;
        Ok(SkillInstallPreview {
            id: p.entry.id,
            name: p.entry.name,
            sha256: p.entry.sha256,
            catalog_url: p.catalog_url,
            install_as: p.install_as,
            text: p.text,
            unknown_tools: p.unknown_tools,
        })
    })
    .await
}

/// Installs a catalog skill the person confirmed, checking it against its pin again.
#[tauri::command]
#[specta::specta]
pub async fn skill_install(app: AppHandle, id: String, install_as: String) -> AppResult<SkillRow> {
    worker::blocking("skill_install", move || {
        let store = store(&app)?;
        let c = catalog::fetch(&store, &Http).map_err(fail)?;
        let as_id = Some(install_as.as_str()).filter(|s| !s.is_empty());
        catalog::install(&store, &c, &id, as_id, &Http).map(|s| row(&s)).map_err(fail)
    })
    .await
}

/// The differences an update would make. Installs nothing.
#[tauri::command]
#[specta::specta]
pub async fn skill_update_offer(app: AppHandle, id: String) -> AppResult<SkillUpdateOffer> {
    worker::blocking("skill_update_offer", move || {
        let store = store(&app)?;
        let c = catalog::fetch(&store, &Http).map_err(fail)?;
        let o = catalog::offer(&store, &c, &id, &Http).map_err(fail)?;
        Ok(SkillUpdateOffer {
            id: o.id,
            as_copy: o.as_copy,
            install_as: o.install_as,
            sha256: o.sha256,
            diff: o.diff.into_iter().map(|l| SkillDiffLine { op: l.op, text: l.text }).collect(),
        })
    })
    .await
}

/// Installs an update the person accepted; `sha256` is the one they saw.
#[tauri::command]
#[specta::specta]
pub async fn skill_update_accept(app: AppHandle, id: String, sha256: String) -> AppResult<SkillRow> {
    worker::blocking("skill_update_accept", move || {
        let store = store(&app)?;
        let c = catalog::fetch(&store, &Http).map_err(fail)?;
        catalog::accept(&store, &c, &id, &sha256, &Http).map(|s| row(&s)).map_err(fail)
    })
    .await
}

/// Drafts a skill from the open project with the assistant (the person clicked Draft). Returns the draft for the
/// editor; nothing is saved until the person saves it. `with_chat` includes the project's latest chat.
#[tauri::command]
#[specta::specta]
pub async fn skill_draft(app: AppHandle, with_chat: bool) -> AppResult<String> {
    use tauri::Manager;
    worker::blocking("skill_draft", move || {
        let state = app.state::<crate::AppState>();
        let (key, model) = state.assistant.key_and_model();
        let key = key.ok_or_else(|| {
            AppError::new(
                ErrorCode::InvalidArgument,
                "Drafting needs the assistant: add an Anthropic API key in Settings → Assistant. You can also save the context pack with `ove skills draft --pack-out`.",
            )
        })?;
        // The project is held only while the pack is built, not during the request.
        let pack = {
            let g = crate::projects::lock(&state.projects.open)?;
            let p = g.as_ref().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "Open the finished project first."))?;
            let chat = with_chat
                .then(|| {
                    let chats = ove_agent::ChatStore::for_bundle(p.bundle());
                    chats.list().first().map(|c| {
                        ove_agent::chat::transcript(&chats.records(&c.id), ove_tools::skills::draft::MAX_CHAT_CHARS)
                    })
                })
                .flatten();
            ove_tools::skills::draft::pack(p, chat).map_err(fail)?
        };
        let client = ove_agent::Client::new(key);
        ove_agent::draft::draft_skill(&client, &model, &pack, &std::sync::atomic::AtomicBool::new(false))
            .map(|d| d.text)
            .map_err(|e| AppError::new(ErrorCode::InvalidArgument, e.message))
    })
    .await
}

/// Exports a skill to the chosen agents. `overwrite`: the person confirmed replacing copies they changed there.
#[tauri::command]
#[specta::specta]
pub async fn skill_export(
    app: AppHandle,
    id: String,
    targets: Vec<String>,
    overwrite: bool,
) -> AppResult<Vec<SkillExportOutcome>> {
    worker::blocking("skill_export", move || {
        let store = store(&app)?;
        let all = export::targets(&home()?);
        Ok(all
            .iter()
            .filter(|t| targets.contains(&t.id))
            .map(|t| match export::export(&store, &id, t, overwrite) {
                Ok(e) => SkillExportOutcome {
                    target: t.id.clone(),
                    name: t.name.clone(),
                    ok: true,
                    file: Some(e.file.display().to_string()),
                    code: None,
                    message: None,
                },
                Err(f) => SkillExportOutcome {
                    target: t.id.clone(),
                    name: t.name.clone(),
                    ok: false,
                    file: None,
                    code: Some(f.code),
                    message: Some(f.message),
                },
            })
            .collect())
    })
    .await
}
