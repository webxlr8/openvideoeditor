//! Settings → Agents & command line: putting the `ove` command on the search path (add-settings-and-templates 1.4,
//! add-agent-cli-and-mcp 3.4).
//!
//! - Windows: the folder of `ove.exe` (the app's own folder) is added to the user's `Path` in
//!   `HKEY_CURRENT_USER\Environment`, the same value System Properties edits, and Windows is told so that new
//!   terminals see it. Remove takes it out again. Nothing needs an administrator.
//! - macOS: `/usr/local/bin` needs an administrator, so the app shows the one command to run in Terminal.
//! - Linux: the package installs `/usr/bin/ove`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::worker;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CliStatus {
    /// The `ove` program that ships with the app.
    pub program: String,
    /// A new terminal finds `ove` (the folder is on the user's or the computer's search path).
    pub on_path: bool,
    /// The app can add it to (and remove it from) the search path itself.
    pub can_install: bool,
    /// What to run by hand instead, where the app cannot do it.
    pub instruction: Option<String>,
}

/// Path lists: entries split on `;`, compared without case and trailing separators (as Windows does).
fn same_dir(a: &str, b: &str) -> bool {
    let n = |s: &str| s.trim().trim_end_matches(['\\', '/']).to_lowercase();
    !a.trim().is_empty() && n(a) == n(b)
}

pub fn path_has(list: &str, dir: &str) -> bool {
    list.split(';').any(|e| same_dir(e, dir))
}

/// `list` with `dir` appended (unchanged when it is already there).
pub fn path_with(list: &str, dir: &str) -> String {
    if path_has(list, dir) {
        return list.to_string();
    }
    let base = list.trim_end_matches(';');
    if base.trim().is_empty() { dir.to_string() } else { format!("{base};{dir}") }
}

/// `list` without `dir` (every occurrence), other entries untouched.
pub fn path_without(list: &str, dir: &str) -> String {
    list.split(';').filter(|e| !same_dir(e, dir)).collect::<Vec<_>>().join(";")
}

fn ove() -> PathBuf {
    crate::agents::ove_program(&ove_agents::Env::from_process())
}

#[cfg(windows)]
mod win {
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, LPARAM, WPARAM};
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_OPTION_NON_VOLATILE,
        REG_SZ, REG_VALUE_TYPE, RegCloseKey, RegCreateKeyExW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    };
    use windows::core::{HSTRING, PCWSTR};

    pub const USER_KEY: &str = "Environment";
    const MACHINE_KEY: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";

    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: the handle came from RegOpenKeyExW / RegCreateKeyExW and is closed once, here.
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }

    /// The `Path` value under `root\sub` (`None` when the key or value is missing).
    fn read(root: HKEY, sub: &str) -> Result<Option<String>, String> {
        let mut key = HKEY::default();
        let sub = HSTRING::from(sub);
        // SAFETY: `sub` is a valid NUL-terminated wide string that outlives the call; `key` receives the handle.
        let r = unsafe { RegOpenKeyExW(root, PCWSTR(sub.as_ptr()), None, KEY_READ, &mut key) };
        if r == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        r.ok().map_err(|e| e.to_string())?;
        let key = Key(key);
        let name = HSTRING::from("Path");
        let mut kind = REG_VALUE_TYPE::default();
        let mut size = 0u32;
        // SAFETY: a size query (no buffer) on an open key with a valid value name.
        let r = unsafe { RegQueryValueExW(key.0, PCWSTR(name.as_ptr()), None, Some(&mut kind), None, Some(&mut size)) };
        if r == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        r.ok().map_err(|e| e.to_string())?;
        let mut buf = vec![0u16; (size as usize).div_ceil(2) + 1];
        let mut bytes = (buf.len() * 2) as u32;
        // SAFETY: `buf` holds `bytes` bytes, as passed; the call writes at most that many.
        let r = unsafe {
            RegQueryValueExW(
                key.0,
                PCWSTR(name.as_ptr()),
                None,
                Some(&mut kind),
                Some(buf.as_mut_ptr().cast()),
                Some(&mut bytes),
            )
        };
        r.ok().map_err(|e| e.to_string())?;
        let len = (bytes as usize / 2).min(buf.len());
        let s = String::from_utf16_lossy(&buf[..len]);
        Ok(Some(s.trim_end_matches('\0').to_string()))
    }

    /// Writes the `Path` value under `HKEY_CURRENT_USER\sub` (as REG_EXPAND_SZ, as Windows keeps it).
    fn write(sub: &str, value: &str) -> Result<(), String> {
        let mut key = HKEY::default();
        let s = HSTRING::from(sub);
        // SAFETY: `s` is a valid wide string; `key` receives the handle; no class, default security.
        unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(s.as_ptr()),
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut key,
                None,
            )
        }
        .ok()
        .map_err(|e| e.to_string())?;
        let key = Key(key);
        let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes: Vec<u8> = wide.iter().flat_map(|w| w.to_le_bytes()).collect();
        let name = HSTRING::from("Path");
        let kind = if value.contains('%') { REG_EXPAND_SZ } else { REG_SZ };
        // SAFETY: an open key, a valid value name and a byte buffer holding the NUL-terminated wide string.
        unsafe { RegSetValueExW(key.0, PCWSTR(name.as_ptr()), None, kind, Some(&bytes)) }
            .ok()
            .map_err(|e| e.to_string())
    }

    pub fn user_path(sub: &str) -> Result<String, String> {
        Ok(read(HKEY_CURRENT_USER, sub)?.unwrap_or_default())
    }

    pub fn machine_path() -> String {
        read(HKEY_LOCAL_MACHINE, MACHINE_KEY).ok().flatten().unwrap_or_default()
    }

    pub fn set_user_path(sub: &str, value: &str) -> Result<(), String> {
        write(sub, value)?;
        if sub == USER_KEY {
            let what = HSTRING::from("Environment");
            // SAFETY: a broadcast with a valid wide string as the parameter; it times out instead of hanging.
            unsafe {
                let _ = SendMessageTimeoutW(
                    HWND_BROADCAST,
                    WM_SETTINGCHANGE,
                    WPARAM(0),
                    LPARAM(what.as_ptr() as isize),
                    SMTO_ABORTIFHUNG,
                    2000,
                    None,
                );
            }
        }
        Ok(())
    }
}

