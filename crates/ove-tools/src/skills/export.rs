//! Export of skills to the local agents that read skill folders (design D5): a copy at `<agent skills>/ove-<id>/
//! SKILL.md`. The copy's front matter is written fresh (name `ove-<id>` and the description, nothing else), so no key
//! from a skill file can change what the other agent allows. A copy the person changed in the other agent's folder is
//! not overwritten unless they confirm.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::json;

use super::{Skill, SkillStore, sha256_hex};
use crate::fail::Fail;

/// The prefix of exported skill folders and names.
pub const PREFIX: &str = "ove-";

/// One export target as found on this computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub id: String,
    pub name: String,
    pub dir: PathBuf,
    pub available: bool,
    /// Why it is unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The export targets under `home`: available when the agent's folder exists.
pub fn targets(home: &Path) -> Vec<Target> {
    ove_agents::SKILL_TARGETS
        .iter()
        .map(|t| {
            let marker = ove_agents::under_home(home, t.marker);
            let available = std::fs::symlink_metadata(&marker).is_ok_and(|m| m.is_dir());
            Target {
                id: t.id.into(),
                name: t.name.into(),
                dir: ove_agents::under_home(home, t.dir),
                available,
                reason: (!available)
                    .then(|| format!("{} is not installed for this user ({} not found)", t.name, marker.display())),
            }
        })
        .collect()
}

/// The exported folder name: `ove-<id>`, at most 64 characters.
pub fn exported_name(id: &str) -> String {
    let mut n = format!("{PREFIX}{id}");
    n.truncate(64);
    n.trim_end_matches('-').to_string()
}

/// The exported file: fresh front matter, then the skill's body.
pub fn exported_text(s: &Skill) -> String {
    let description = serde_json::to_string(&s.description).unwrap_or_else(|_| "\"\"".into());
    format!(
        "---\nname: {}\ndescription: {description}\n---\n\n<!-- Exported from OpenVideoEditor skill `{}` ({}). -->\n\n{}\n",
        exported_name(&s.id),
        s.id,
        s.origin.label(),
        s.body().trim()
    )
}

/// What an export did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Exported {
    pub target: String,
    pub file: PathBuf,
    /// True when an earlier export was replaced.
    pub replaced: bool,
}

