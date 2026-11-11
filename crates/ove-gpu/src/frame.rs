//! Decoded video frames as handed to the viewer.

use std::sync::Arc;

/// Pixel layout of a decoded frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// 8-bit RGBA, one plane, 4 bytes per pixel.
    Rgba8,
    /// 8-bit 4:2:0: plane 0 = Y (1 byte/px), plane 1 = interleaved UV at half resolution.
    Nv12,
    /// 10-bit 4:2:0 in 16-bit little-endian words, value in the high 10 bits:
    /// plane 0 = Y (2 bytes/px), plane 1 = interleaved UV at half resolution (4 bytes per pair).
    P010,
}

impl PixelFormat {
    /// Number of planes this format uses.
    pub fn plane_count(self) -> usize {
        match self {
            PixelFormat::Rgba8 => 1,
            PixelFormat::Nv12 | PixelFormat::P010 => 2,
        }
    }

    /// (width in texels, height in texels, bytes per texel) of plane `index`.
    pub fn plane_layout(self, index: usize, width: u32, height: u32) -> (u32, u32, u32) {
        let half = |v: u32| v.div_ceil(2);
        match (self, index) {
            (PixelFormat::Rgba8, 0) => (width, height, 4),
            (PixelFormat::Nv12, 0) => (width, height, 1),
            (PixelFormat::Nv12, 1) => (half(width), half(height), 2),
            (PixelFormat::P010, 0) => (width, height, 2),
            (PixelFormat::P010, 1) => (half(width), half(height), 4),
            _ => (0, 0, 0),
        }
    }
}

/// YCbCr → RGB matrix of a YUV frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum YuvMatrix {
    Bt601,
    #[default]
    Bt709,
    Bt2020,
}

impl YuvMatrix {
    /// (Kr, Kb) luma coefficients.
    pub fn coefficients(self) -> (f32, f32) {
        match self {
            YuvMatrix::Bt601 => (0.299, 0.114),
            YuvMatrix::Bt709 => (0.2126, 0.0722),
            YuvMatrix::Bt2020 => (0.2627, 0.0593),
        }
    }
}

/// One decoded frame in CPU memory (the copy-upload path). Planes are reference counted so a
/// frame can be retained for device-loss recovery without copying.
#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub planes: Vec<Arc<[u8]>>,
    /// Bytes per row of each plane.
    pub strides: Vec<u32>,
    /// Presentation time in microseconds.
    pub pts: i64,
    pub matrix: YuvMatrix,
    /// Full ("PC") range instead of limited ("TV") range. Ignored for RGBA.
    pub full_range: bool,
}

/// Reasons a frame cannot be uploaded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    #[error("frame has zero width or height")]
    Empty,
    #[error("{format:?} needs {expected} planes and strides, frame has {actual}")]
    PlaneCount { format: PixelFormat, expected: usize, actual: usize },
    #[error("plane {plane}: stride {stride} is smaller than one row ({row} bytes)")]
    Stride { plane: usize, stride: u32, row: u32 },
    #[error("plane {plane}: {actual} bytes, needs at least {expected}")]
    PlaneSize { plane: usize, expected: usize, actual: usize },
}

impl VideoFrame {
    /// Tightly packed RGBA frame.
    pub fn rgba(width: u32, height: u32, data: Vec<u8>, pts: i64) -> Self {
        VideoFrame {
            width,
            height,
            format: PixelFormat::Rgba8,
            planes: vec![data.into()],
            strides: vec![width * 4],
            pts,
            matrix: YuvMatrix::Bt709,
            full_range: true,
        }
    }

    /// Tightly packed NV12 frame (`y` is width×height, `uv` is interleaved at half resolution).
    pub fn nv12(width: u32, height: u32, y: Vec<u8>, uv: Vec<u8>, pts: i64) -> Self {
        VideoFrame {
            width,
            height,
            format: PixelFormat::Nv12,
            planes: vec![y.into(), uv.into()],
            strides: vec![width, width.div_ceil(2) * 2],
            pts,
            matrix: YuvMatrix::Bt709,
            full_range: false,
        }
    }

    /// Checks plane count, strides and sizes against the declared format.
    pub fn validate(&self) -> Result<(), FrameError> {
        if self.width == 0 || self.height == 0 {
            return Err(FrameError::Empty);
        }
        let expected = self.format.plane_count();
        if self.planes.len() != expected || self.strides.len() != expected {
            return Err(FrameError::PlaneCount {
                format: self.format,
                expected,
                actual: self.planes.len().min(self.strides.len()),
            });
        }
        for plane in 0..expected {
            let (w, h, bpp) = self.format.plane_layout(plane, self.width, self.height);
            let row = w * bpp;
            let stride = self.strides[plane];
            if stride < row {
                return Err(FrameError::Stride { plane, stride, row });
            }
            let needed = stride as usize * (h as usize - 1) + row as usize;
            if self.planes[plane].len() < needed {
                return Err(FrameError::PlaneSize { plane, expected: needed, actual: self.planes[plane].len() });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_nv12() {
        let f = VideoFrame::nv12(4, 2, vec![0; 8], vec![0; 4], 0);
        assert_eq!(f.validate(), Ok(()));
        let mut short = f.clone();
        short.planes[1] = vec![0u8; 3].into();
        assert!(matches!(short.validate(), Err(FrameError::PlaneSize { plane: 1, .. })));
        let mut bad = f.clone();
        bad.strides[0] = 3;
        assert!(matches!(bad.validate(), Err(FrameError::Stride { plane: 0, .. })));
        let mut missing = f;
        missing.planes.pop();
        assert!(matches!(missing.validate(), Err(FrameError::PlaneCount { .. })));
    }

    #[test]
    fn odd_sizes_round_chroma_up() {
        assert_eq!(PixelFormat::Nv12.plane_layout(1, 5, 3), (3, 2, 2));
        assert_eq!(PixelFormat::P010.plane_layout(1, 5, 3), (3, 2, 4));
    }
}
