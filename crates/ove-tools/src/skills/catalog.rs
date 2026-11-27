//! The community catalog (design D4): a JSON document at an address the person sets, listing skills pinned by the
//! full SHA-256 of their file. Installing fetches the file and compares its hash with the pin; a file that does not
//! match is refused and nothing is written. Updates are offered, never installed silently, and a skill the person
//! edited is never overwritten: its update comes as a separate copy.
//!
//! Nothing here runs on its own. With no address set, no request is made. Catalog text is untrusted: it is checked
//! like any other skill file, its front matter cannot set its own source, and it cannot change which tools an agent
//! may use.

use std::io::Read;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Ledger, MAX_SKILL_BYTES, Origin, SkillStore, sha256_hex, valid_id, validate_text};
use crate::fail::Fail;

/// Largest catalog document read, in bytes.
pub const MAX_CATALOG_BYTES: usize = 1024 * 1024;
/// Most entries read from one catalog.
pub const MAX_ENTRIES: usize = 500;
/// The settings file in the store folder.
pub const SETTINGS_FILE: &str = "settings.json";
/// Overrides the catalog address set in the store (for tests and managed installs).
pub const CATALOG_ENV: &str = "OVE_SKILLS_CATALOG";

/// One catalog entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub description: String,
    /// SHA-256 (hex) of the skill file.
    pub sha256: String,
    /// Where the skill file is, relative to the catalog document.
    pub path: String,
}

/// A fetched, checked catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub url: String,
    pub entries: Vec<Entry>,
    /// Entries left out, with why (a bad id, hash or path).
    pub skipped: Vec<String>,
}

/// The store's settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// The catalog address; empty: none.
    #[serde(default)]
    pub catalog_url: String,
}

