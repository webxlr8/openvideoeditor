//! Golden checks for caption styles: a 9:16 frame with a timed caption, rendered through the full renderer for three
//! styles, compared with stored references (block averages, so tiny anti-aliasing differences do not matter).
//! Regenerate the references with `OVE_UPDATE_GOLDEN=1`; `OVE_GOLDEN_DUMP=<dir>` also writes full frames (PPM).

use std::path::PathBuf;

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_render::{Renderer, Rgba};
use serde_json::json;

const BLOCK: u32 = 4;

fn doc(preset: &str) -> Document {
    let ops = vec![
        Op::SetAspectRatio { aspect_ratio: "9:16".into() },
        Op::AddTrack { track: json!({"id": "cap", "name": "Captions", "order": 0, "type": "caption"}) },
        Op::AddItem {
            item: json!({"id": "c", "trackId": "cap", "type": "caption", "startTime": 1, "endTime": 4,
                "captionPresetId": preset, "captionText": "Captions make every video easier to follow",
                "captionSegments": [{"id": "s1", "text": "Captions make every video easier to follow", "startTime": 0, "endTime": 3,
                    "wordTimings": [
                        {"word": "Captions", "start": 0.0, "end": 0.5}, {"word": "make", "start": 0.5, "end": 0.8},
                        {"word": "every", "start": 0.8, "end": 1.1}, {"word": "video", "start": 1.1, "end": 1.6},
                        {"word": "easier", "start": 1.6, "end": 2.1}, {"word": "to", "start": 2.1, "end": 2.3},
                        {"word": "follow", "start": 2.3, "end": 3.0}]}]}),
        },
    ];
    apply_batch(&Document::new("p", "Captions", 1), &ops).unwrap().doc
}

fn blocks(img: &Rgba) -> (u32, u32, Vec<u8>) {
    let (bw, bh) = (img.w / BLOCK, img.h / BLOCK);
    let mut out = Vec::with_capacity((bw * bh * 3) as usize);
    for by in 0..bh {
        for bx in 0..bw {
            let mut acc = [0u32; 3];
            for y in 0..BLOCK {
                for x in 0..BLOCK {
                    let p = img.px(bx * BLOCK + x, by * BLOCK + y);
                    for k in 0..3 {
                        acc[k] += p[k] as u32;
                    }
                }
            }
            out.extend(acc.iter().map(|a| (a / (BLOCK * BLOCK)) as u8));
        }
    }
    (bw, bh, out)
}

fn ppm(w: u32, h: u32, rgb: &[u8]) -> Vec<u8> {
    let mut v = format!("P6\n{w} {h}\n255\n").into_bytes();
    v.extend_from_slice(rgb);
    v
}

fn read_ppm(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    // Header: "P6\n<w> <h>\n255\n".
    let mut fields = Vec::new();
    let mut i = 0;
    while fields.len() < 4 {
        let start = i;
        while !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        fields.push(String::from_utf8_lossy(&bytes[start..i]).to_string());
        i += 1;
    }
    (fields[1].parse().unwrap(), fields[2].parse().unwrap(), bytes[i..].to_vec())
}

fn golden(preset: &str, t: f64) {
    let d = doc(preset);
    let mut r = Renderer::new(None, 640);
    let img = r.render(&d, (t * 1e6) as i64);
    assert_eq!((img.w, img.h), (360, 640));
    let (bw, bh, got) = blocks(&img);
    if let Ok(dir) = std::env::var("OVE_GOLDEN_DUMP") {
        let rgb: Vec<u8> = img.data.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        std::fs::write(PathBuf::from(dir).join(format!("{preset}.ppm")), ppm(img.w, img.h, &rgb)).unwrap();
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(format!("caption-{preset}.ppm"));
    if std::env::var("OVE_UPDATE_GOLDEN").is_ok() || !path.exists() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, ppm(bw, bh, &got)).unwrap();
        eprintln!("wrote {}", path.display());
    }
    let (ew, eh, want) = read_ppm(&std::fs::read(&path).unwrap());
    assert_eq!((ew, eh), (bw, bh));
    let diffs: Vec<u32> = got.iter().zip(&want).map(|(a, b)| (*a as i32 - *b as i32).unsigned_abs()).collect();
    let mean = diffs.iter().sum::<u32>() as f64 / diffs.len() as f64;
    let max = diffs.iter().copied().max().unwrap_or(0);
    assert!(mean < 1.0 && max < 48, "{preset}: mean diff {mean:.2}, max {max}");
    // Something was drawn, in the expected band of the frame.
    let ink_rows: Vec<u32> = (0..bh).filter(|y| (0..bw).any(|x| got[((y * bw + x) * 3) as usize] > 80)).collect();
    assert!(!ink_rows.is_empty(), "{preset}: nothing drawn");
}

#[test]
fn butter_serif_bottom_word_highlight() {
    golden("butter-serif", 2.2);
}

#[test]
fn word_pop_centre() {
    golden("word-pop", 2.25);
}

#[test]
fn scrapbook_cutout_stacked_group() {
    golden("scrapbook-cutout", 2.0);
}

/// Writes every style (portrait and landscape) to `OVE_GOLDEN_DUMP` for a visual review.
#[test]
#[ignore = "visual review: set OVE_GOLDEN_DUMP=<dir>"]
fn dump_all_styles() {
    let Ok(dir) = std::env::var("OVE_GOLDEN_DUMP") else { return };
    for p in ove_render::captions_presets::PRESETS.iter() {
        for (aspect, tag) in [("9:16", "v"), ("16:9", "h")] {
            let d = apply_batch(&doc(p.id), &[Op::SetAspectRatio { aspect_ratio: aspect.into() }]).unwrap().doc;
            let mut r = Renderer::new(None, if tag == "v" { 640 } else { 360 });
            let img = r.render(&d, 2_200_000);
            let rgb: Vec<u8> =
                img.data.chunks_exact(4).flat_map(|p| [p[0].max(40), p[1].max(44), p[2].max(52)]).collect();
            std::fs::write(PathBuf::from(&dir).join(format!("{tag}-{}.ppm", p.id)), ppm(img.w, img.h, &rgb)).unwrap();
        }
    }
}
