//! Media commands on synthetic files (needs `--features media` and the ffmpeg CLI for fixtures:
//! `OVE_FIXTURE_FFMPEG` or `ffmpeg` on PATH; skipped with a message otherwise).
#![cfg(feature = "media")]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use serde_json::{Value, json};

fn ffmpeg() -> Option<String> {
    let bin = std::env::var("OVE_FIXTURE_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    Command::new(&bin).arg("-version").output().ok().filter(|o| o.status.success()).map(|_| bin)
}

fn make(dir: &Path, name: &str, args: &[&str]) -> PathBuf {
    let out = dir.join(name);
    let st = Command::new(ffmpeg().unwrap()).args(["-loglevel", "error", "-y"]).args(args).arg(&out).status().unwrap();
    assert!(st.success(), "ffmpeg failed for {name}");
    out
}

fn ove(lib: &Path, args: &[&str]) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_ove"))
        .arg("--library")
        .arg(lib)
        .arg("--json")
        .args(args)
        .env_remove("OVE_LIBRARY")
        .output()
        .unwrap();
    assert!(out.status.success(), "ove {args:?}\nstderr: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn add_inspect_look_and_remove() {
    if ffmpeg().is_none() {
        eprintln!("skipped: no ffmpeg CLI to make fixtures (set OVE_FIXTURE_FFMPEG)");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    // 1 s test pattern then 1 s blue (a hard cut at 1 s), with a tone that stops at 1 s.
    let clip = make(
        d.path(),
        "clip.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=s=320x180:r=25:d=1",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=320x180:r=25:d=1",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=1",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=48000:cl=mono",
            "-filter_complex",
            "[0:v][1:v]concat=n=2:v=1:a=0[v];[2:a][3:a]concat=n=2:v=0:a=1,atrim=0:2[a]",
            "-map",
            "[v]",
            "-map",
            "[a]",
            "-pix_fmt",
            "yuv420p",
            "-c:v",
            "libx264",
            "-c:a",
            "aac",
            "-t",
            "2",
        ],
    );
    let music = make(
        d.path(),
        "music.wav",
        &["-f", "lavfi", "-i", "sine=frequency=1000:sample_rate=48000:duration=3", "-af", "volume=-20dB"],
    );

    ove(&lib, &["project", "create", "Promo"]);
    let added = ove(&lib, &["media", "add", "Promo", clip.to_str().unwrap(), music.to_str().unwrap()]);
    assert_eq!(added["mediaAdded"], 2, "{added}");
    assert_eq!(added["clips"].as_array().unwrap().len(), 2);
    // Adding the same file again reuses the media.
    let again = ove(&lib, &["media", "add", "Promo", clip.to_str().unwrap(), "--at", "10"]);
    assert_eq!(again["mediaAdded"], 0);

    let listed = ove(&lib, &["media", "list", "Promo"]);
    let rows = listed["media"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let video = rows.iter().find(|r| r["kind"] == "video").unwrap();
    assert_eq!(video["uses"], 2);
    assert_eq!(video["status"], "online");
    assert!((video["durationSeconds"].as_f64().unwrap() - 2.0).abs() < 0.1);
    let shown = ove(&lib, &["project", "show", "Promo"]);
    let main = shown["tracks"].as_array().unwrap().iter().find(|t| t["id"] == "t_main").unwrap().clone();
    assert_eq!(main["items"], 2);

    let probe = ove(&lib, &["media", "probe", "Promo", "clip.mp4"]);
    assert_eq!(probe["video"]["width"], 320);

    let png = d.path().join("f.png");
    let g = ove(&lib, &["media", "grab", "Promo", "clip.mp4", "--at", "1.5", "--out", png.to_str().unwrap()]);
    assert_eq!((g["width"].as_u64(), g["height"].as_u64()), (Some(320), Some(180)));
    assert!(g["stats"]["meanRgb"][2].as_f64().unwrap() > 0.6, "blue frame: {g}");
    assert!(std::fs::read(&png).unwrap().starts_with(b"\x89PNG"));

    let sc = ove(&lib, &["media", "scenes", "Promo", "clip.mp4"]);
    let cuts = sc["cuts"].as_array().unwrap();
    assert_eq!(cuts.len(), 1, "{sc}");
    assert!((cuts[0]["time"].as_f64().unwrap() - 1.0).abs() <= 0.05);

    let strip = ove(
        &lib,
        &["media", "filmstrip", "Promo", "clip.mp4", "--count", "4", "--out", d.path().join("s.png").to_str().unwrap()],
    );
    assert_eq!(strip["times"].as_array().unwrap().len(), 4);

    let sil = ove(&lib, &["audio", "silence", "Promo", "clip.mp4"]);
    let r = &sil["ranges"][0];
    assert!((r["start"].as_f64().unwrap() - 1.0).abs() < 0.1 && r["end"].as_f64().unwrap() > 1.8, "{sil}");
    let loud = ove(&lib, &["audio", "loudness", "Promo", "music.wav"]);
    // lavfi's sine has amplitude 1/8 (−18.06 dBFS); −20 dB more peaks at −38.06 dBFS, and a mono sine reads
    // 3.01 dB below its peak (BS.1770): −41.07 LUFS.
    assert!((loud["samplePeakDbfs"].as_f64().unwrap() + 38.06).abs() < 0.1, "{loud}");
    assert!((loud["integratedLufs"].as_f64().unwrap() + 41.07).abs() < 0.3, "{loud}");

    let look = ove(
        &lib,
        &[
            "look",
            "Promo",
            "--at",
            "0.5,1.5",
            "--width",
            "320",
            "--stats",
            "--sheet",
            d.path().join("sheet.png").to_str().unwrap(),
        ],
    );
    let frames = look["frames"].as_array().unwrap();
    assert_eq!(frames.len(), 2);
    assert!(frames[1]["stats"]["meanRgb"][2].as_f64().unwrap() > 0.6, "timeline frame at 1.5 s is blue: {look}");
    assert!(d.path().join("sheet.png").is_file());

    let removed = ove(&lib, &["media", "remove", "Promo", "music.wav"]);
    assert_eq!(removed["removedClips"], 1);
    assert_eq!(ove(&lib, &["media", "list", "Promo", "--all"])["count"], 1);

    // Relink: move the file, find it again by folder.
    let moved = d.path().join("moved");
    std::fs::create_dir_all(&moved).unwrap();
    std::fs::rename(&clip, moved.join("clip.mp4")).unwrap();
    assert_eq!(ove(&lib, &["media", "list", "Promo"])["media"][0]["status"], "offline");
    let rl = ove(&lib, &["media", "relink", "Promo", "--folder", moved.to_str().unwrap()]);
    assert_eq!(rl["relinked"].as_array().unwrap().len(), 1);
    assert_eq!(ove(&lib, &["media", "list", "Promo"])["media"][0]["status"], "online");

    // MCP returns frames as images.
    let mut child = Command::new(env!("CARGO_BIN_EXE_ove"))
        .arg("--library")
        .arg(&lib)
        .arg("mcp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::{BufRead, Write};
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, "{}", json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "look", "arguments": {"project": "Promo", "at": "1.5", "width": 160}}})).unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let _ = child.kill();
    let _ = child.wait();
    let v: Value = serde_json::from_str(&line).unwrap();
    let content = v["result"]["content"].as_array().unwrap();
    assert_eq!(v["result"]["isError"], false, "{v}");
    assert!(content.iter().any(|c| c["type"] == "image" && c["mimeType"] == "image/png"), "{line}");
}
