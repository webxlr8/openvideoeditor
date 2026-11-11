//! Viewer geometry: where a frame lands inside the viewer rectangle.

/// A rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PhysicalRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// A rectangle in logical (CSS / point) pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LogicalRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl LogicalRect {
    /// Converts to physical pixels. Edges are rounded independently so adjacent rectangles stay
    /// adjacent and each edge is off by at most half a physical pixel.
    pub fn to_physical(self, scale: f64) -> PhysicalRect {
        let left = (self.x * scale).round();
        let top = (self.y * scale).round();
        let right = ((self.x + self.width.max(0.0)) * scale).round();
        let bottom = ((self.y + self.height.max(0.0)) * scale).round();
        PhysicalRect {
            x: left as i32,
            y: top as i32,
            width: (right - left).max(0.0) as u32,
            height: (bottom - top).max(0.0) as u32,
        }
    }
}

/// Aspect-fits a `frame_w`×`frame_h` image (square pixels) into a `target_w`×`target_h` area,
/// centred, letterboxed or pillarboxed. Returns the image rectangle relative to the area.
pub fn fit_rect(frame_w: u32, frame_h: u32, target_w: u32, target_h: u32) -> PhysicalRect {
    if frame_w == 0 || frame_h == 0 || target_w == 0 || target_h == 0 {
        return PhysicalRect::default();
    }
    let (fw, fh, tw, th) = (u64::from(frame_w), u64::from(frame_h), u64::from(target_w), u64::from(target_h));
    let (w, h) = if fw * th >= tw * fh {
        // Frame is relatively wider: full width, bars top and bottom.
        let h = (tw * fh + fw / 2) / fw;
        (target_w, (h as u32).clamp(1, target_h))
    } else {
        let w = (th * fw + fh / 2) / fh;
        ((w as u32).clamp(1, target_w), target_h)
    };
    PhysicalRect { x: ((target_w - w) / 2) as i32, y: ((target_h - h) / 2) as i32, width: w, height: h }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_fit() {
        assert_eq!(fit_rect(1920, 1080, 1280, 720), PhysicalRect { x: 0, y: 0, width: 1280, height: 720 });
    }

    #[test]
    fn letterbox_wide_frame() {
        let r = fit_rect(2390, 1000, 1600, 900);
        assert_eq!((r.y, r.width, r.height), (115, 1600, 669));
    }

    #[test]
    fn pillarbox_tall_frame() {
        let r = fit_rect(1080, 1920, 1600, 900);
        assert_eq!((r.x, r.width, r.height), (547, 506, 900));
    }

    #[test]
    fn degenerate_sizes() {
        assert_eq!(fit_rect(0, 10, 10, 10), PhysicalRect::default());
        assert_eq!(fit_rect(10, 10, 0, 10), PhysicalRect::default());
        assert_eq!(fit_rect(100_000, 1, 100, 100).height, 1);
    }

    #[test]
    fn logical_to_physical_rounds_edges() {
        let r = LogicalRect { x: 10.3, y: 20.6, width: 100.5, height: 50.2 }.to_physical(2.0);
        assert_eq!(r, PhysicalRect { x: 21, y: 41, width: 201, height: 101 });
        let r = LogicalRect { x: 1.0, y: 1.0, width: 3.0, height: 3.0 }.to_physical(1.5);
        assert_eq!(r, PhysicalRect { x: 2, y: 2, width: 4, height: 4 });
    }
}
