use super::*;

fn env(os: &'static str) -> (tempfile::TempDir, ClientEnv) {
    let d = tempfile::tempdir().unwrap();
    let e = ClientEnv {
        home: Some(d.path().join("home")),
        appdata: Some(d.path().join("roaming")),
        local_appdata: Some(d.path().join("local")),
        os,
    };
    (d, e)
}

const OVE: &str = "C:\\Program Files\\OpenVideoEditor\\ove.exe";

fn cursor() -> &'static ClientSpec {
    client("cursor").unwrap()
}

fn claude() -> &'static ClientSpec {
    client("claude-desktop").unwrap()
}

#[test]
fn the_cursor_link_carries_the_server_as_base64_json() {
    let link = install_link(cursor(), Path::new(OVE)).unwrap();
    let prefix = "cursor://anysphere.cursor-deeplink/mcp/install?name=openvideoeditor&config=";
    assert!(link.starts_with(prefix), "{link}");
    let encoded = &link[prefix.len()..];
    // URL-safe: base64's + / = are percent-encoded.
    assert!(!encoded.contains(['+', '/', '=']), "{encoded}");
    let b64 = encoded.replace("%2B", "+").replace("%2F", "/").replace("%3D", "=");
    let decoded = decode(&b64);
    let v: Value = serde_json::from_slice(&decoded).unwrap();
    assert_eq!(v, json!({"command": OVE, "args": ["mcp"]}));
    assert_eq!(install_link(claude(), Path::new(OVE)), None);
}

fn decode(s: &str) -> Vec<u8> {
    const T: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let bits: Vec<u32> = s.bytes().filter(|b| *b != b'=').map(|b| T.find(b as char).unwrap() as u32).collect();
    for c in bits.chunks(4) {
        let n = c.iter().enumerate().fold(0u32, |n, (i, v)| n | v << (18 - 6 * i));
        out.push((n >> 16) as u8);
        if c.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if c.len() > 3 {
            out.push(n as u8);
        }
    }
    out
}

#[test]
fn base64_pads() {
    assert_eq!(base64(b"f"), "Zg==");
    assert_eq!(base64(b"foobar"), "Zm9vYmFy");
}

#[test]
fn the_snippet_is_the_mcp_servers_object() {
    let v: Value = serde_json::from_str(&snippet(Path::new(OVE))).unwrap();
    assert_eq!(v, json!({"mcpServers": {"openvideoeditor": {"command": OVE, "args": ["mcp"]}}}));
}

#[test]
fn status_parsing_finds_the_editors_server_by_name_or_command() {
    assert!(config_has_server(r#"{"mcpServers": {"openvideoeditor": {"command": "x"}}}"#));
    assert!(config_has_server(
        r#"{"mcpServers": {"video": {"command": "/Applications/OpenVideoEditor.app/Contents/MacOS/ove", "args": ["mcp"]}}}"#
    ));
    assert!(config_has_server(
        r#"{"mcpServers": {"v": {"command": "C:\\x\\OVE.EXE", "args": ["mcp", "--project", "p"]}}}"#
    ));
    assert!(!config_has_server(r#"{"mcpServers": {"other": {"command": "node", "args": ["server.js"]}}}"#));
    assert!(!config_has_server(r#"{"mcpServers": {"v": {"command": "ove", "args": ["docs"]}}}"#));
    assert!(!config_has_server("{}"));
    assert!(!config_has_server("not json"));
}

#[test]
fn cursor_status_follows_its_folder_and_config() {
    let (_d, e) = env("windows");
    let s = status(cursor(), &e, Path::new(OVE));
    assert!(!s.installed && !s.connected);
    let dir = e.home.as_ref().unwrap().join(".cursor");
    std::fs::create_dir_all(&dir).unwrap();
    let s = status(cursor(), &e, Path::new(OVE));
    assert!(s.installed && !s.connected);
    assert_eq!(s.config_file, Some(dir.join("mcp.json")));
    std::fs::write(dir.join("mcp.json"), snippet(Path::new(OVE))).unwrap();
    assert!(status(cursor(), &e, Path::new(OVE)).connected);
}

#[test]
fn claude_desktop_status_on_windows_and_macos() {
    let (_d, e) = env("windows");
    let s = status(claude(), &e, Path::new(OVE));
    assert!(!s.installed && !s.connected);
    assert!(s.link.is_none());
    let dir = e.appdata.as_ref().unwrap().join("Claude");
    std::fs::create_dir_all(&dir).unwrap();
    assert!(status(claude(), &e, Path::new(OVE)).installed);
    // Connected by the manual configuration...
    std::fs::write(dir.join("claude_desktop_config.json"), snippet(Path::new(OVE))).unwrap();
    assert!(status(claude(), &e, Path::new(OVE)).connected);
    std::fs::remove_file(dir.join("claude_desktop_config.json")).unwrap();
    // ...or by the installed extension.
    let ext = dir.join("Claude Extensions").join("local.mcpb.openvideoeditor");
    std::fs::create_dir_all(&ext).unwrap();
    std::fs::write(ext.join("manifest.json"), mcpb_manifest("win32", "1.0.0").to_string()).unwrap();
    assert!(status(claude(), &e, Path::new(OVE)).connected);

    let (_d2, m) = env("macos");
    let s = status(claude(), &m, Path::new("/Applications/OpenVideoEditor.app/Contents/MacOS/ove"));
    assert_eq!(
        s.config_file,
        Some(m.home.as_ref().unwrap().join("Library/Application Support/Claude").join("claude_desktop_config.json"))
    );
}

#[test]
fn disconnect_removes_only_the_editors_entry_and_keeps_a_copy() {
    let (_d, e) = env("windows");
    let dir = e.home.as_ref().unwrap().join(".cursor");
    std::fs::create_dir_all(&dir).unwrap();
    let before = json!({"mcpServers": {
        "openvideoeditor": {"command": OVE, "args": ["mcp"]},
        "other": {"command": "node", "args": ["s.js"]}
    }, "theme": "dark"});
    std::fs::write(dir.join("mcp.json"), before.to_string()).unwrap();
    assert!(disconnect(cursor(), &e).unwrap());
    let after: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("mcp.json")).unwrap()).unwrap();
    assert_eq!(after, json!({"mcpServers": {"other": {"command": "node", "args": ["s.js"]}}, "theme": "dark"}));
    let backup: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("mcp.json.bak")).unwrap()).unwrap();
    assert_eq!(backup, before);
    assert!(!disconnect(cursor(), &e).unwrap(), "nothing left to remove");
    // A broken file is left alone.
    std::fs::write(dir.join("mcp.json"), "{oops").unwrap();
    assert!(disconnect(cursor(), &e).is_err());
    assert_eq!(std::fs::read_to_string(dir.join("mcp.json")).unwrap(), "{oops");
}

#[test]
fn the_extension_manifest_matches_the_packer() {
    for p in ["win32", "darwin"] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../tools/mcpb/manifest-{p}.json"));
        let packed: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(packed, mcpb_manifest(p, "0.0.0"), "{p}: tools/mcpb/pack.mjs and mcpb_manifest differ");
    }
}
