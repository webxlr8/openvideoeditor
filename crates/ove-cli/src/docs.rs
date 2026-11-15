//! `ove docs`, `ove skills install`, `ove doctor`.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::common::{Fail, Out};

/// Built-in documentation pages (the files in docs/cli).
pub const TOPICS: &[(&str, &str, &str)] = &[
    ("agents", "How an agent edits a project with ove (start here)", include_str!("../../../docs/cli/agents.md")),
    ("timeline", "source print / check / apply, verify, history, undo", include_str!("../../../docs/cli/timeline.md")),
    (
        "project",
        "project list / create / show / rename / duplicate / remove / open / reveal",
        include_str!("../../../docs/cli/project.md"),
    ),
    ("media", "media list / add / remove / relink / probe", include_str!("../../../docs/cli/media.md")),
    (
        "app",
        "app context / open / seek / play / pause and how edits reach the running app",
        include_str!("../../../docs/cli/app.md"),
    ),
    ("export", "export to MP4", include_str!("../../../docs/cli/export.md")),
    (
        "interchange",
        "timeline export / import (FCPXML, XML, EDL, OTIO)",
        include_str!("../../../docs/cli/interchange.md"),
    ),
    ("mcp", "the MCP server and agent setup", include_str!("../../../docs/cli/mcp.md")),
];

pub fn docs(topic: Option<&str>) -> Out {
    match topic {
        None => {
            let text = TOPICS.iter().map(|(n, about, _)| format!("{n}\t{about}")).collect::<Vec<_>>().join("\n");
            let v = json!({"topics": TOPICS.iter().map(|(n, about, _)| json!({"name": n, "about": about})).collect::<Vec<_>>()});
            Ok((v, format!("{text}\n\n`ove docs <topic>` prints one.")))
        }
        Some(t) => {
            let (n, _, body) = TOPICS
                .iter()
                .find(|(n, ..)| n.eq_ignore_ascii_case(t))
                .ok_or_else(|| Fail::new("docs.notFound", format!("no topic {t}; `ove docs` lists them")))?;
            Ok((json!({"topic": n, "text": body}), body.trim_end().to_string()))
        }
    }
}

/// The agent skill: front matter + the agents guide.
pub fn skill_text() -> String {
    let guide = TOPICS[0].2;
    format!(
        "---\nname: openvideoeditor\ndescription: Edit OpenVideoEditor video projects with the `ove` command line — read and change the timeline as TSX text, add media, check frames, export. Use when the user asks to cut, arrange, caption or export a video in OpenVideoEditor.\n---\n\n{guide}"
    )
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

/// Writes the skill to `<dir>/openvideoeditor/SKILL.md` for each skills folder: the given ones, or the Claude Code
/// and Codex user folders that exist.
pub fn install_skills(dirs: &[PathBuf]) -> Out {
    let targets: Vec<PathBuf> = if dirs.is_empty() {
        let h = home().ok_or_else(|| Fail::new("home.unknown", "no home folder; pass --dir"))?;
        [(".claude", "skills"), (".codex", "skills")]
            .iter()
            .filter(|(app, _)| h.join(app).is_dir())
            .map(|(app, sub)| h.join(app).join(sub))
            .collect()
    } else {
        dirs.to_vec()
    };
    if targets.is_empty() {
        return Err(Fail::new(
            "skills.noTarget",
            "found neither ~/.claude nor ~/.codex; pass --dir <skills folder> (Claude Code: ~/.claude/skills)",
        ));
    }
    let text = skill_text();
    let mut written = Vec::new();
    for d in targets {
        let f = d.join("openvideoeditor").join("SKILL.md");
        std::fs::create_dir_all(f.parent().unwrap_or(&d))
            .and_then(|_| std::fs::write(&f, &text))
            .map_err(|e| Fail::new("file.unwritable", format!("{}: {e}", f.display())))?;
        written.push(f);
    }
    let lines: Vec<String> = written.iter().map(|p| format!("wrote {}", p.display())).collect();
    Ok((
        json!({"written": written}),
        format!(
            "{}\nTo give agents the MCP tools as well:\n  claude mcp add openvideoeditor -- {} mcp\n  codex mcp add openvideoeditor -- {} mcp",
            lines.join("\n"),
            exe(),
            exe()
        ),
    ))
}

fn exe() -> String {
    std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "ove".into())
}

fn agent_line(d: &ove_agents::Detected) -> String {
    match d.state {
        ove_agents::State::Ready => format!("ok   {}: {}", d.name, d.version.as_deref().unwrap_or("")),
        ove_agents::State::Found => {
            format!("ok   {}: found at {} (it did not say its version)", d.name, d.path.as_deref().unwrap_or("?"))
        }
        ove_agents::State::NotInstalled => format!("--   {}: not installed ({})", d.name, d.get_it),
    }
}

/// The supported agents as found on this computer.
pub fn agents_list() -> Out {
    let found = ove_agents::detect_all(&ove_agents::Env::from_process(), &ove_agents::Limits::default());
    let text = found.iter().map(agent_line).collect::<Vec<_>>().join("\n");
    Ok((json!({"agents": found}), text))
}

