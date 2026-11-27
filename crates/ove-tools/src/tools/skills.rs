//! `list_skills`, `read_skill` and the in-app `manage_skills`: the skill store (`crate::skills`) as agent tools.

use serde_json::json;

use crate::ctx::ToolCtx;
use crate::fail::{Fail, plural};
use crate::receipt::Receipt;
use crate::skills::{Skill, agent_text};
use crate::spec::Args;

fn line(s: &Skill) -> String {
    format!("{}\t{}\t({})", s.id, s.description, s.origin.label())
}

/// The valid skills: id, name, description and source.
pub fn list_skills(ctx: &mut dyn ToolCtx, _: &Args) -> Result<Receipt, Fail> {
    let skills = ctx.skills()?.valid();
    let text = if skills.is_empty() {
        "No skills are installed.".to_string()
    } else {
        let mut t = skills.iter().map(line).collect::<Vec<_>>().join("\n");
        t.push_str("\n\n`read_skill` with an id gives the whole skill.");
        t
    };
    Ok(Receipt::new(
        json!({"count": skills.len(), "skills": skills.iter().map(Skill::summary_json).collect::<Vec<_>>()}),
        text,
    ))
}

/// One skill's whole text. Refused with `skill.unknown` (listing the installed ids) for an id that is not
/// installed.
pub fn read_skill(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let id = a.req_str("id")?;
    let s = ctx.skills()?.get(id)?;
    let mut v = s.summary_json();
    v["text"] = s.text.clone().into();
    Ok(Receipt::new(v, agent_text(&s)))
}

/// Creates, updates or removes a skill that is not builtin (in-app only). The change shows in Settings → Skills as
/// created by the assistant.
pub fn manage_skills(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let store = ctx.skills()?;
    let action = a.req_str("action")?;
    match action {
        "create" | "update" => {
            let text = a.req_str("text")?;
            let id = match (action, a.str("id")) {
                ("update", Some(id)) => {
                    // The skill must exist; builtin ones are refused by the store.
                    if !crate::skills::is_builtin(id) && std::fs::symlink_metadata(store.dir(id)).is_err() {
                        return Err(store.get(id).err().unwrap_or_else(|| Fail::new("skill.unknown", id.to_string())));
                    }
                    id.to_string()
                }
                ("update", None) => return Err(Fail::new("argument.missing", "update needs `id`")),
                (_, Some(id)) => {
                    if crate::skills::is_builtin(id) {
                        return Err(Fail::new("skill.builtin", format!("`{id}` is a builtin skill; pick another id")));
                    }
                    if std::fs::symlink_metadata(store.dir(id)).is_ok() {
                        return Err(Fail::new(
                            "skill.exists",
                            format!("a skill `{id}` already exists; use action update, or another id"),
                        ));
                    }
                    id.to_string()
                }
                (_, None) => {
                    let fm = crate::skills::validate_text(text)
                        .map_err(|why| Fail::new("skill.invalid", format!("not saved: {why}")))?;
                    store.free_id(&crate::skills::id_from_name(fm.name.as_deref().unwrap_or("skill")))
                }
            };
            let s = store.write(&id, text, crate::skills::Origin::Assistant, None)?;
            let verb = if action == "create" { "Created" } else { "Updated" };
            let unknown = crate::skills::unknown_tool_names(&s.text);
            let mut msg = format!("{verb} skill `{}` ({}). It shows in Settings → Skills.", s.id, s.name);
            if !unknown.is_empty() {
                msg.push_str(&format!(
                    " Note: it names {} that do not exist: {}.",
                    plural(unknown.len(), "tool"),
                    unknown.join(", ")
                ));
            }
            let mut v = s.summary_json();
            v["action"] = action.into();
            v["unknownTools"] = json!(unknown);
            Ok(Receipt::new(v, msg))
        }
        "remove" => {
            let id = a.req_str("id")?;
            store.remove(id)?;
            Ok(Receipt::new(json!({"action": "remove", "id": id}), format!("Removed skill `{id}`.")))
        }
        other => {
            Err(Fail::new("argument.invalid", format!("`action` must be create, update or remove, not {other:?}")))
        }
    }
}
