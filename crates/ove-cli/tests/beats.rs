//! add-audio-intelligence 3.2–3.3: `ove audio beats` and the `detect_beats` MCP contract. The cached-result and
//! "model not installed" checks need no model. The model checks (a 120 BPM click track and three-four time) run when
//! `OVE_BEAT_MODEL_DIR` holds the two pinned ONNX files and `OVE_ONNXRUNTIME_LIB` names the runtime library;
//! otherwise they print "skipped".
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]
#![cfg(feature = "media")]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;

use ove_analyze::beats::{self, BeatAnalysis};
use serde_json::{Value, json};

fn ove(lib: &Path, models: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ove"))
        .arg("--library")
        .arg(lib)
        .args(args)
        .env_remove("OVE_LIBRARY")
        .env("OVE_MODELS_DIR", models)
        .output()
        .unwrap()
}

fn json_out(o: &Output) -> Value {
    serde_json::from_slice(&o.stdout).unwrap_or_else(|_| {
        panic!("stdout: {}\nstderr: {}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
    })
}

/// 48 kHz mono 16-bit WAV of clicks at `bpm`, accented every `bar` beats.
fn clicks(path: &Path, bpm: f64, bar: usize, seconds: f64) {
    let rate = 48_000usize;
    let n = (seconds * rate as f64) as usize;
    let mut x = vec![0.0f32; n];
    let period = 60.0 / bpm;
    let mut k = 0usize;
    while (k as f64) * period < seconds - 0.1 {
        let a = ((k as f64) * period * rate as f64) as usize;
        let down = k.is_multiple_of(bar);
        let f = if down { 1_500.0 } else { 900.0 };
        for i in 0..(0.05 * rate as f64) as usize {
            if a + i >= n {
                break;
            }
            let t = i as f64 / rate as f64;
            let env = (-t / 0.012).exp();
            let mut v = (if down { 0.8 } else { 0.45 }) * env * (std::f64::consts::TAU * f * t).sin();
            if down {
                v += 0.6 * env * (std::f64::consts::TAU * 60.0 * t).sin();
            }
            x[a + i] += v as f32;
        }
        k += 1;
    }
    write_wav(path, &x);
}

fn write_wav(path: &Path, x: &[f32]) {
    let rate = 48_000usize;
    let data: Vec<u8> = x.iter().flat_map(|v| ((v.clamp(-1.0, 1.0) * 32_767.0) as i16).to_le_bytes()).collect();
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    for v in [16u32.to_le_bytes().to_vec(), 1u16.to_le_bytes().to_vec(), 1u16.to_le_bytes().to_vec()] {
        out.extend(v);
    }
    out.extend_from_slice(&(rate as u32).to_le_bytes());
    out.extend_from_slice(&(rate as u32 * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend(data);
    std::fs::write(path, out).unwrap();
}

/// A waltz: a bass note and a kick on each bar's first beat, a quieter chord and a hat on beats two and three, and a
/// new chord every bar (I, IV, V, I), so the bar is heard from the harmony as music has it.
fn waltz(path: &Path, bpm: f64, seconds: f64) {
    let rate = 48_000usize;
    let n = (seconds * rate as f64) as usize;
    let mut x = vec![0.0f32; n];
    let beat = 60.0 / bpm;
    let roots = [110.0, 146.83, 164.81, 110.0];
    let mut k = 0usize;
    let mut seed = 7u64;
    while (k as f64) * beat < seconds - 0.1 {
        let a = ((k as f64) * beat * rate as f64) as usize;
        let bar = k / 3;
        let root = roots[bar % 4];
        let first = k.is_multiple_of(3);
        let len = (beat * rate as f64) as usize;
        for i in 0..len {
            if a + i >= n {
                break;
            }
            let t = i as f64 / rate as f64;
            let tau = std::f64::consts::TAU;
            let mut v = 0.0;
            if first {
                let env = (-t / 0.25).exp();
                v += 0.5 * env * ((tau * root * t).sin() + 0.5 * (tau * 2.0 * root * t).sin());
                v += 0.7 * (-t / 0.03).exp() * (tau * (50.0 + 60.0 * (-t / 0.02).exp()) * t).sin();
            } else {
                let env = (-t / 0.12).exp();
                for m in [2.0, 2.5198, 2.9966] {
                    v += 0.12 * env * (tau * root * 2.0 * m * t).sin();
                }
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let noise = (seed % 2001) as f64 / 1000.0 - 1.0;
                v += 0.08 * (-t / 0.01).exp() * noise;
            }
            x[a + i] += v as f32;
        }
        k += 1;
    }
    write_wav(path, &x);
}

/// Project "Song": music media `m_<name>` per file, each with an item `<name>` on its own track.
fn setup(dir: &Path, files: &[(&str, &Path, f64, Value)]) -> PathBuf {
    let lib = dir.join("lib");
    let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = library.create("Song").unwrap();
    let mut ops = vec![];
    for (i, (name, path, secs, extra)) in files.iter().enumerate() {
        ops.push(
            json!({"op": "addTrack", "track": {"id": format!("t_{name}"), "name": name, "order": i, "type": "audio"}}),
        );
        let mut entry =
            json!({"mediaId": format!("m_{name}"), "nativePath": path.display().to_string(), "ownedByBundle": false});
        entry.as_object_mut().unwrap().extend(ove_store::media::identity_fields(path));
        ops.push(json!({"op": "addMedia", "media": {"id": format!("m_{name}"), "data": {"type": "audio", "fileName": format!("{name}.wav"), "duration": secs, "hasAudio": true}}, "entry": entry}));
        let mut item = json!({"id": name, "trackId": format!("t_{name}"), "type": "audio", "startTime": 0, "endTime": secs, "mediaItemId": format!("m_{name}")});
        if let Value::Object(x) = extra {
            for (k, v) in x {
                item[k] = v.clone();
            }
        }
        ops.push(json!({"op": "addItem", "item": item}));
    }
    let ops: Vec<ove_core::Op> = serde_json::from_value(Value::Array(ops)).unwrap();
    p.apply(ops, ove_core::Source::Gui, None).unwrap();
    p.close().unwrap();
    lib
}

fn content_id(path: &Path) -> String {
    ove_store::media::identity_fields(path).get("contentId").unwrap().as_str().unwrap().to_string()
}

#[test]
fn cached_beats_map_to_the_timeline_and_a_missing_model_is_reported() {
    let d = tempfile::tempdir().unwrap();
    let wav = d.path().join("music.wav");
    clicks(&wav, 120.0, 4, 6.0);
    // The item plays its source at 2× from source second 1, starting at timeline second 10.
    let lib = setup(
        d.path(),
        &[("music", &wav, 6.0, json!({"startTime": 10, "endTime": 12, "trimStart": 1, "playbackSpeed": 2}))],
    );
    let models = d.path().join("models");
    // No model, no cache: the status says so and nothing is downloaded.
    let o = ove(&lib, &models, &["audio", "beats", "Song", "music", "--json"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let v = json_out(&o);
    assert_eq!(v["status"], "modelNotInstalled", "{v}");
    assert!(v["bytes"].as_u64().unwrap() > 10_000_000);
    assert!(!models.join(beats::MODEL_ID).exists(), "nothing was downloaded");

    // A cached analysis answers without the model.
    let src: Vec<f64> = (0..12).map(|k| k as f64 * 0.5).collect();
    let downs: Vec<f64> = (0..3).map(|k| k as f64 * 2.0).collect();
    let a: BeatAnalysis = beats::analyse(&src, &downs, beats::MODEL_ID);
    beats::store(&lib.join("Song.ove"), &content_id(&wav), &a).unwrap();
    let o = ove(&lib, &models, &["audio", "beats", "Song", "music", "--json"]);
    let v = json_out(&o);
    // Golden: source beats as cached; timeline beats every 0.25 s from second 10 (source 1.0 → timeline 10.0).
    assert_eq!(v["status"], "ready");
    assert_eq!(v["cached"], true);
    assert_eq!((v["meter"].as_u64(), v["bpm"].as_f64()), (Some(4), Some(120.0)));
    assert_eq!(v["timelineBpm"].as_f64(), Some(240.0));
    let tl: Vec<f64> = v["timelineBeats"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
    assert_eq!(tl, vec![10.0, 10.25, 10.5, 10.75, 11.0, 11.25, 11.5, 11.75]);
    assert_eq!(v["timelineDownbeats"], json!([10.5, 11.5]));
    let media =
        json_out(&ove(&lib, &models, &["audio", "beats", "Song", "m_music", "--from", "2", "--to", "3", "--json"]));
    assert_eq!(media["beats"], json!([2.0, 2.5, 3.0]));
    assert!(media.get("timelineBeats").is_none());
    // Nothing went into the project document.
    let doc = std::fs::read_to_string(lib.join("Song.ove/project.json")).unwrap();
    assert!(!doc.contains("downbeat"));
}

#[test]
fn detect_beats_answers_its_contract_over_mcp() {
    let d = tempfile::tempdir().unwrap();
    let wav = d.path().join("music.wav");
    clicks(&wav, 120.0, 4, 4.0);
    let lib = setup(d.path(), &[("music", &wav, 4.0, json!({}))]);
    let mut child = Command::new(env!("CARGO_BIN_EXE_ove"))
        .arg("--library")
        .arg(&lib)
        .arg("mcp")
        .env_remove("OVE_LIBRARY")
        .env("OVE_MODELS_DIR", d.path().join("models"))
        .env("OVE_MCP_HOLD_MS", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut stdin = child.stdin.take().unwrap();
    let mut id = 0;
    let mut request = |method: &str, params: Value| -> Value {
        id += 1;
        writeln!(stdin, "{}", json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).unwrap();
        stdin.flush().unwrap();
        let mut reply = String::new();
        out.read_line(&mut reply).unwrap();
        serde_json::from_str(&reply).unwrap()
    };
    request("initialize", json!({"protocolVersion": "2025-06-18", "clientInfo": {"name": "contract"}}));
    let tools = request("tools/list", json!({}))["result"]["tools"].as_array().unwrap().clone();
    let t = tools.iter().find(|t| t["name"] == "detect_beats").expect("detect_beats is listed");
    assert_eq!(t["annotations"]["readOnlyHint"], true, "{t}");
    for k in ["target", "from", "to"] {
        assert!(t["inputSchema"]["properties"].get(k).is_some(), "{t}");
    }
    let r = request("tools/call", json!({"name": "detect_beats", "arguments": {"project": "Song", "target": "music"}}))
        ["result"]
        .clone();
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["structuredContent"]["status"], "modelNotInstalled");
    let _ = child.kill();
    let _ = child.wait();
}

/// The installed model folder for the model checks, or `None` (skipped).
fn model_dir(models: &Path) -> Option<()> {
    let src = std::env::var_os("OVE_BEAT_MODEL_DIR").map(PathBuf::from)?;
    std::env::var_os("OVE_ONNXRUNTIME_LIB")?;
    let dst = models.join(beats::MODEL_ID);
    std::fs::create_dir_all(&dst).unwrap();
    for pin in beats::PINS {
        std::fs::copy(src.join(pin.name), dst.join(pin.name)).ok()?;
    }
    Some(())
}

#[test]
fn the_model_finds_a_120_bpm_click_track_and_three_four_time() {
    let d = tempfile::tempdir().unwrap();
    let models = d.path().join("models");
    if model_dir(&models).is_none() {
        eprintln!("skipped: set OVE_BEAT_MODEL_DIR (the exported ONNX files) and OVE_ONNXRUNTIME_LIB");
        return;
    }
    let (four, three) = (d.path().join("four.wav"), d.path().join("three.wav"));
    clicks(&four, 120.0, 4, 40.0);
    waltz(&three, 150.0, 40.0);
    let lib = setup(d.path(), &[("four", &four, 40.0, json!({})), ("three", &three, 40.0, json!({}))]);
    let v = json_out(&ove(&lib, &models, &["audio", "beats", "Song", "four", "--json"]));
    assert_eq!(v["status"], "ready", "{v}");
    let b: Vec<f64> = v["beats"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
    let worst = b.iter().map(|t| (t / 0.5 - (t / 0.5).round()).abs() * 0.5).fold(0.0, f64::max);
    let ibi: Vec<f64> = b.windows(2).map(|w| w[1] - w[0]).collect();
    eprintln!(
        "120 BPM: {} beats, bpm {}, meter {}, worst offset {:.1} ms",
        b.len(),
        v["bpm"],
        v["meter"],
        worst * 1000.0
    );
    assert!(worst <= 0.020 + 1e-9, "beats within 20 ms of the 500 ms grid: {worst}");
    assert!(ibi.iter().all(|i| (i - 0.5).abs() <= 0.02 + 1e-9), "{ibi:?} {b:?}");
    assert!((v["bpm"].as_f64().unwrap() - 120.0).abs() <= 0.5, "{v}");
    assert!(b.len() >= 76, "{} beats in 40 s", b.len());
    let v3 = json_out(&ove(&lib, &models, &["audio", "beats", "Song", "three", "--json"]));
    eprintln!("3/4: meter {}, bpm {}, {} downbeats", v3["meter"], v3["bpm"], v3["downbeats"].as_array().unwrap().len());
    assert_eq!(v3["meter"], 3, "{v3}");
    // A second call reads the cache.
    assert_eq!(json_out(&ove(&lib, &models, &["audio", "beats", "Song", "four", "--json"]))["cached"], true);
}