impl SkillStore {
    pub fn settings(&self) -> Settings {
        std::fs::read(self.root.join(SETTINGS_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save_settings(&self, s: &Settings) -> Result<(), Fail> {
        if !s.catalog_url.is_empty() {
            check_url(&s.catalog_url)?;
        }
        std::fs::create_dir_all(&self.root)
            .map_err(|e| Fail::new("skill.io", format!("could not create {}: {e}", self.root.display())))?;
        let path = self.root.join(SETTINGS_FILE);
        let bytes = serde_json::to_vec_pretty(s).map_err(|e| Fail::new("internal", e.to_string()))?;
        ove_store::fsio::write_derived(&path, &bytes)
            .map_err(|e| Fail::new("skill.io", format!("could not write {}: {e}", path.display())))
    }

    /// The catalog address in use: `OVE_SKILLS_CATALOG`, else the store's setting. `None` when neither is set.
    pub fn catalog_url(&self) -> Option<String> {
        std::env::var(CATALOG_ENV)
            .ok()
            .filter(|u| !u.trim().is_empty())
            .or_else(|| Some(self.settings().catalog_url).filter(|u| !u.trim().is_empty()))
            .map(|u| u.trim().to_string())
    }
}

/// Fetches bytes from an address. The real one is [`Http`]; tests count its calls.
pub trait Fetch {
    fn get(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>, Fail>;
}

/// HTTPS through the platform's certificate checks (plain HTTP only to this computer, for tests).
pub struct Http;

impl Fetch for Http {
    fn get(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>, Fail> {
        check_url(url)?;
        let agent = ove_guard::http::agent_builder()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(30))
            .redirects(3)
            .build();
        let resp = agent.get(url).call().map_err(|e| match e {
            ureq::Error::Status(code, _) => Fail::new("catalog.http", format!("{url} answered HTTP {code}")),
            other => Fail::new("catalog.unreachable", format!("could not reach {url}: {other}")),
        })?;
        // A redirect must not leave HTTPS.
        check_url(resp.get_url())?;
        let mut bytes = Vec::new();
        resp.into_reader()
            .take(max_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| Fail::new("catalog.unreachable", format!("reading {url} failed: {e}")))?;
        if bytes.len() > max_bytes {
            return Err(Fail::new("catalog.tooLarge", format!("{url} is larger than {} KB", max_bytes / 1024)));
        }
        Ok(bytes)
    }
}

fn loopback(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

/// Accepts `https://` addresses, and `http://` only to this computer.
pub fn check_url(url: &str) -> Result<(), Fail> {
    let bad = || Fail::new("catalog.url", format!("{url:?} is not an HTTPS address"));
    if let Some(rest) = url.strip_prefix("https://") {
        return if rest.is_empty() || rest.starts_with('/') { Err(bad()) } else { Ok(()) };
    }
    if let Some(rest) = url.strip_prefix("http://") {
        let host = rest.split(['/', '?', '#']).next().unwrap_or("");
        let host = host
            .rsplit_once(':')
            .map(|(h, port)| if port.chars().all(|c| c.is_ascii_digit()) { h } else { host })
            .unwrap_or(host);
        if loopback(host) {
            return Ok(());
        }
    }
    Err(bad())
}

fn valid_sha(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// True for a relative path inside the catalog's folder (no scheme, no leading `/`, no `..`, no `\`).
fn valid_path(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && !p.contains("://")
        && !p.contains('\\')
        && !p.contains('?')
        && !p.contains('#')
        && p.split('/').all(|seg| !seg.is_empty() && seg != ".." && seg != ".")
}

/// The address of an entry's file: its path resolved against the catalog document's folder.
pub fn entry_url(catalog_url: &str, path: &str) -> String {
    let base = catalog_url.split(['?', '#']).next().unwrap_or(catalog_url);
    let folder = match base.rfind('/') {
        Some(i) if i + 1 > base.find("://").map_or(0, |s| s + 3) => &base[..=i],
        _ => return format!("{base}/{path}"),
    };
    format!("{folder}{path}")
}

/// Parses a catalog document: `{"skills": [{id, name, description, sha256, path}]}`. Bad entries are skipped with
/// the reason; duplicate ids keep the first.
pub fn parse(url: &str, bytes: &[u8]) -> Result<Catalog, Fail> {
    let v: Value = serde_json::from_slice(bytes)
        .map_err(|e| Fail::new("catalog.invalid", format!("the catalog at {url} is not JSON: {e}")))?;
    let list = v["skills"]
        .as_array()
        .ok_or_else(|| Fail::new("catalog.invalid", format!("the catalog at {url} has no `skills` list")))?;
    let mut entries: Vec<Entry> = Vec::new();
    let mut skipped = Vec::new();
    for raw in list.iter().take(MAX_ENTRIES) {
        let e: Entry = match serde_json::from_value(raw.clone()) {
            Ok(e) => e,
            Err(err) => {
                skipped.push(format!("an entry is incomplete: {err}"));
                continue;
            }
        };
        let why = if !valid_id(&e.id) {
            Some("its id is not usable")
        } else if !valid_sha(&e.sha256) {
            Some("its sha256 is not a full lowercase SHA-256")
        } else if !valid_path(&e.path) {
            Some("its path leaves the catalog folder")
        } else if entries.iter().any(|x| x.id == e.id) {
            Some("its id is listed twice")
        } else {
            None
        };
        match why {
            Some(w) => skipped.push(format!("{}: {w}", e.id)),
            None => entries.push(Entry {
                name: super::cut(&e.name, super::MAX_NAME_CHARS),
                description: super::cut(&e.description, super::MAX_DESCRIPTION_CHARS),
                ..e
            }),
        }
    }
    if list.len() > MAX_ENTRIES {
        skipped.push(format!("only the first {MAX_ENTRIES} entries are read"));
    }
    Ok(Catalog { url: url.to_string(), entries, skipped })
}

/// Fetches and parses the configured catalog. Refused with `catalog.unset`, with no request, when no address is set.
pub fn fetch(store: &SkillStore, net: &dyn Fetch) -> Result<Catalog, Fail> {
    let url = store.catalog_url().ok_or_else(|| {
        Fail::new(
            "catalog.unset",
            "no skill catalog is set. Add its address in Settings → Skills → Community, or with `ove skills catalog --set <https address>`.",
        )
    })?;
    check_url(&url)?;
    let bytes = net.get(&url, MAX_CATALOG_BYTES)?;
    parse(&url, &bytes)
}

/// Fetches an entry's file and checks it against the pin. Refused with `skill.tampered` when the hash differs.
pub fn fetch_entry(catalog: &Catalog, e: &Entry, net: &dyn Fetch) -> Result<String, Fail> {
    let url = entry_url(&catalog.url, &e.path);
    let bytes = net.get(&url, MAX_SKILL_BYTES)?;
    let got = sha256_hex(&bytes);
    if got != e.sha256 {
        return Err(Fail::new(
            "skill.tampered",
            format!(
                "the content of `{}` failed verification: its SHA-256 is {got}, the catalog pins {}. Nothing was installed.",
                e.id, e.sha256
            ),
        )
        .details(json!({"id": e.id, "expected": e.sha256, "actual": got, "url": url})));
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| Fail::new("skill.invalid", format!("`{}` is not UTF-8 text; nothing was installed", e.id)))?;
    validate_text(&text).map_err(|why| Fail::new("skill.invalid", format!("`{}` was not installed: {why}", e.id)))?;
    Ok(text)
}

fn entry<'a>(catalog: &'a Catalog, id: &str) -> Result<&'a Entry, Fail> {
    catalog.entries.iter().find(|e| e.id == id).ok_or_else(|| {
        let ids: Vec<&str> = catalog.entries.iter().map(|e| e.id.as_str()).collect();
        Fail::new("catalog.missing", format!("the catalog has no skill `{id}`")).details(json!({"available": ids}))
    })
}

/// What installing an entry would do, for the person to confirm: the entry, the verified text, and the id it
/// would get.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub entry: Entry,
    pub catalog_url: String,
    pub install_as: String,
    pub text: String,
    /// Tool names the text mentions that are not agent tools.
    pub unknown_tools: Vec<String>,
}

/// Fetches and verifies an entry without installing it.
pub fn preview(store: &SkillStore, catalog: &Catalog, id: &str, net: &dyn Fetch) -> Result<Preview, Fail> {
    let e = entry(catalog, id)?;
    let text = fetch_entry(catalog, e, net)?;
    let install_as = if super::is_builtin(&e.id) || std::fs::symlink_metadata(store.dir(&e.id)).is_ok() {
        store.free_id(&e.id)
    } else {
        e.id.clone()
    };
    Ok(Preview {
        entry: e.clone(),
        catalog_url: catalog.url.clone(),
        install_as,
        unknown_tools: super::unknown_tool_names(&text),
        text,
    })
}

/// Installs a verified entry under `as_id` (default: the entry id). The person confirmed it. Refused with
/// `skill.tampered` when the content does not match its pin, and with `skill.exists` when the id is taken.
pub fn install(
    store: &SkillStore,
    catalog: &Catalog,
    id: &str,
    as_id: Option<&str>,
    net: &dyn Fetch,
) -> Result<super::Skill, Fail> {
    let e = entry(catalog, id)?;
    let target = as_id.unwrap_or(&e.id);
    if super::is_builtin(target) {
        return Err(Fail::new(
            "skill.builtin",
            format!("`{target}` is a builtin skill's id; install it under another id"),
        ));
    }
    if std::fs::symlink_metadata(store.dir(target)).is_ok() {
        return Err(Fail::new(
            "skill.exists",
            format!("a skill `{target}` is already installed; install it under another id, or update it"),
        )
        .details(json!({"suggestion": store.free_id(target)})));
    }
    let text = fetch_entry(catalog, e, net)?;
    store.write(target, &text, Origin::Catalog { url: catalog.url.clone() }, Some(e.id.clone()))
}

/// The update state of one catalog-installed skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum UpdateState {
    /// The catalog pins what is installed.
    #[serde(rename_all = "camelCase")]
    UpToDate { id: String },
    /// The catalog changed and the local file is as installed: the update may replace it once the person accepts.
    #[serde(rename_all = "camelCase")]
    Available { id: String, catalog_id: String, sha256: String },
    /// The catalog changed and the person edited the local file: the update is offered as a copy.
    #[serde(rename_all = "camelCase")]
    Modified { id: String, catalog_id: String, sha256: String },
    /// The person edited the local file; the catalog has not changed.
    #[serde(rename_all = "camelCase")]
    EditedLocally { id: String },
    /// The catalog no longer lists the skill. The installed copy stays.
    #[serde(rename_all = "camelCase")]
    Gone { id: String },
}

/// The update state of every skill installed from `catalog`.
pub fn updates(store: &SkillStore, catalog: &Catalog) -> Vec<UpdateState> {
    let ledger: Ledger = store.ledger();
    let mut out = Vec::new();
    for (id, row) in &ledger.skills {
        let Origin::Catalog { url } = &row.origin else { continue };
        if *url != catalog.url {
            continue;
        }
        let Ok(bytes) = std::fs::read(store.dir(id).join(super::SKILL_FILE)) else { continue };
        let local = sha256_hex(&bytes);
        let edited = local != row.sha256;
        let cid = row.catalog_id.clone().unwrap_or_else(|| id.clone());
        let state = match catalog.entries.iter().find(|e| e.id == cid) {
            None => UpdateState::Gone { id: id.clone() },
            Some(e) if e.sha256 == row.sha256 => {
                if edited {
                    UpdateState::EditedLocally { id: id.clone() }
                } else {
                    UpdateState::UpToDate { id: id.clone() }
                }
            }
            Some(e) if edited => UpdateState::Modified { id: id.clone(), catalog_id: cid, sha256: e.sha256.clone() },
            Some(e) => UpdateState::Available { id: id.clone(), catalog_id: cid, sha256: e.sha256.clone() },
        };
        out.push(state);
    }
    out
}

/// An offered update: the new text, verified against its pin, and the differences from the local file.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    pub id: String,
    /// True when the local file was edited: accepting installs a copy under `install_as`.
    pub as_copy: bool,
    pub install_as: String,
    pub sha256: String,
    pub text: String,
    pub diff: Vec<DiffLine>,
}

/// Fetches and verifies the update of `id` and shows what would change. Installs nothing.
pub fn offer(store: &SkillStore, catalog: &Catalog, id: &str, net: &dyn Fetch) -> Result<Offer, Fail> {
    let state = updates(store, catalog).into_iter().find(|s| match s {
        UpdateState::Available { id: x, .. } | UpdateState::Modified { id: x, .. } => x == id,
        _ => false,
    });
    let (catalog_id, as_copy) = match state {
        Some(UpdateState::Available { catalog_id, .. }) => (catalog_id, false),
        Some(UpdateState::Modified { catalog_id, .. }) => (catalog_id, true),
        _ => return Err(Fail::new("skill.noUpdate", format!("there is no update for `{id}`"))),
    };
    let e = entry(catalog, &catalog_id)?;
    let text = fetch_entry(catalog, e, net)?;
    let local = std::fs::read_to_string(store.dir(id).join(super::SKILL_FILE)).unwrap_or_default();
    let install_as = if as_copy { store.free_id(&format!("{id}-update")) } else { id.to_string() };
    Ok(Offer { id: id.to_string(), as_copy, install_as, sha256: e.sha256.clone(), diff: diff(&local, &text), text })
}

/// Installs an offered update the person accepted. The text is fetched and verified again, and must still be the
/// one the person saw (`sha256`). An unmodified skill is replaced; an edited one stays and the update is installed as
/// a copy with a different id.
pub fn accept(
    store: &SkillStore,
    catalog: &Catalog,
    id: &str,
    sha256: &str,
    net: &dyn Fetch,
) -> Result<super::Skill, Fail> {
    let o = offer(store, catalog, id, net)?;
    if o.sha256 != sha256 {
        return Err(Fail::new("skill.changed", "the catalog changed since the update was shown; look at it again"));
    }
    let catalog_id = updates(store, catalog)
        .into_iter()
        .find_map(|s| match s {
            UpdateState::Available { id: x, catalog_id, .. } | UpdateState::Modified { id: x, catalog_id, .. }
                if x == id =>
            {
                Some(catalog_id)
            }
            _ => None,
        })
        .unwrap_or_else(|| id.to_string());
    store.write(&o.install_as, &o.text, Origin::Catalog { url: catalog.url.clone() }, Some(catalog_id))
}

/// One line of a difference listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    /// `" "` unchanged, `"-"` removed, `"+"` added.
    pub op: String,
    pub text: String,
}

/// Line differences from `old` to `new` (longest common subsequence). Very large inputs are shown as a whole
/// replacement.
pub fn diff(old: &str, new: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let line = |op: &str, t: &str| DiffLine { op: op.into(), text: t.into() };
    if a.len().saturating_mul(b.len()) > 4_000_000 {
        return a.iter().map(|t| line("-", t)).chain(b.iter().map(|t| line("+", t))).collect();
    }
    let (n, m) = (a.len(), b.len());
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < n && j < m {
        if a[i] == b[j] {
            out.push(line(" ", a[i]));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            out.push(line("-", a[i]));
            i += 1;
        } else {
            out.push(line("+", b[j]));
            j += 1;
        }
    }
    out.extend(a[i..].iter().map(|t| line("-", t)));
    out.extend(b[j..].iter().map(|t| line("+", t)));
    out
}

#[cfg(test)]
mod tests;
