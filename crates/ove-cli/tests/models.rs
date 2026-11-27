//! `ove models`: the examples in docs/cli/models.md with their documented exit codes, the JSON shapes, and the
//! success and refusal paths.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

/// Runs `ove` with the model store at `store` and no other model settings from the environment.
fn ove(store: &Path, extra_env: &[(&str, &str)], args: &[&str]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ove"));
    c.args(args)
        .env("OVE_MODELS_DIR", store)
        .env_remove("OVE_WHISPER_MODEL_DIR")
        .env_remove("OVE_WHISPER_MODEL_URL")
        .env_remove("OVE_MODEL_BASE_URL");
    for (k, v) in extra_env {
        c.env(k, v);
    }
    c.output().unwrap()
}

fn json_err(out: &Output) -> Value {
    serde_json::from_slice(&out.stderr).unwrap_or_else(|_| panic!("stderr: {}", String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn documented_examples_run_with_documented_exit_codes() {
    let store = tempfile::tempdir().unwrap();
    let doc = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/cli/models.md")).unwrap();
    let mut in_sh = false;
    let mut ran = 0;
    for line in doc.lines() {
        if line.starts_with("```") {
            in_sh = line == "```sh";
            continue;
        }
        if !in_sh || !line.starts_with("ove ") || !line.contains("# exit") {
            continue;
        }
        let (cmd, comment) = line.split_once('#').unwrap();
        let want: i32 = comment.trim().strip_prefix("exit ").unwrap().parse().unwrap();
        let args: Vec<&str> = cmd.split_whitespace().skip(1).collect();
        let out = ove(store.path(), &[], &args);
        assert_eq!(out.status.code(), Some(want), "`{}`: {}", cmd.trim(), String::from_utf8_lossy(&out.stderr));
        if args.contains(&"--json") && want == 0 {
            serde_json::from_slice::<Value>(&out.stdout).expect("one JSON object on stdout");
        }
        ran += 1;
    }
    assert_eq!(ran, 7);
    // Nothing was downloaded by any of them.
    assert!(!store.path().join("whisper-base").exists());
}

#[test]
fn list_reports_every_model_with_its_size_and_state() {
    let store = tempfile::tempdir().unwrap();
    let out = ove(store.path(), &[], &["models", "list", "--json"]);
    assert!(out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["root"].as_str().map(PathBuf::from), Some(store.path().to_path_buf()));
    assert_eq!(v["installedBytes"], 0);
    let models = v["models"].as_array().unwrap();
    let base = models.iter().find(|m| m["id"] == "whisper-base").unwrap();
    for k in ["id", "version", "purpose", "licence", "runtime", "inProcess", "bytes", "installed", "location"] {
        assert!(base.get(k).is_some(), "missing {k}: {base}");
    }
    assert_eq!(base["bytes"], 147_951_465);
    assert_eq!(base["installed"], false);
    assert_eq!(base["licence"], "MIT");
    // Text output names each model.
    let text = String::from_utf8(ove(store.path(), &[], &["models", "list"]).stdout).unwrap();
    assert!(text.contains("whisper-small") && text.contains("not installed"), "{text}");
}

#[test]
fn a_large_model_needs_consent() {
    let store = tempfile::tempdir().unwrap();
    let out = ove(store.path(), &[], &["models", "install", "whisper-base", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let e = json_err(&out);
    assert_eq!(e["error"]["code"], "model.consent");
    assert_eq!(e["error"]["details"]["bytes"], 147_951_465);
    assert_eq!(e["error"]["details"]["install"], "ove models install whisper-base --yes");
    assert!(e["error"]["message"].as_str().unwrap().contains("148.0 MB"), "{e}");
    let text = ove(store.path(), &[], &["models", "install", "whisper-base"]);
    assert!(String::from_utf8_lossy(&text.stderr).contains("ove models install whisper-base --yes"));
}

#[test]
fn unknown_and_not_installed_are_refused() {
    let store = tempfile::tempdir().unwrap();
    let out = ove(store.path(), &[], &["models", "install", "nope", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(json_err(&out)["error"]["code"], "model.unknown");
    let out = ove(store.path(), &[], &["models", "remove", "whisper-tiny", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(json_err(&out)["error"]["code"], "model.notInstalled");
    let out = ove(store.path(), &[], &["models", "remove", "nope", "--json"]);
    assert_eq!(json_err(&out)["error"]["code"], "model.unknown");
}

/// Serves one file at every path, for a mirror download.
fn serve_file(file: PathBuf) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let body = std::fs::read(&file).unwrap();
        for mut s in listener.incoming().flatten() {
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
            while r.read_line(&mut line).is_ok_and(|n| n > 0) && line != "\r\n" {
                line.clear();
            }
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = s.write_all(head.as_bytes()).and_then(|_| s.write_all(&body));
        }
    });
    base
}

/// Success paths with the real tiny speech model (77 MB), taken from `OVE_WHISPER_MODEL_DIR` and served from a local
/// mirror so the test does not use the network.
#[test]
#[ignore = "needs ggml-tiny.bin in OVE_WHISPER_MODEL_DIR"]
fn install_list_and_remove_with_the_tiny_speech_model() {
    let cache = PathBuf::from(std::env::var_os("OVE_WHISPER_MODEL_DIR").expect("set OVE_WHISPER_MODEL_DIR"));
    let tiny = [cache.join("ggml-tiny.bin"), cache.join("whisper-tiny/ggml-1/ggml-tiny.bin")]
        .into_iter()
        .find(|p| p.exists())
        .expect("ggml-tiny.bin in OVE_WHISPER_MODEL_DIR");
    let base = serve_file(tiny.clone());
    let store = tempfile::tempdir().unwrap();
    let mirror = [("OVE_MODEL_BASE_URL", base.as_str())];

    let out = ove(store.path(), &mirror, &["models", "install", "whisper-tiny", "--yes", "--json"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!((v["installed"].as_bool(), v["already"].as_bool()), (Some(true), Some(false)));
    let path = store.path().join("whisper-tiny/ggml-1/ggml-tiny.bin");
    assert_eq!(v["paths"][0].as_str().map(PathBuf::from), Some(path.clone()));
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 77_691_713);

    let again = ove(store.path(), &mirror, &["models", "install", "whisper-tiny", "--yes", "--json"]);
    let v: Value = serde_json::from_slice(&again.stdout).unwrap();
    assert_eq!(v["already"], true);

    let list: Value = serde_json::from_slice(&ove(store.path(), &[], &["models", "list", "--json"]).stdout).unwrap();
    let t = list["models"].as_array().unwrap().iter().find(|m| m["id"] == "whisper-tiny").unwrap().clone();
    assert_eq!(t["installed"], true);
    assert_eq!(t["location"].as_str().map(PathBuf::from), Some(store.path().join("whisper-tiny/ggml-1")));
    assert_eq!(list["installedBytes"], 77_691_713);

    let out = ove(store.path(), &[], &["models", "remove", "whisper-tiny", "--json"]);
    assert!(out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["freedBytes"], 77_691_713);
    assert!(v["freeBytes"].as_u64().unwrap() > 0);
    assert!(!path.exists());

    // A file saved by an earlier version in the store folder is listed as installed without downloading.
    std::fs::copy(&tiny, store.path().join("ggml-tiny.bin")).unwrap();
    let list: Value = serde_json::from_slice(&ove(store.path(), &[], &["models", "list", "--json"]).stdout).unwrap();
    let t = list["models"].as_array().unwrap().iter().find(|m| m["id"] == "whisper-tiny").unwrap().clone();
    assert_eq!(t["installed"], true);
    assert_eq!(t["location"].as_str().map(PathBuf::from), Some(store.path().to_path_buf()));
}

#[test]
fn mcp_lists_the_local_models() {
    use std::io::Read;
    let store = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ove"))
        .arg("mcp")
        .env("OVE_MODELS_DIR", store.path())
        .env_remove("OVE_WHISPER_MODEL_DIR")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let msgs = [
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}}),
        serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
        serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "list_models_local", "arguments": {}}}),
        serde_json::json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "list_models_local", "arguments": {"id": "x"}}}),
    ];
    for m in &msgs {
        writeln!(stdin, "{m}").unwrap();
    }
    drop(stdin);
    let mut out = String::new();
    child.stdout.take().unwrap().read_to_string(&mut out).unwrap();
    child.wait().unwrap();
    let replies: Vec<Value> = out.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let by_id = |id: i64| replies.iter().find(|r| r["id"] == id).unwrap_or_else(|| panic!("no reply {id}: {out}"));

    let tools = by_id(2)["result"]["tools"].as_array().unwrap();
    let tool = tools.iter().find(|t| t["name"] == "list_models_local").expect("list_models_local is listed");
    assert_eq!(tool["inputSchema"]["type"], "object");
    assert_eq!(tool["annotations"]["readOnlyHint"], true, "{tool}");

    let call = by_id(3)["result"].clone();
    assert_eq!(call["isError"], false, "{call}");
    let models = call["structuredContent"]["models"].as_array().unwrap();
    let ids: Vec<&str> = models.iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids[..3], ["whisper-tiny", "whisper-base", "whisper-small"]);
    // The inference runtime is listed on the platforms it is pinned for.
    let runtime = ove_models::manifest::RuntimePlatform::current().is_some();
    assert_eq!(ids[3..5], ["silero-vad", "siglip2-base"]);
    assert_eq!(ids.len(), if runtime { 6 } else { 5 }, "{ids:?}");
    if runtime {
        assert_eq!(ids[5], "onnxruntime");
        assert_eq!(models[5]["host"], "files.pythonhosted.org");
    }
    assert!(models.iter().all(|m| m["installed"] == false && m["bytes"].as_u64().unwrap() > 0));

    // Unknown arguments are refused as an error result, as for every tool.
    assert_eq!(by_id(4)["result"]["isError"], true);
}
