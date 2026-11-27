//! Shared helpers for the golden-frame tests: a small RGBA image file format (PAM, `P7`) and a tolerance comparison.
//! PAM keeps alpha, needs no image library, and opens in common image viewers for a review by eye.
#![allow(dead_code, reason = "each test binary uses a subset of the helpers")]

use std::path::{Path, PathBuf};

/// Largest allowed difference of any channel (0–255) between a frame and its golden copy. One 8-bit step absorbs
/// rounding differences between the maths libraries of macOS, Windows and Linux; a real change is larger.
pub const TOLERANCE: u8 = 1;

/// `crates/ove-render/tests/golden/<name>`.
pub fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name)
}

/// Whether the run should write new golden files (`OVE_UPDATE_GOLDEN=1`). Bless only after checking the frames by eye.
pub fn blessing() -> bool {
    std::env::var_os("OVE_UPDATE_GOLDEN").is_some()
}

pub fn write_pam(path: &Path, w: u32, h: u32, rgba: &[u8]) -> std::io::Result<()> {
    assert_eq!(rgba.len(), (w * h * 4) as usize, "RGBA buffer size");
    let mut v = format!("P7\nWIDTH {w}\nHEIGHT {h}\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n").into_bytes();
    v.extend_from_slice(rgba);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, v)
}

/// Reads a PAM file written by [`write_pam`]: `(width, height, rgba)`, or `None` when the file is not one.
pub fn read_pam(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    const END: &[u8] = b"ENDHDR\n";
    let end = bytes.windows(END.len()).position(|w| w == END)? + END.len();
    let header = std::str::from_utf8(&bytes[..end]).ok()?;
    let mut lines = header.lines();
    if lines.next()? != "P7" {
        return None;
    }
    let (mut w, mut h, mut depth) = (0u32, 0u32, 0u32);
    for line in lines {
        let mut f = line.split_whitespace();
        match (f.next(), f.next()) {
            (Some("WIDTH"), Some(v)) => w = v.parse().ok()?,
            (Some("HEIGHT"), Some(v)) => h = v.parse().ok()?,
            (Some("DEPTH"), Some(v)) => depth = v.parse().ok()?,
            _ => {}
        }
    }
    let data = bytes[end..].to_vec();
    (depth == 4 && data.len() == (w * h * 4) as usize).then_some((w, h, data))
}

/// Largest channel difference between two equal-sized buffers.
pub fn max_diff(a: &[u8], b: &[u8]) -> u8 {
    a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0)
}

/// Writes the golden file when blessing, then returns the stored golden contents. A missing golden file fails the
/// test: a new golden is made on purpose, never by accident.
pub fn golden_or_bless(name: &str, w: u32, h: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
    let path = golden_path(name);
    if blessing() {
        write_pam(&path, w, h, rgba).unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
        eprintln!("wrote {}", path.display());
    }
    assert!(path.exists(), "{} is missing: check the frames by eye, then run with OVE_UPDATE_GOLDEN=1", path.display());
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    read_pam(&bytes).unwrap_or_else(|| panic!("{} is not an RGBA PAM file", path.display()))
}