/// Gives an installed agent the editing guide (when it reads skills) and this program's MCP server.
pub fn agents_connect(id: &str) -> Out {
    let known = || ove_agents::AGENTS.iter().map(|a| a.id).collect::<Vec<_>>().join(", ");
    let spec = ove_agents::agent(id)
        .ok_or_else(|| Fail::new("agent.unknown", format!("no agent {id:?}; the supported agents are: {}", known())))?;
    let env = ove_agents::Env::from_process();
    let limits = ove_agents::Limits::default();
    let found = ove_agents::detect(spec, &env, &limits);
    let program = found.path.as_deref().map(PathBuf::from).ok_or_else(|| {
        Fail::new("agent.notInstalled", format!("{} is not installed on this computer ({})", spec.name, spec.get_it))
    })?;
    let ove = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("ove"));
    let mut lines = Vec::new();
    let mut guide = None;
    if let Some(dir) = env.home.as_deref().and_then(|h| ove_agents::skills_dir(spec, h)) {
        let (_, text) = install_skills(std::slice::from_ref(&dir))?;
        lines.extend(text.lines().take_while(|l| l.starts_with("wrote ")).map(str::to_string));
        guide = Some(dir);
    }
    let outcome = ove_agents::connect(spec, &program, &ove, &limits);
    lines.push(match outcome.status {
        ove_agents::ConnectStatus::Connected => format!("{} now has the {} tools", spec.name, ove_agents::SERVER_NAME),
        ove_agents::ConnectStatus::AlreadyConnected => {
            format!("{} already had the {} tools", spec.name, ove_agents::SERVER_NAME)
        }
        ove_agents::ConnectStatus::Manual => {
            format!("To finish, do this yourself ({}):\n  {}", outcome.detail, outcome.instructions)
        }
        ove_agents::ConnectStatus::Failed => {
            return Err(Fail::new(
                "agent.connectFailed",
                format!(
                    "{} refused: {}\nYou can run it yourself:\n  {}",
                    spec.name, outcome.detail, outcome.instructions
                ),
            ));
        }
    });
    Ok((json!({"agent": spec.id, "outcome": outcome, "guide": guide}), lines.join("\n")))
}

/// Environment check: library, media support, encoders, the running app, agent setup.
pub fn doctor(library: Option<&Path>) -> Out {
    let mut checks: Vec<Value> = Vec::new();
    let mut add =
        |name: &str, ok: bool, detail: String| checks.push(json!({"check": name, "ok": ok, "detail": detail}));
    add("version", true, env!("CARGO_PKG_VERSION").to_string());
    let lib = match library {
        Some(p) => ove_store::library::validate_library(p).map(|_| p.to_path_buf()),
        None => ove_store::library::resolve_library(None),
    };
    match &lib {
        Ok(p) => {
            let n = ove_store::Library::open(p, crate::common::clock())
                .and_then(|l| l.list())
                .map(|v| v.len())
                .unwrap_or(0);
            add("library", true, format!("{} ({n} projects)", p.display()));
        }
        Err(e) => add("library", false, e.to_string()),
    }
    #[cfg(feature = "media")]
    {
        match std::panic::catch_unwind(ove_media::ffmpeg_version) {
            Ok(v) => add("media", true, format!("FFmpeg {v}")),
            Err(_) => add("media", false, "FFmpeg libraries could not be loaded".into()),
        }
        add("export encoders", true, ove_render::export::encoder_candidates().join(", "));
    }
    #[cfg(not(feature = "media"))]
    add("media", false, "this build cannot read media or export (build with --features media)".into());
    match lib.as_ref().ok().map(|p| ove_control::call(p, "context", json!({}), std::time::Duration::from_secs(3))) {
        Some(Ok(v)) => add(
            "app",
            true,
            format!(
                "running; {}",
                v.pointer("/project/name")
                    .and_then(Value::as_str)
                    .map(|n| format!("project {n} open"))
                    .unwrap_or("no project open".into())
            ),
        ),
        Some(Err(e)) => add("app", false, e.to_string()),
        None => add("app", false, "no library".into()),
    }
    let skill = home().map(|h| h.join(".claude/skills/openvideoeditor/SKILL.md"));
    add(
        "agent skill",
        skill.as_ref().is_some_and(|p| p.is_file()),
        match &skill {
            Some(p) if p.is_file() => p.display().to_string(),
            _ => "not installed (`ove skills install`)".into(),
        },
    );
    let agents = ove_agents::detect_all(&ove_agents::Env::from_process(), &ove_agents::Limits::default());
    let installed: Vec<String> = agents
        .iter()
        .filter(|a| a.state != ove_agents::State::NotInstalled)
        .map(|a| match &a.version {
            Some(v) => format!("{} ({v})", a.name),
            None => a.name.clone(),
        })
        .collect();
    add(
        "agents",
        !installed.is_empty(),
        if installed.is_empty() {
            "none of the supported agents is installed (`ove agents`)".into()
        } else {
            installed.join(", ")
        },
    );
    let text = checks
        .iter()
        .map(|c| {
            format!(
                "{} {}: {}",
                if c["ok"] == true { "ok  " } else { "--  " },
                c["check"].as_str().unwrap_or(""),
                c["detail"].as_str().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok((json!({"checks": checks}), text))
}
