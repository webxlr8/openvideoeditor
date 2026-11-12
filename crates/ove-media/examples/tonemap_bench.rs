//! Tone-mapping throughput: `cargo run --release -p ove-media --example tonemap_bench [width height frames]`.

use ove_media::Transfer;
use ove_media::tonemap::{YuvParams, YuvSource, tone_map_yuv};

fn main() {
    let args: Vec<usize> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let (w, h, n) = (*args.first().unwrap_or(&1920), *args.get(1).unwrap_or(&1080), *args.get(2).unwrap_or(&100));
    // A 10-bit 4:2:0 luma ramp with varying chroma, reaching into the highlights.
    let y: Vec<u8> = (0..w * h).flat_map(|i| ((64 + (i % w) * 876 / w) as u16).to_le_bytes()).collect();
    let c: Vec<u8> = (0..w / 2 * h / 2).flat_map(|i| ((400 + (i % 200)) as u16).to_le_bytes()).collect();
    let src = YuvSource {
        width: w,
        height: h,
        depth: 10,
        log2_chroma_w: 1,
        log2_chroma_h: 1,
        y: (&y, w * 2),
        cb: (&c, w),
        cr: Some((&c, w)),
        msb_aligned: false,
    };
    let params = YuvParams { kr: 0.2627, kb: 0.0593, full_range: false };
    let mut out = vec![0u8; w * h * 4];
    for transfer in [Transfer::Pq, Transfer::Hlg] {
        tone_map_yuv(&src, params, transfer, true, &mut out);
        let t = std::time::Instant::now();
        for _ in 0..n {
            tone_map_yuv(&src, params, transfer, true, &mut out);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
        println!("{transfer:?} {w}x{h}: {ms:.2} ms/frame on {} threads", rayon::current_num_threads());
    }
}
