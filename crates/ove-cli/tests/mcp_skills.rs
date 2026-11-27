//! Skills over `ove mcp` (add-agent-skills-and-connectors tasks 1.2 and 1.4): `list_skills` / `read_skill` as tools,
//! and every valid skill as a prompt and an `ove://skills/<id>` resource. `prompts/list` and `resources/list` are
//! pinned by golden files and must match the store.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use serde_json::{Value, json};

const TIDY: &str =
    "---\nname: Tidy cuts\ndescription: Close small gaps on the main track.\n---\n\nPrint, then close each gap.\n";

/// A library with one project and a skill store holding one valid skill and one without a description.
fn setup() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    library.create("P").unwrap().close().unwrap();
    let store = ove_tools::skills::SkillStore::of_library(&lib);
    for (id, text) in [("tidy-cuts", TIDY), ("half-done", "---\nname: Half\n---\nbody\n")] {
        std::fs::create_dir_all(store.dir(id)).unwrap();
        std::fs::write(store.dir(id).join("SKILL.md"), text).unwrap();
    }
    (d, lib)
}

struct Mcp {
    child: std::process::Child,
    out: BufReader<std::process::ChildStdout>,
    id: i64,
}

impl Mcp {
    fn start(lib: &Path) -> Mcp {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ove"))
            .arg("--library")
            .arg(lib)
            .arg("mcp")
            .env_remove("OVE_LIBRARY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        Mcp { child, out, id: 0 }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{}", json!({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params})).unwrap();
        stdin.flush().unwrap();
        let mut reply = String::new();
        self.out.read_line(&mut reply).unwrap();
        serde_json::from_str(&reply).unwrap_or_else(|e| panic!("{e}: {reply}"))
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn golden(name: &str, v: &Value) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name);
    let text = serde_json::to_string_pretty(v).unwrap() + "\n";
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &text).unwrap();
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} is missing; run with UPDATE_GOLDEN=1", path.display()))
        .replace("\r\n", "\n");
    assert_eq!(text, want, "{name} changed (UPDATE_GOLDEN=1 rewrites it)");
}

#[test]
fn skills_are_prompts_and_resources_that_match_the_store() {
    let (_d, lib) = setup();
    let mut mcp = Mcp::start(&lib);
    let init = mcp.request(
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test"}}),
    );
    let caps = &init["result"]["capabilities"];
    assert!(caps["prompts"].is_object() && caps["resources"].is_object(), "{init}");

    let prompts = mcp.request("prompts/list", json!({}))["result"].clone();
    golden("skills-prompts.json", &prompts);
    let resources = mcp.request("resources/list", json!({}))["result"].clone();
    golden("skills-resources.json", &resources);

    // Both lists match the store's valid skills, in order.
    let store = ove_tools::skills::SkillStore::of_library(&lib);
    let ids: Vec<String> = store.valid().into_iter().map(|s| s.id).collect();
    let names: Vec<&str> = prompts["prompts"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ids);
    let uris: Vec<&str> =
        resources["resources"].as_array().unwrap().iter().map(|r| r["uri"].as_str().unwrap()).collect();
    assert_eq!(uris, ids.iter().map(|i| format!("ove://skills/{i}")).collect::<Vec<_>>());

    let got = mcp.request("prompts/get", json!({"name": "tidy-cuts"}));
    let text = got["result"]["messages"][0]["content"]["text"].as_str().unwrap();
    assert!(text.contains("Print, then close each gap."), "{got}");
    assert_eq!(got["result"]["messages"][0]["role"], "user");
    let read = mcp.request("resources/read", json!({"uri": "ove://skills/tidy-cuts"}));
    assert_eq!(read["result"]["contents"][0]["text"], TIDY);

    let missing = mcp.request("prompts/get", json!({"name": "nope"}));
    assert_eq!(missing["error"]["data"]["code"], "skill.unknown", "{missing}");
    assert_eq!(missing["error"]["data"]["details"]["installed"][3], "tidy-cuts");
    let invalid = mcp.request("resources/read", json!({"uri": "ove://skills/half-done"}));
    assert_eq!(invalid["error"]["data"]["code"], "skill.invalid", "{invalid}");
    let other = mcp.request("resources/read", json!({"uri": "file:///etc/hosts"}));
    assert_eq!(other["error"]["data"]["code"], "resource.unknown", "{other}");

    // The same skills as tools.
    let listed = mcp.request("tools/call", json!({"name": "list_skills", "arguments": {}}));
    assert_eq!(listed["result"]["structuredContent"]["count"], 4, "{listed}");
    let unknown = mcp.request("tools/call", json!({"name": "read_skill", "arguments": {"id": "nope"}}));
    assert_eq!(unknown["result"]["isError"], true);
    assert_eq!(unknown["result"]["structuredContent"]["error"]["code"], "skill.unknown");
}

#[test]
fn a_skill_added_while_the_session_runs_is_listed_at_once() {
    let (_d, lib) = setup();
    let mut mcp = Mcp::start(&lib);
    let count = |m: &mut Mcp| m.request("prompts/list", json!({}))["result"]["prompts"].as_array().unwrap().len();
    assert_eq!(count(&mut mcp), 4);
    let store = ove_tools::skills::SkillStore::of_library(&lib);
    std::fs::create_dir_all(store.dir("late")).unwrap();
    std::fs::write(store.dir("late").join("SKILL.md"), TIDY.replace("Tidy cuts", "Late")).unwrap();
    assert_eq!(count(&mut mcp), 5);
}
