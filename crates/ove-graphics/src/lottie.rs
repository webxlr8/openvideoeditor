//! Lottie frames rendered from the vector source (add-viewer-interaction-and-text-fills, spike task 5.1).
//!
//! ThorVG's software rasteriser, reached through dotlottie-rs, draws one requested frame into an RGBA buffer at the
//! output size. Nothing is cached and nothing touches the disk. The `lottie` item type (tasks 5.2–5.4) builds on this.

use std::ffi::CString;

use dotlottie_rs::{ColorSpace, Player};

/// Why a Lottie frame could not be rendered.
#[derive(Debug, thiserror::Error)]
pub enum LottieError {
    #[error("the Lottie output size {width}x{height} is empty or too large")]
    Size { width: u32, height: u32 },
    #[error("the Lottie data contains a NUL byte")]
    Nul,
    #[error("the Lottie data could not be loaded: {0:?}")]
    Load(dotlottie_rs::player::Error),
    #[error("the Lottie frame could not be rendered: {0:?}")]
    Render(dotlottie_rs::player::Error),
}

/// A loaded animation's timing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LottieInfo {
    /// Frame count (the animation's `op - ip`).
    pub total_frames: f32,
    /// Duration in seconds.
    pub duration: f32,
}

/// Renders frame `frame` of Lottie JSON `json` at `width`×`height`, returning straight (not premultiplied) RGBA8.
pub fn render_frame(json: &str, frame: f32, width: u32, height: u32) -> Result<(Vec<u8>, LottieInfo), LottieError> {
    const MAX_SIDE: u32 = 16_384;
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(LottieError::Size { width, height });
    }
    let data = CString::new(json).map_err(|_| LottieError::Nul)?;
    let pixels = (width as usize) * (height as usize);
    // The buffer is declared before the player so that it outlives it: ThorVG keeps a pointer to it.
    let mut buffer = vec![0u32; pixels];
    let mut player = Player::with_threads(0);
    player.set_autoplay(false);
    player.set_sw_target(&mut buffer, width, width, height, ColorSpace::ABGR8888S).map_err(LottieError::Render)?;
    player.load_animation_data(&data).map_err(LottieError::Load)?;
    // dotlottie-rs reports the duration in milliseconds.
    let info = LottieInfo { total_frames: player.total_frames(), duration: player.duration() / 1000.0 };
    let last = (info.total_frames - 1.0).max(0.0);
    let frame = frame.clamp(0.0, last);
    // Loading already draws frame 0 into the buffer, and ThorVG refuses to "move" to the frame it is on. So a refused
    // move to the current frame means the buffer already holds it.
    match player.set_frame(frame) {
        Ok(()) => player.render().map_err(LottieError::Render)?,
        Err(_) if player.current_frame() == frame => {}
        Err(error) => return Err(LottieError::Render(error)),
    }
    drop(player);
    // ABGR8888 is little-endian 0xAABBGGRR, which is R, G, B, A in memory order.
    let rgba = buffer.iter().flat_map(|px| px.to_le_bytes()).collect();
    Ok((rgba, info))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 100x100, 30-frame animation: a red square at the centre that slides 40 px to the right.
    const SLIDE: &str = r#"{"v":"5.7.0","fr":30,"ip":0,"op":30,"w":100,"h":100,"layers":[
        {"ty":4,"ind":1,"ip":0,"op":30,"st":0,"ks":{
            "o":{"a":0,"k":100},"r":{"a":0,"k":0},"s":{"a":0,"k":[100,100]},"a":{"a":0,"k":[0,0]},
            "p":{"a":1,"k":[{"t":0,"s":[30,50],"i":{"x":[1],"y":[1]},"o":{"x":[0],"y":[0]}},{"t":29,"s":[70,50]}]}},
         "shapes":[{"ty":"gr","it":[
            {"ty":"rc","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[20,20]},"r":{"a":0,"k":0}},
            {"ty":"fl","c":{"a":0,"k":[1,0,0,1]},"o":{"a":0,"k":100}},
            {"ty":"tr","p":{"a":0,"k":[0,0]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100}}]}]}]}"#;

    fn pixel(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * width + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    }

    #[test]
    fn renders_the_requested_frame() {
        let (first, info) = render_frame(SLIDE, 0.0, 100, 100).unwrap();
        assert_eq!(info.total_frames, 30.0);
        assert!((info.duration - 1.0).abs() < 1e-3, "{info:?}");
        assert_eq!(pixel(&first, 100, 30, 50), [255, 0, 0, 255]);
        assert_eq!(pixel(&first, 100, 70, 50)[3], 0);
        let (last, _) = render_frame(SLIDE, 29.0, 100, 100).unwrap();
        assert_eq!(pixel(&last, 100, 70, 50), [255, 0, 0, 255]);
        assert_eq!(pixel(&last, 100, 30, 50)[3], 0);
    }

    #[test]
    fn scales_to_the_output_size_from_the_vector_source() {
        // Drawn at 4x, the square's edge is still a hard step: no pixel between the square and the background.
        let (big, _) = render_frame(SLIDE, 0.0, 400, 400).unwrap();
        assert_eq!(pixel(&big, 400, 120, 200), [255, 0, 0, 255]);
        assert_eq!(pixel(&big, 400, 159, 200), [255, 0, 0, 255]);
        assert_eq!(pixel(&big, 400, 160, 200)[3], 0);
    }

    #[test]
    fn repeat_renders_are_byte_identical() {
        let (a, _) = render_frame(SLIDE, 12.5, 256, 256).unwrap();
        let (b, _) = render_frame(SLIDE, 12.5, 256, 256).unwrap();
        assert!(a == b);
    }

    #[test]
    fn corrupt_data_is_refused() {
        assert!(matches!(render_frame("{\"v\":", 0.0, 64, 64), Err(LottieError::Load(_))));
        assert!(matches!(render_frame(SLIDE, 0.0, 0, 64), Err(LottieError::Size { .. })));
    }
}