/// Exports skill `id` to one target. Refused with `export.unavailable` when the agent is not installed, and with
/// `export.modified` when the copy there was changed since the last export, unless `overwrite` (the person
/// confirmed).
pub fn export(store: &SkillStore, id: &str, target: &Target, overwrite: bool) -> Result<Exported, Fail> {
    if !target.available {
        return Err(Fail::new(
            "export.unavailable",
            target.reason.clone().unwrap_or_else(|| format!("{} is not available", target.name)),
        ));
    }
    let skill = store.get(id)?;
    let dir = target.dir.join(exported_name(id));
    let file = dir.join(super::SKILL_FILE);
    let io = |what: &str, p: &Path, e: std::io::Error| {
        Fail::new("export.io", format!("could not {what} {}: {e}", p.display()))
    };
    for p in [&dir, &file] {
        if std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(Fail::new("export.io", format!("{} is a link; nothing was written", p.display())));
        }
    }
    let key = file.to_string_lossy().into_owned();
    let mut ledger = store.ledger();
    let existing = std::fs::read(&file).ok();
    if let Some(bytes) = &existing {
        let recorded = ledger.exports.get(&key);
        let unchanged = recorded.is_some_and(|sha| *sha == sha256_hex(bytes));
        if !unchanged && !overwrite {
            return Err(Fail::new(
                "export.modified",
                format!(
                    "{} was changed in {} since it was exported (or was not written by this app). Confirm to replace it.",
                    file.display(),
                    target.name
                ),
            )
            .details(json!({"file": file, "target": target.id})));
        }
    }
    let text = exported_text(&skill);
    std::fs::create_dir_all(&dir).map_err(|e| io("create", &dir, e))?;
    ove_store::fsio::write_derived(&file, text.as_bytes()).map_err(|e| io("write", &file, e))?;
    ledger.exports.insert(key, sha256_hex(text.as_bytes()));
    store.save_ledger(&ledger)?;
    Ok(Exported { target: target.id.clone(), file, replaced: existing.is_some() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::{Origin, SKILL_FILE};

    const TIDY: &str = "---\nname: Tidy cuts\ndescription: \"Close gaps: small ones\"\nallowed-tools: Bash(*)\n---\n\nClose each gap.\n";

    fn setup(agents: &[&str]) -> (tempfile::TempDir, SkillStore, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        for a in agents {
            std::fs::create_dir_all(home.join(a)).unwrap();
        }
        let s = SkillStore::of_library(&d.path().join("lib"));
        s.write("tidy-cuts", TIDY, Origin::Local, None).unwrap();
        (d, s, home)
    }

    fn target(home: &Path, id: &str) -> Target {
        targets(home).into_iter().find(|t| t.id == id).unwrap()
    }

    #[test]
    fn export_writes_a_prefixed_copy_with_fresh_front_matter() {
        let (_d, s, home) = setup(&[".claude"]);
        let out = export(&s, "tidy-cuts", &target(&home, "claude"), false).unwrap();
        assert_eq!(out.file, home.join(".claude").join("skills").join("ove-tidy-cuts").join(SKILL_FILE));
        assert!(!out.replaced);
        let text = std::fs::read_to_string(&out.file).unwrap();
        assert!(text.starts_with("---\nname: ove-tidy-cuts\ndescription: \"Close gaps: small ones\"\n---\n"), "{text}");
        assert!(!text.contains("allowed-tools"), "no key from the skill reaches the other agent: {text}");
        assert!(text.contains("Close each gap."));
        // The copy is itself a valid skill.
        assert!(crate::skills::validate_text(&text).is_ok());
        // Builtin skills export too.
        let b = export(&s, "caption-styling", &target(&home, "claude"), false).unwrap();
        assert!(b.file.ends_with("ove-caption-styling/SKILL.md") || b.file.ends_with("ove-caption-styling\\SKILL.md"));
    }

    #[test]
    fn a_missing_agent_is_unavailable_with_the_reason() {
        let (_d, s, home) = setup(&[".claude"]);
        let all = targets(&home);
        let ids: Vec<&str> = all.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["claude", "codex", "cursor"]);
        let cursor = target(&home, "cursor");
        assert!(!cursor.available);
        assert!(cursor.reason.as_deref().unwrap().contains("Cursor is not installed"));
        assert_eq!(export(&s, "tidy-cuts", &cursor, false).unwrap_err().code, "export.unavailable");
        assert!(!home.join(".cursor").exists());
    }

    #[test]
    fn a_changed_copy_is_replaced_only_after_confirmation() {
        let (_d, s, home) = setup(&[".codex"]);
        let t = target(&home, "codex");
        let first = export(&s, "tidy-cuts", &t, false).unwrap();
        // Exporting again over an unchanged copy needs no confirmation.
        assert!(export(&s, "tidy-cuts", &t, false).unwrap().replaced);
        std::fs::write(&first.file, "my own edit").unwrap();
        let e = export(&s, "tidy-cuts", &t, false).unwrap_err();
        assert_eq!(e.code, "export.modified");
        assert_eq!(std::fs::read_to_string(&first.file).unwrap(), "my own edit");
        export(&s, "tidy-cuts", &t, true).unwrap();
        assert!(std::fs::read_to_string(&first.file).unwrap().contains("Close each gap."));
        // A folder the app did not write counts as changed.
        let other = t.dir.join("ove-caption-styling");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join(SKILL_FILE), "someone else's").unwrap();
        assert_eq!(export(&s, "caption-styling", &t, false).unwrap_err().code, "export.modified");
    }

    #[test]
    fn unknown_and_invalid_skills_are_not_exported() {
        let (_d, s, home) = setup(&[".claude"]);
        assert_eq!(export(&s, "nope", &target(&home, "claude"), false).unwrap_err().code, "skill.unknown");
    }

    #[test]
    fn names_stay_within_64_characters() {
        let id = "a".repeat(64);
        assert_eq!(exported_name(&id).len(), 64);
        assert!(exported_name(&id).starts_with("ove-"));
    }
}
