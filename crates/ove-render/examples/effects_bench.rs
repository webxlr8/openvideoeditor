//! Per-effect timings at 540p and 1080p, plus a contact sheet of every effect at its defaults.
//!
//!   cargo run --release -p ove-render --example effects_bench -- [out.ppm]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "developer example, not shipped: failing loudly on a broken setup is the point"
)]

use std::time::Instant;

use ove_render::Rgba;
use ove_render::effects::{EFFECTS, apply, sample_image};
use serde_json::Map;

fn time_ms(w: u32, h: u32, id: &str) -> f64 {
    let base = sample_image(w, h);
    let mut best = f64::MAX;
    for _ in 0..3 {
        let mut img = base.clone();
        let t0 = Instant::now();
        apply(&mut img, id, &Map::new(), 1.0, 1.0);
        best = best.min(t0.elapsed().as_secs_f64() * 1000.0);
    }
    best
}

fn main() {
    let out = std::env::args().nth(1);
    let only: Vec<String> = std::env::args().skip(2).collect();
    if !only.is_empty() {
        // Large single renders of the named effects, for close inspection.
        for id in &only {
            let mut img = sample_image(960, 540);
            apply(&mut img, id, &Map::new(), 1.0, 1.0);
            write_ppm(&format!("{}-{id}.ppm", out.as_deref().unwrap_or("fx")), &img);
        }
        return;
    }
    println!("| effect | 960×540 ms | 1920×1080 ms |");
    println!("|---|---:|---:|");
    for e in EFFECTS {
        println!("| {} | {:.1} | {:.1} |", e.id, time_ms(960, 540, e.id), time_ms(1920, 1080, e.id));
    }
    if let Some(path) = out {
        let (tw, th, cols) = (240u32, 135u32, 7u32);
        let rows = (EFFECTS.len() as u32).div_ceil(cols);
        let mut sheet = Rgba::new(tw * cols, th * rows);
        for (i, e) in EFFECTS.iter().enumerate() {
            let mut img = sample_image(tw, th);
            apply(&mut img, e.id, &Map::new(), 1.0, 1.0);
            let (ox, oy) = ((i as u32 % cols) * tw, (i as u32 / cols) * th);
            for y in 0..th {
                let s = (y * tw * 4) as usize;
                let d = (((oy + y) * sheet.w + ox) * 4) as usize;
                sheet.data[d..d + (tw * 4) as usize].copy_from_slice(&img.data[s..s + (tw * 4) as usize]);
            }
        }
        write_ppm(&path, &sheet);
    }
}

fn write_ppm(path: &str, img: &Rgba) {
    let mut ppm = format!("P6 {} {} 255\n", img.w, img.h).into_bytes();
    for p in img.data.chunks(4) {
        ppm.extend_from_slice(&p[..3]);
    }
    std::fs::write(path, ppm).expect("write ppm");
    eprintln!("wrote {path}");
}
