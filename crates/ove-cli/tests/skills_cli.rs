//! `ove skills catalog | install | update | export` (add-agent-skills-and-connectors tasks 2.2–2.4) against a local
//! catalog server and a temporary home folder. Nothing installs without `--yes`; tampered content is refused.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

struct Server {
    url: String,
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    hits: Arc<AtomicUsize>,
}

impl Server {
    fn start() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let files: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::default();
        let hits = Arc::new(AtomicUsize::new(0));
        let (f, h) = (files.clone(), hits.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                h.fetch_add(1, Ordering::SeqCst);
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                let _ = reader.read_line(&mut first);
                loop {
                    let mut l = String::new();
                    if reader.read_line(&mut l).unwrap_or(0) == 0 || l.trim().is_empty() {
                        break;
                    }
                }
                let path = first.split_whitespace().nth(1).unwrap_or("/").to_string();
                let body = f.lock().unwrap().get(&path).cloned();
                let mut s = stream;
                let _ = match body {
                    Some(b) => {
                        let _ =
                            write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", b.len());
                        s.write_all(&b)
                    }
                    None => write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                };
            }
        });
        Server { url, files, hits }
    }

    fn publish(&self, skills: &[(&str, &str)]) {
        let mut files = self.files.lock().unwrap();
        let entries: Vec<Value> = skills
            .iter()
            .map(|(id, text)| {
                files.insert(format!("/c/{id}.md"), text.as_bytes().to_vec());
                let sha = ove_tools::skills::sha256_hex(text.as_bytes());
                json!({"id": id, "name": id, "description": "A test skill", "sha256": sha, "path": format!("{id}.md")})
            })
            .collect();
        files.insert("/c/catalog.json".into(), json!({"skills": entries}).to_string().into_bytes());
    }

    fn catalog(&self) -> String {
        format!("{}/c/catalog.json", self.url)
    }
}

const V1: &str = "---\nname: Pacing\ndescription: Keep a steady pace.\n---\n\nCut on the beat.\n";
const V2: &str =
    "---\nname: Pacing\ndescription: Keep a steady pace.\n---\n\nCut on the beat.\nHold wide shots longer.\n";

struct Env {
    _d: tempfile::TempDir,
    lib: PathBuf,
    home: PathBuf,
}

fn env() -> Env {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let home = d.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    Env { lib, home, _d: d }
}

fn ove(e: &Env, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ove"))
        .arg("--library")
        .arg(&e.lib)
        .args(args)
        .arg("--json")
        .env_remove("OVE_LIBRARY")
        .env_remove("OVE_SKILLS_CATALOG")
        .env("HOME", &e.home)
        .env("USERPROFILE", &e.home)
        .output()
        .unwrap()
}