fn status_now() -> CliStatus {
    let program = ove();
    let dir = program.parent().map(Path::to_path_buf).unwrap_or_default();
    #[cfg(windows)]
    {
        let user = win::user_path(win::USER_KEY).unwrap_or_default();
        let d = dir.display().to_string();
        CliStatus {
            program: program.display().to_string(),
            on_path: path_has(&user, &d) || path_has(&win::machine_path(), &d),
            can_install: program.is_file(),
            instruction: None,
        }
    }
    #[cfg(not(windows))]
    {
        let on_path = ove_agents::find_program(&["ove"], &ove_agents::Env::from_process()).is_some();
        let instruction = (!on_path && cfg!(target_os = "macos"))
            .then(|| format!("sudo ln -sf \"{}\" /usr/local/bin/ove", program.display()));
        let _ = dir;
        CliStatus { program: program.display().to_string(), on_path, can_install: false, instruction }
    }
}

/// Whether a new terminal finds `ove`, and how to get there.
#[tauri::command]
#[specta::specta]
pub async fn cli_status() -> AppResult<CliStatus> {
    worker::blocking("cli_status", || Ok(status_now())).await
}

/// Adds the folder of `ove` to the user's search path (`install`), or removes it.
#[tauri::command]
#[specta::specta]
pub async fn cli_install(install: bool) -> AppResult<CliStatus> {
    worker::blocking("cli_install", move || {
        #[cfg(windows)]
        {
            let program = ove();
            if !program.is_file() {
                return Err(AppError::new(ErrorCode::NotFound, "The ove program is not next to the app."));
            }
            let dir = program.parent().map(|d| d.display().to_string()).unwrap_or_default();
            let user = win::user_path(win::USER_KEY).map_err(|e| AppError::new(ErrorCode::Io, e))?;
            let next = if install { path_with(&user, &dir) } else { path_without(&user, &dir) };
            if next != user {
                win::set_user_path(win::USER_KEY, &next).map_err(|e| AppError::new(ErrorCode::Io, e))?;
                tracing::info!(dir = %dir, install, "command line search path changed");
            }
            Ok(status_now())
        }
        #[cfg(not(windows))]
        {
            let _ = install;
            Err(AppError::new(
                ErrorCode::InvalidArgument,
                "Run the command shown in Settings in a terminal to put ove on the search path.",
            ))
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_lists_are_edited_like_windows_compares_them() {
        let dir = r"C:\Program Files\OpenVideoEditor";
        assert_eq!(path_with("", dir), dir);
        assert_eq!(path_with(r"C:\a;C:\b", dir), format!(r"C:\a;C:\b;{dir}"));
        assert_eq!(path_with(r"C:\a;", dir), format!(r"C:\a;{dir}"));
        let with = r"C:\a;c:\program files\openvideoeditor\;%USERPROFILE%\bin".to_string();
        assert!(path_has(&with, dir), "case and a trailing separator do not matter");
        assert_eq!(path_with(&with, dir), with, "not added twice");
        assert_eq!(path_without(&with, dir), r"C:\a;%USERPROFILE%\bin", "only that entry goes");
        assert!(!path_has(r"C:\a;;C:\b", ""), "an empty entry is nothing");
    }

    /// The real registry, under a test key of its own (not the user's Path).
    #[cfg(windows)]
    #[test]
    fn the_user_path_value_round_trips_in_the_registry() {
        let sub = format!(r"Software\OpenVideoEditor-test-{}\Environment", std::process::id());
        assert_eq!(win::user_path(&sub).unwrap(), "", "missing reads as empty");
        let v = r"%USERPROFILE%\bin;C:\Tools";
        win::set_user_path(&sub, v).unwrap();
        assert_eq!(win::user_path(&sub).unwrap(), v);
        win::set_user_path(&sub, &path_with(v, r"C:\Ove")).unwrap();
        assert!(path_has(&win::user_path(&sub).unwrap(), r"c:\ove\"));
        // Clean up the test key.
        let parent = sub.trim_end_matches(r"\Environment");
        let _ = std::process::Command::new("reg").args(["delete", &format!(r"HKCU\{parent}"), "/f"]).output();
    }
}
