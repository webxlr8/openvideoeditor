use serde::Serialize;

use crate::util::Micros;

/// Pixel layout of a decoded frame handed to the viewer or to tests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PixelFormat {
    /// 8-bit 4:2:0 (also used for 8-bit 4:2:2/4:4:4 sources, chroma subsampled for preview), plane 0 = Y, plane 1 = interleaved UV.
    Nv12,
    /// 10-bit 4:2:0 in 16-bit little-endian words (high bits), plane 0 = Y, plane 1 = interleaved UV. Used for every
    /// 9–16-bit planar YUV source (ProRes, DNxHR 10-bit, HEVC Main10/4:2:2), chroma subsampled to 4:2:0 for preview.
    P010,
    /// 8-bit RGBA, one plane, straight (non-premultiplied) alpha. Sources with alpha or RGB sources.
    Rgba8,
    /// The decoder's own format (libavutil name, e.g. `yuv422p10le`), planes as decoded. Used by checksum tests.
    Native(String),
}

/// How the decoder should hand frames out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    /// Convert to a GPU-friendly format: NV12 (8-bit YUV), P010 (deeper YUV) or RGBA8 (alpha / RGB sources).
    #[default]
    Display,
    /// Keep the decoder's format; planes tightly packed exactly like FFmpeg's `rawvideo`/`framemd5` output.
    Native,
    /// RGBA8 (straight alpha), scaled down to fit `max_width` x `max_height` (aspect kept, never upscaled; 0 = no
    /// limit). For the compositor, which works in RGBA at canvas resolution.
    Rgba { max_width: u32, max_height: u32 },
}

/// One tightly packed plane (`stride == bytes per row`, no padding).
#[derive(Clone, PartialEq, Eq)]
pub struct Plane {
    pub data: Vec<u8>,
    pub stride: usize,
    /// Width in samples (pixels for luma/RGBA, chroma samples for subsampled planes).
    pub width: u32,
    pub height: u32,
}

impl std::fmt::Debug for Plane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Plane({}x{}, stride {}, {} bytes)", self.width, self.height, self.stride, self.data.len())
    }
}

/// Where a frame was decoded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "api")]
pub enum DecodePath {
    /// Platform decoder, e.g. `videotoolbox`, `d3d11va`.
    Hardware(String),
    Software,
}

impl std::fmt::Display for DecodePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodePath::Hardware(api) => write!(f, "hardware ({api})"),
            DecodePath::Software => f.write_str("software"),
        }
    }
}

/// A decoded picture with its presentation time.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoFrame {
    /// Presentation time, µs from the stream's start.
    pub pts_us: Micros,
    /// Duration until the next frame as reported by the container (0 if unknown).
    pub duration_us: Micros,
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub planes: Vec<Plane>,
    /// Matrix, range and HDR flag for YUV → RGB in the viewer.
    pub color: crate::convert::ColorInfo,
    pub decode_path: DecodePath,
    /// Number of CPU copies this frame went through after decode (0 = zero-copy, reported by diagnostics).
    pub copies: u8,
}

impl VideoFrame {
    /// Total bytes over all planes.
    pub fn byte_len(&self) -> usize {
        self.planes.iter().map(|p| p.data.len()).sum()
    }
}
