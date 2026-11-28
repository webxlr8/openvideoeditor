//! Desktop agent clients the editor's MCP server can be added to in one step (add-agent-skills-and-connectors):
//! Claude Desktop through a desktop-extension package (`.mcpb`), and Cursor through its install link. Each client is
//! data: where it is installed, where its configuration lives, and how a connection is made. A copyable manual
//! configuration is always available as the fallback.
//!
//! Nothing here sends anything anywhere: detection reads folders and configuration files, the install link is
//! opened by the operating system on the person's click, and Disconnect removes only the editor's own entry.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Value, json};

use crate::SERVER_NAME;

/// How a client is connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectKind {
    /// Open a desktop-extension package with the OS; the client shows its install prompt.
    Extension,
    /// Open an install link built from `template` (`{name}`, `{config}`: the base64 of the server's JSON).
    Link { template: &'static str },
}

/// One client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub get_it: &'static str,
    pub connect: ConnectKind,
}

/// The Cursor install link (from Cursor's documentation of MCP install links). Kept as data: when the format
/// changes, only this line does, and the manual snippet still works.
pub const CURSOR_LINK: &str = "cursor://anysphere.cursor-deeplink/mcp/install?name={name}&config={config}";

pub const CLIENTS: &[ClientSpec] = &[
    ClientSpec {
        id: "claude-desktop",
        name: "Claude Desktop",
        get_it: "https://claude.ai/download",
        connect: ConnectKind::Extension,
    },
    ClientSpec {
        id: "cursor",
        name: "Cursor",
        get_it: "https://cursor.com/downloads",
        connect: ConnectKind::Link { template: CURSOR_LINK },
    },
];

pub fn client(id: &str) -> Option<&'static ClientSpec> {
    CLIENTS.iter().find(|c| c.id == id)
}

/// The folders detection looks in. Built from the process by [`ClientEnv::from_process`]; tests build their own.
#[derive(Debug, Clone, Default)]
pub struct ClientEnv {
    pub home: Option<PathBuf>,
    /// Windows `%APPDATA%` (roaming).
    pub appdata: Option<PathBuf>,
    /// Windows `%LOCALAPPDATA%`.
    pub local_appdata: Option<PathBuf>,
    /// `macos`, `windows` or another OS name.
    pub os: &'static str,
}

impl ClientEnv {
    pub fn from_process() -> ClientEnv {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        ClientEnv {
            home: var("HOME").or_else(|| var("USERPROFILE")),
            appdata: var("APPDATA"),
            local_appdata: var("LOCALAPPDATA"),
            os: std::env::consts::OS,
        }
    }

    /// The client's configuration folder.
    fn config_dir(&self, id: &str) -> Option<PathBuf> {
        match (id, self.os) {
            ("claude-desktop", "macos") => self.home.as_ref().map(|h| h.join("Library/Application Support/Claude")),
            ("claude-desktop", "windows") => self.appdata.as_ref().map(|a| a.join("Claude")),
            ("claude-desktop", _) => self.home.as_ref().map(|h| h.join(".config").join("Claude")),
            ("cursor", _) => self.home.as_ref().map(|h| h.join(".cursor")),
            _ => None,
        }
    }

    /// The client's MCP configuration file.
    pub fn config_file(&self, id: &str) -> Option<PathBuf> {
        let dir = self.config_dir(id)?;
        Some(match id {
            "claude-desktop" => dir.join("claude_desktop_config.json"),
            _ => dir.join("mcp.json"),
        })
    }

    /// Where the client's program is, when it is installed for everyone or for this user.
    fn programs(&self, id: &str) -> Vec<PathBuf> {
        let mut out = Vec::new();
        match (id, self.os) {
            ("claude-desktop", "macos") => {
                out.push(PathBuf::from("/Applications/Claude.app"));
                out.extend(self.home.as_ref().map(|h| h.join("Applications/Claude.app")));
            }
            ("claude-desktop", "windows") => {
                out.extend(self.local_appdata.as_ref().map(|l| l.join("AnthropicClaude").join("claude.exe")));
                out.extend(self.local_appdata.as_ref().map(|l| l.join("Programs").join("Claude").join("Claude.exe")));
            }
            ("cursor", "macos") => {
                out.push(PathBuf::from("/Applications/Cursor.app"));
                out.extend(self.home.as_ref().map(|h| h.join("Applications/Cursor.app")));
            }
            ("cursor", "windows") => {
                out.extend(self.local_appdata.as_ref().map(|l| l.join("Programs").join("cursor").join("Cursor.exe")));
            }
            _ => {}
        }
        out
    }
}

/// A client's state on this computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientStatus {
    pub id: String,
    pub name: String,
    pub installed: bool,
    /// Its configuration holds the editor's server (or, for Claude Desktop, the editor's extension is installed).
    pub connected: bool,
    pub get_it: String,
    /// The configuration file the manual snippet goes into.
    pub config_file: Option<PathBuf>,
    /// The manual configuration to paste.
    pub snippet: String,
    /// The install link (Cursor).
    pub link: Option<String>,
}

/// The server entry: `ove mcp` with the absolute path of `ove`.
pub fn server_entry(ove: &Path) -> Value {
    json!({"command": ove.display().to_string(), "args": ["mcp"]})
}

/// The manual configuration: the `mcpServers` object with the editor's server.
pub fn snippet(ove: &Path) -> String {
    let v = json!({"mcpServers": {SERVER_NAME: server_entry(ove)}});
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// Base64 (standard alphabet, padded).
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}

fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The install link for a link client, carrying the server entry for `ove`.
pub fn install_link(spec: &ClientSpec, ove: &Path) -> Option<String> {
    let ConnectKind::Link { template } = spec.connect else { return None };
    let config = base64(server_entry(ove).to_string().as_bytes());
    Some(template.replace("{name}", SERVER_NAME).replace("{config}", &url_encode(&config)))
}

/// True when a server entry runs the editor's `ove mcp`.
fn is_ours(name: &str, entry: &Value) -> bool {
    if name == SERVER_NAME {
        return true;
    }
    let command = entry["command"].as_str().unwrap_or("");
    let file = command.rsplit(['/', '\\']).next().unwrap_or("").to_ascii_lowercase();
    let runs_mcp = entry["args"].as_array().is_some_and(|a| a.first().and_then(Value::as_str) == Some("mcp"));
    (file == "ove" || file == "ove.exe") && runs_mcp
}

/// Whether a client configuration (`{"mcpServers": {...}}`) holds the editor's server. Unreadable or other JSON:
/// not connected.
pub fn config_has_server(text: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return false };
    v["mcpServers"].as_object().is_some_and(|m| m.iter().any(|(k, e)| is_ours(k, e)))
}

/// Whether Claude Desktop has the editor's extension installed: a folder under `Claude Extensions` whose manifest
/// names it.
fn extension_installed(config_dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(config_dir.join("Claude Extensions")) else { return false };
    rd.flatten().any(|e| {
        std::fs::read(e.path().join("manifest.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .is_some_and(|m| m["name"] == SERVER_NAME)
    })
}

/// The state of client `spec`.
pub fn status(spec: &ClientSpec, env: &ClientEnv, ove: &Path) -> ClientStatus {
    let config_file = env.config_file(spec.id);
    let config_dir = env.config_dir(spec.id);
    let program = env.programs(spec.id).into_iter().any(|p| p.exists());
    let installed = program || config_dir.as_ref().is_some_and(|d| d.is_dir());
    let in_config =
        config_file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()).is_some_and(|t| config_has_server(&t));
    let extension =
        spec.connect == ConnectKind::Extension && config_dir.as_ref().is_some_and(|d| extension_installed(d));
    ClientStatus {
        id: spec.id.into(),
        name: spec.name.into(),
        installed,
        connected: in_config || extension,
        get_it: spec.get_it.into(),
        config_file,
        snippet: snippet(ove),
        link: install_link(spec, ove),
    }
}

/// Every client's state.
pub fn statuses(env: &ClientEnv, ove: &Path) -> Vec<ClientStatus> {
    CLIENTS.iter().map(|c| status(c, env, ove)).collect()
}

/// Removes the editor's server entries from the client's configuration file (the person clicked Disconnect). Keeps
/// every other entry and key, and leaves a copy of the old file next to it (`.bak`). Returns whether anything was
/// removed. A Claude Desktop extension is removed in Claude Desktop itself.
pub fn disconnect(spec: &ClientSpec, env: &ClientEnv) -> Result<bool, String> {
    let file = env.config_file(spec.id).ok_or("no configuration folder for this client")?;
    let text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(_) => return Ok(false),
    };
    let mut v: Value = serde_json::from_str(&text)
        .map_err(|e| format!("{} is not valid JSON ({e}); nothing was changed", file.display()))?;
    let Some(servers) = v.get_mut("mcpServers").and_then(Value::as_object_mut) else { return Ok(false) };
    let ours: Vec<String> = servers.iter().filter(|(k, e)| is_ours(k, e)).map(|(k, _)| k.clone()).collect();
    if ours.is_empty() {
        return Ok(false);
    }
    for k in &ours {
        servers.remove(k);
    }
    let backup = file.with_extension("json.bak");
    std::fs::write(&backup, &text).map_err(|e| format!("could not keep a copy at {}: {e}", backup.display()))?;
    let out = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())? + "\n";
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, out)
        .and_then(|()| std::fs::rename(&tmp, &file))
        .map_err(|e| format!("could not write {}: {e}", file.display()))?;
    Ok(true)
}

/// The desktop-extension manifest (manifest version 0.3) for `platform` (`win32` or `darwin`). tools/mcpb/pack.mjs
/// writes the same one; a test keeps them equal.
pub fn mcpb_manifest(platform: &str, version: &str) -> Value {
    let entry = if platform == "win32" { "server/ove.exe" } else { "server/MacOS/ove" };
    json!({
        "manifest_version": "0.3",
        "name": SERVER_NAME,
        "display_name": "OpenVideoEditor",
        "version": version,
        "description": "Edit OpenVideoEditor video projects: read and change the timeline, add media, check frames, export.",
        "long_description": "Runs the editor's own command line (`ove mcp`) as a local MCP server. Edits to a project open in the app appear there at once and can be undone. Skills installed in the editor are offered as prompts. Nothing is sent anywhere by the server itself.",
        "author": {"name": "OpenVideoEditor"},
        "license": "Apache-2.0",
        "server": {
            "type": "binary",
            "entry_point": entry,
            "mcp_config": {
                "command": format!("${{__dirname}}/{entry}"),
                "args": ["mcp"],
                "env": {"OVE_LIBRARY": "${user_config.library}"},
            },
        },
        "user_config": {
            "library": {
                "type": "directory",
                "title": "Library folder",
                "description": "The folder that holds your projects. Leave empty to use the app's library.",
                "required": false,
                "default": "",
            },
        },
        "tools_generated": true,
        "prompts_generated": true,
        "keywords": ["video", "editing", "timeline"],
        "compatibility": {"platforms": [platform]},
    })
}

#[cfg(test)]
mod tests;