fn ok(out: &Output) -> Value {
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

fn refused(out: &Output) -> Value {
    assert_eq!(out.status.code(), Some(1), "stdout: {}", String::from_utf8_lossy(&out.stdout));
    serde_json::from_slice::<Value>(&out.stderr).unwrap()["error"].clone()
}

fn skill_file(e: &Env, id: &str) -> PathBuf {
    e.lib.join(".skills").join(id).join("SKILL.md")
}

#[test]
fn without_a_catalog_nothing_is_fetched() {
    let e = env();
    let err = refused(&ove(&e, &["skills", "catalog"]));
    assert_eq!(err["code"], "catalog.unset");
    assert_eq!(refused(&ove(&e, &["skills", "install", "pacing", "--yes"]))["code"], "catalog.unset");
    assert_eq!(refused(&ove(&e, &["skills", "update"]))["code"], "catalog.unset");
    // Only HTTPS addresses (and this computer) can be set.
    assert_eq!(refused(&ove(&e, &["skills", "catalog", "--set", "http://example.org/c.json"]))["code"], "catalog.url");
}

#[test]
fn install_needs_confirmation_and_checks_the_pin() {
    let e = env();
    let server = Server::start();
    server.publish(&[("pacing", V1)]);
    ok(&ove(&e, &["skills", "catalog", "--set", &server.catalog()]));
    let listed = ok(&ove(&e, &["skills", "catalog"]));
    assert_eq!(listed["catalog"]["entries"][0]["id"], "pacing");

    // Without --yes: shown, verified, not installed.
    let shown = refused(&ove(&e, &["skills", "install", "pacing"]));
    assert_eq!(shown["code"], "skill.confirm");
    assert_eq!(shown["details"]["text"], V1);
    assert!(shown["message"].as_str().unwrap().contains("matches its pinned SHA-256"));
    assert!(!skill_file(&e, "pacing").exists());

    let installed = ok(&ove(&e, &["skills", "install", "pacing", "--yes"]));
    assert_eq!(installed["origin"]["kind"], "catalog");
    assert_eq!(installed["origin"]["url"], server.catalog());
    assert_eq!(std::fs::read_to_string(skill_file(&e, "pacing")).unwrap(), V1);
    let list = ok(&ove(&e, &["skills", "list"]));
    let row = list["skills"].as_array().unwrap().iter().find(|s| s["id"] == "pacing").unwrap().clone();
    assert_eq!(row["origin"]["url"], server.catalog());

    // Tampered content: nothing installed.
    server.files.lock().unwrap().insert("/c/pacing.md".into(), b"---\nname: P\ndescription: d\n---\nevil\n".to_vec());
    let err = refused(&ove(&e, &["skills", "install", "pacing", "--as", "pacing-2", "--yes"]));
    assert_eq!(err["code"], "skill.tampered");
    assert!(!skill_file(&e, "pacing-2").exists());
    assert_eq!(refused(&ove(&e, &["skills", "install", "nope", "--yes"]))["code"], "catalog.missing");
    assert!(server.hits.load(Ordering::SeqCst) > 0);
}

#[test]
fn updates_are_shown_then_installed_and_edits_are_kept() {
    let e = env();
    let server = Server::start();
    server.publish(&[("pacing", V1), ("mine", V1)]);
    ok(&ove(&e, &["skills", "catalog", "--set", &server.catalog()]));
    ok(&ove(&e, &["skills", "install", "pacing", "--yes"]));
    ok(&ove(&e, &["skills", "install", "mine", "--yes"]));
    let edited = V1.replace("beat", "downbeat");
    std::fs::write(skill_file(&e, "mine"), &edited).unwrap();
    server.publish(&[("pacing", V2), ("mine", V2)]);

    let states = ok(&ove(&e, &["skills", "update"]));
    let by_id: HashMap<String, String> = states["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["id"].as_str().unwrap().to_string(), s["state"].as_str().unwrap().to_string()))
        .collect();
    assert_eq!(by_id["pacing"], "available");
    assert_eq!(by_id["mine"], "modified");

    let shown = refused(&ove(&e, &["skills", "update", "pacing"]));
    assert_eq!(shown["code"], "skill.confirm");
    assert!(shown["message"].as_str().unwrap().contains("+Hold wide shots longer."));
    assert_eq!(std::fs::read_to_string(skill_file(&e, "pacing")).unwrap(), V1);
    ok(&ove(&e, &["skills", "update", "pacing", "--yes"]));
    assert_eq!(std::fs::read_to_string(skill_file(&e, "pacing")).unwrap(), V2);

    let copy = ok(&ove(&e, &["skills", "update", "mine", "--yes"]));
    assert_eq!(copy["id"], "mine-update");
    assert_eq!(std::fs::read_to_string(skill_file(&e, "mine")).unwrap(), edited);
    assert_eq!(std::fs::read_to_string(skill_file(&e, "mine-update")).unwrap(), V2);
}

fn exported(home: &Path, agent: &str, id: &str) -> PathBuf {
    home.join(agent).join("skills").join(format!("ove-{id}")).join("SKILL.md")
}

#[test]
fn export_goes_to_the_installed_agents_and_keeps_their_edits() {
    let e = env();
    std::fs::create_dir_all(e.home.join(".claude")).unwrap();
    std::fs::create_dir_all(e.home.join(".codex")).unwrap();
    let out = ok(&ove(&e, &["skills", "export", "interview-cutting"]));
    let targets: Vec<&str> =
        out["exported"].as_array().unwrap().iter().map(|x| x["target"].as_str().unwrap()).collect();
    assert_eq!(targets, ["claude", "codex"]);
    let text = std::fs::read_to_string(exported(&e.home, ".claude", "interview-cutting")).unwrap();
    assert!(text.starts_with("---\nname: ove-interview-cutting\n"), "{text}");
    assert!(exported(&e.home, ".codex", "interview-cutting").is_file());
    // Cursor is not installed: named with the reason.
    let cursor = out["targets"].as_array().unwrap().iter().find(|t| t["id"] == "cursor").unwrap().clone();
    assert_eq!(cursor["available"], false);
    assert!(cursor["reason"].as_str().unwrap().contains("not installed"));
    assert_eq!(
        refused(&ove(&e, &["skills", "export", "interview-cutting", "--to", "cursor"]))["code"],
        "export.unavailable"
    );
    assert_eq!(refused(&ove(&e, &["skills", "export", "interview-cutting", "--to", "nope"]))["code"], "export.target");

    // An edit in Claude's folder is kept unless confirmed.
    std::fs::write(exported(&e.home, ".claude", "interview-cutting"), "mine").unwrap();
    let err = refused(&ove(&e, &["skills", "export", "interview-cutting", "--to", "claude"]));
    assert_eq!(err["code"], "export.modified");
    assert_eq!(std::fs::read_to_string(exported(&e.home, ".claude", "interview-cutting")).unwrap(), "mine");
    ok(&ove(&e, &["skills", "export", "interview-cutting", "--to", "claude", "--yes"]));
    assert_ne!(std::fs::read_to_string(exported(&e.home, ".claude", "interview-cutting")).unwrap(), "mine");
}

#[test]
fn export_with_no_agent_installed_says_why() {
    let e = env();
    let err = refused(&ove(&e, &["skills", "export", "caption-styling"]));
    assert_eq!(err["code"], "export.unavailable");
    assert!(err["message"].as_str().unwrap().contains("Cursor is not installed"));
}
