//! Settings → Agents → Connect for desktop clients (add-agent-skills-and-connectors): Claude Desktop through a
//! desktop-extension package (`.mcpb`), Cursor through its install link, both with a copyable manual configuration.
//! Connect runs only on the person's click. For Claude Desktop it writes the package from the app's own `ove`, its
//! libraries and the third-party notices into the app's cache folder and opens it with the system, so Claude Desktop
//! shows its install prompt; for Cursor it opens the install link.

use std::io::Write;
use std::path::{Path, PathBuf};

use ove_agents::clients::{self, ClientEnv, ConnectKind};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult, ErrorCode};
use crate::worker;

/// One desktop client as the Connect list shows it.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorInfo {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub connected: bool,
    pub get_it: String,
    /// `extension` (a package the client installs) or `link` (an install link).
    pub kind: String,
    /// Where the manual configuration goes.
    pub config_file: Option<String>,
    /// The manual configuration to copy.
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorOpened {
    /// What was opened: the package file or the link.
    pub opened: String,
    pub detail: String,
}

fn ove() -> PathBuf {
    crate::agents::ove_program(&ove_agents::Env::from_process())
}

fn info(s: clients::ClientStatus, kind: ConnectKind) -> ConnectorInfo {
    ConnectorInfo {
        id: s.id,
        name: s.name,
        installed: s.installed,
        connected: s.connected,
        get_it: s.get_it,
        kind: match kind {
            ConnectKind::Extension => "extension".into(),
            ConnectKind::Link { .. } => "link".into(),
        },
        config_file: s.config_file.map(|p| p.display().to_string()),
        snippet: s.snippet,
    }
}

/// Claude Desktop and Cursor on this computer: installed, connected, and the manual configuration.
#[tauri::command]
#[specta::specta]
pub async fn connectors() -> AppResult<Vec<ConnectorInfo>> {
    worker::blocking("connectors", move || {
        let env = ClientEnv::from_process();
        let ove = ove();
        Ok(clients::CLIENTS.iter().map(|c| info(clients::status(c, &env, &ove), c.connect)).collect())
    })
    .await
}

/// Opens a file or link with the system's handler.
fn open_with_system(target: &str) -> std::io::Result<()> {
    let r = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(target).spawn()
    } else if cfg!(windows) {
        std::process::Command::new("explorer").arg(target).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(target).spawn()
    };
    r.map(|_| ())
}

/// The files of the package's `server/` folder: `ove`, the libraries it loads, and the notices.
fn server_files(ove: &Path, licenses: Option<&Path>) -> AppResult<Vec<(String, PathBuf)>> {
    if !ove.is_file() {
        return Err(AppError::new(
            ErrorCode::NotFound,
            format!("The ove command line was not found at {}.", ove.display()),
        ));
    }
    let lib = |name: &str| {
        ["avcodec", "avformat", "avutil", "swscale", "swresample", "dav1d", "openh264"].iter().any(|k| name.contains(k))
    };
    let mut files = Vec::new();
    let dir = ove.parent().unwrap_or(Path::new("."));
    if cfg!(target_os = "macos") {
        files.push(("MacOS/ove".to_string(), ove.to_path_buf()));
        let frameworks = dir.join("../Frameworks");
        for e in std::fs::read_dir(&frameworks).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.ends_with(".dylib") && lib(&name) {
                files.push((format!("Frameworks/{name}"), e.path()));
            }
        }
    } else {
        files.push(("ove.exe".to_string(), ove.to_path_buf()));
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.to_ascii_lowercase().ends_with(".dll") && lib(&name) {
                files.push((name, e.path()));
            }
        }
    }
    if let Some(l) = licenses.filter(|l| l.is_dir()) {
        let mut stack = vec![l.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if let Ok(rel) = p.strip_prefix(l) {
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    files.push((format!("licenses/{rel}"), p));
                }
            }
        }
    }
    Ok(files)
}

