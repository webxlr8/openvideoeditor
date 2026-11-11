#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;

use md5::{Digest, Md5};
use ove_media::VideoFrame;

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// Path of a matrix file, generating the synthetic matrix on first use (needs python3 + ffmpeg on PATH).
pub fn media(name: &str) -> PathBuf {
    static GEN: Once = Once::new();
    let root = repo_root();
    GEN.call_once(|| {
        let manifest = root.join("tests/media/MANIFEST.json");
        let m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
        let missing = m["synthetic"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| !root.join("tests/media/files").join(e["name"].as_str().unwrap()).exists());
        if missing {
            let st = Command::new("python3")
                .arg(root.join("tests/media/media.py"))
                .arg("generate")
                .status()
                .expect("python3 needed to generate the media matrix");
            assert!(st.success(), "media matrix generation failed");
        }
    });
    root.join("tests/media/files").join(name)
}

/// Every synthetic matrix entry name.
pub fn matrix() -> Vec<String> {
    let manifest = repo_root().join("tests/media/MANIFEST.json");
    let m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(manifest).unwrap()).unwrap();
    m["synthetic"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap().to_string()).collect()
}

pub const VIDEO_FILES: &[&str] = &[
    "h264_longgop.mp4",
    "h264_60fps.mov",
    "h264_4k60.mp4",
    "hevc_hdr_vfr.mov",
    "hevc_main10_420.mp4",
    "hevc_main422_10.mov",
    "prores_proxy.mov",
    "prores_lt.mov",
    "prores_hq.mov",
    "prores_4444.mov",
    "dnxhr_hq.mxf",
    "av1.mkv",
    "vp9_opus.webm",
    "vfr_phone.mp4",
];

pub const AUDIO_FILES: &[&str] = &[
    "audio_aac.m4a",
    "audio_pcm.wav",
    "audio_alac.m4a",
    "audio_opus.mka",
    "prores_hq.mov",
    "dnxhr_hq.mxf",
    "h264_longgop.mp4",
    "vp9_opus.webm",
];

pub fn frame_md5(f: &VideoFrame) -> String {
    let mut h = Md5::new();
    for p in &f.planes {
        h.update(&p.data);
    }
    format!("{:x}", h.finalize())
}

/// The ffmpeg CLI used for reference output: `OVE_FFMPEG_CLI` (CI points it at tools/ffmpeg's build, which uses the
/// same libraries the crate links), else `ffmpeg` on PATH.
pub fn ffmpeg_cli() -> String {
    std::env::var("OVE_FFMPEG_CLI").unwrap_or_else(|_| "ffmpeg".into())
}

/// Whether the reference CLI runs the same FFmpeg version as the linked libraries. Video decoders are bit-exact
/// across versions; audio decoders (AAC) are not, so audio checks are strict only when versions match.
pub fn cli_matches_libraries() -> bool {
    let out = Command::new(ffmpeg_cli()).arg("-version").output().expect("ffmpeg CLI");
    let first = String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or_default().to_string();
    let lib = ove_media::ffmpeg_version();
    let same = first.split_whitespace().nth(2) == Some(lib.as_str());
    if !same {
        eprintln!("note: reference CLI is '{first}', libraries are {lib}");
    }
    same
}

/// Per-frame md5 of the first video stream as decoded by the ffmpeg CLI (`framemd5`), in output order.
pub fn reference_framemd5(path: &Path) -> Vec<String> {
    let out = Command::new(ffmpeg_cli())
        .args(["-hide_banner", "-loglevel", "error", "-threads", "1", "-i"])
        .arg(path)
        .args(["-map", "0:v:0", "-fps_mode", "passthrough", "-f", "framemd5", "-"])
        .output()
        .expect("ffmpeg on PATH");
    assert!(out.status.success(), "ffmpeg framemd5 failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| l.rsplit(',').next().unwrap().trim().to_string())
        .collect()
}

/// Tiny deterministic PRNG (xorshift64*), so seek targets are reproducible without extra dependencies.
pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}
