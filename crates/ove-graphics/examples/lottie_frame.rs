//! Renders one frame of a Lottie JSON file and prints its size and timing (task 5.1 spike).
//!
//! `cargo run -p ove-graphics --features lottie --example lottie_frame -- <file.json> <frame> <width> <height>`

#[allow(clippy::unwrap_used, clippy::expect_used)]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, path, frame, width, height] = args.as_slice() else {
        eprintln!("usage: lottie_frame <file.json> <frame> <width> <height>");
        std::process::exit(2);
    };
    let json = std::fs::read_to_string(path).expect("read the Lottie file");
    let started = std::time::Instant::now();
    let (rgba, info) = ove_graphics::lottie::render_frame(
        &json,
        frame.parse().expect("frame"),
        width.parse().expect("width"),
        height.parse().expect("height"),
    )
    .expect("render");
    let opaque = rgba.chunks_exact(4).filter(|px| px[3] > 0).count();
    println!(
        "{} frames, {:.3} s, {} drawn pixels, rendered in {:.2} ms",
        info.total_frames,
        info.duration,
        opaque,
        started.elapsed().as_secs_f64() * 1000.0
    );
}