/// Writes the package: `manifest.json` and `server/…`.
pub fn write_package(out: &Path, platform: &str, version: &str, files: &[(String, PathBuf)]) -> AppResult<()> {
    let io = |e: std::io::Error| AppError::new(ErrorCode::Io, format!("Could not write the extension package: {e}"));
    let zerr =
        |e: zip::result::ZipError| AppError::new(ErrorCode::Io, format!("Could not write the extension package: {e}"));
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d).map_err(io)?;
    }
    let mut z = zip::ZipWriter::new(std::fs::File::create(out).map_err(io)?);
    let opts = |mode: u32| {
        zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(mode)
    };
    let manifest = serde_json::to_vec_pretty(&ove_agents::clients::mcpb_manifest(platform, version))
        .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?;
    z.start_file("manifest.json", opts(0o644)).map_err(zerr)?;
    z.write_all(&manifest).map_err(io)?;
    let has_notices = files.iter().any(|(n, _)| n.starts_with("licenses/"));
    for (name, path) in files {
        let exec = !name.starts_with("licenses/");
        z.start_file(format!("server/{name}"), opts(if exec { 0o755 } else { 0o644 })).map_err(zerr)?;
        z.write_all(&std::fs::read(path).map_err(io)?).map_err(io)?;
    }
    if !has_notices {
        // A development build has no generated notices folder: say what the libraries are and where their licences are.
        z.start_file("server/licenses/NOTICE.md", opts(0o644)).map_err(zerr)?;
        z.write_all(b"This package contains the OpenVideoEditor command line (Apache-2.0) and the FFmpeg libraries it loads, used under the LGPL 2.1 or later and dynamically linked. The full notices ship with the app in its licenses folder.\n")
            .map_err(io)?;
    }
    z.finish().map_err(zerr)?;
    Ok(())
}

/// Connect: opens the client's install path (the person clicked Connect). The client then asks the person to confirm.
#[tauri::command]
#[specta::specta]
pub async fn connector_connect(app: AppHandle, id: String) -> AppResult<ConnectorOpened> {
    worker::blocking("connector_connect", move || {
        let spec = clients::client(&id)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, format!("There is no connector {id}.")))?;
        let ove = ove();
        let open_err = |what: &str, e: std::io::Error| {
            AppError::new(
                ErrorCode::Io,
                format!("The system could not open the {what} ({e}). Use the manual configuration instead."),
            )
        };
        match spec.connect {
            ConnectKind::Link { .. } => {
                let link = clients::install_link(spec, &ove)
                    .ok_or_else(|| AppError::new(ErrorCode::Internal, "no install link"))?;
                open_with_system(&link).map_err(|e| open_err("install link", e))?;
                Ok(ConnectorOpened { opened: link, detail: format!("{} asks you to confirm the server.", spec.name) })
            }
            ConnectKind::Extension => {
                let platform = if cfg!(windows) { "win32" } else { "darwin" };
                let licenses = app.path().resource_dir().ok().map(|r| r.join("licenses"));
                let files = server_files(&ove, licenses.as_deref())?;
                let cache = app
                    .path()
                    .app_cache_dir()
                    .map_err(|e| AppError::new(ErrorCode::Io, format!("No cache folder: {e}")))?;
                let out = cache.join("connectors").join(format!("openvideoeditor-{platform}.mcpb"));
                write_package(&out, platform, &app.package_info().version.to_string(), &files)?;
                let target = out.display().to_string();
                open_with_system(&target).map_err(|e| open_err("extension package", e))?;
                Ok(ConnectorOpened {
                    opened: target,
                    detail: format!("{} asks you to install the extension.", spec.name),
                })
            }
        }
    })
    .await
}

/// Disconnect: removes the editor's server from the client's configuration file (a copy of the old file is kept). A
/// Claude Desktop extension is removed in Claude Desktop's settings.
#[tauri::command]
#[specta::specta]
pub async fn connector_disconnect(id: String) -> AppResult<bool> {
    worker::blocking("connector_disconnect", move || {
        let spec = clients::client(&id)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, format!("There is no connector {id}.")))?;
        clients::disconnect(spec, &ClientEnv::from_process()).map_err(|e| AppError::new(ErrorCode::Io, e))
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    #[test]
    fn the_package_holds_the_manifest_the_server_and_notices() {
        let d = tempfile::tempdir().unwrap();
        let bin = d.path().join("ove.exe");
        std::fs::write(&bin, b"binary").unwrap();
        let out = d.path().join("p.mcpb");
        write_package(&out, "win32", "1.2.3", &[("ove.exe".into(), bin.clone())]).unwrap();
        let mut z = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        let mut names: Vec<String> = (0..z.len()).map(|i| z.by_index(i).unwrap().name().unwrap().to_string()).collect();
        names.sort();
        assert_eq!(names, ["manifest.json", "server/licenses/NOTICE.md", "server/ove.exe"]);
        let mut m = String::new();
        z.by_name("manifest.json").unwrap().read_to_string(&mut m).unwrap();
        let v: serde_json::Value = serde_json::from_str(&m).unwrap();
        assert_eq!(v["server"]["entry_point"], "server/ove.exe");
        assert_eq!(v["version"], "1.2.3");
        // The server stays executable where that is a file mode (the package Connect writes on macOS).
        if cfg!(unix) {
            assert_eq!(z.by_name("server/ove.exe").unwrap().unix_mode().map(|m| m & 0o777), Some(0o755));
        }
        // A missing command line is reported, not packaged.
        assert!(server_files(&d.path().join("nope.exe"), None).is_err());
    }
}
