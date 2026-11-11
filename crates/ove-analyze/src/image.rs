//! RGBA images: resizing, contact sheets, PNG encoding and simple exposure statistics.

use serde::Serialize;

/// An RGBA8 image (straight alpha, tightly packed rows). Serializes as its size only.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Image {
    pub width: u32,
    pub height: u32,
    #[serde(skip)]
    pub rgba: Vec<u8>,
}

impl Image {
    /// A `width`×`height` image filled with one colour.
    pub fn filled(width: u32, height: u32, c: [u8; 4]) -> Image {
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..width * height {
            rgba.extend_from_slice(&c);
        }
        Image { width, height, rgba }
    }

    pub fn px(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }

    fn put(&mut self, x: u32, y: u32, c: [u8; 4]) {
        let i = ((y * self.width + x) * 4) as usize;
        self.rgba[i..i + 4].copy_from_slice(&c);
    }

    /// PNG bytes (RGBA, 8-bit).
    pub fn png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.width.max(1), self.height.max(1));
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(png::Compression::Fast);
            let mut w = enc.write_header().expect("png header to memory");
            w.write_image_data(&self.rgba).expect("png data to memory");
        }
        out
    }

    /// Resized to `width`×`height`: area averaging when shrinking, bilinear when enlarging.
    pub fn resize(&self, width: u32, height: u32) -> Image {
        let (width, height) = (width.max(1), height.max(1));
        if width == self.width && height == self.height {
            return self.clone();
        }
        if width <= self.width && height <= self.height {
            self.shrink(width, height)
        } else {
            self.bilinear(width, height)
        }
    }

    /// Resized to `width` keeping the aspect ratio.
    pub fn resize_to_width(&self, width: u32) -> Image {
        let h = ((self.height as f64 * width as f64 / self.width.max(1) as f64).round() as u32).max(1);
        self.resize(width, h)
    }

    fn shrink(&self, width: u32, height: u32) -> Image {
        let (sx, sy) = (self.width as f64 / width as f64, self.height as f64 / height as f64);
        let mut out = Image::filled(width, height, [0, 0, 0, 0]);
        for y in 0..height {
            let (y0, y1) = (y as f64 * sy, (y + 1) as f64 * sy);
            for x in 0..width {
                let (x0, x1) = (x as f64 * sx, (x + 1) as f64 * sx);
                let mut acc = [0f64; 4];
                let mut wsum = 0.0;
                let mut yy = y0.floor() as u32;
                while (yy as f64) < y1 && yy < self.height {
                    let wy = (y1.min(yy as f64 + 1.0) - y0.max(yy as f64)).max(0.0);
                    let mut xx = x0.floor() as u32;
                    while (xx as f64) < x1 && xx < self.width {
                        let wx = (x1.min(xx as f64 + 1.0) - x0.max(xx as f64)).max(0.0);
                        let w = wx * wy;
                        let p = self.px(xx, yy);
                        for c in 0..4 {
                            acc[c] += p[c] as f64 * w;
                        }
                        wsum += w;
                        xx += 1;
                    }
                    yy += 1;
                }
                let c = |i: usize| (acc[i] / wsum.max(1e-9)).round().clamp(0.0, 255.0) as u8;
                out.put(x, y, [c(0), c(1), c(2), c(3)]);
            }
        }
        out
    }

    fn bilinear(&self, width: u32, height: u32) -> Image {
        let mut out = Image::filled(width, height, [0, 0, 0, 0]);
        let (sx, sy) = (self.width as f64 / width as f64, self.height as f64 / height as f64);
        for y in 0..height {
            let fy = ((y as f64 + 0.5) * sy - 0.5).clamp(0.0, (self.height - 1) as f64);
            let (y0, ty) = (fy.floor() as u32, fy.fract());
            let y1 = (y0 + 1).min(self.height - 1);
            for x in 0..width {
                let fx = ((x as f64 + 0.5) * sx - 0.5).clamp(0.0, (self.width - 1) as f64);
                let (x0, tx) = (fx.floor() as u32, fx.fract());
                let x1 = (x0 + 1).min(self.width - 1);
                let (a, b, c, d) = (self.px(x0, y0), self.px(x1, y0), self.px(x0, y1), self.px(x1, y1));
                let mut p = [0u8; 4];
                for i in 0..4 {
                    let top = a[i] as f64 * (1.0 - tx) + b[i] as f64 * tx;
                    let bot = c[i] as f64 * (1.0 - tx) + d[i] as f64 * tx;
                    p[i] = (top * (1.0 - ty) + bot * ty).round().clamp(0.0, 255.0) as u8;
                }
                out.put(x, y, p);
            }
        }
        out
    }

    /// Copies `src` with its top-left corner at (`x`, `y`), clipped to this image.
    pub fn blit(&mut self, src: &Image, x: u32, y: u32) {
        for row in 0..src.height {
            let dy = y + row;
            if dy >= self.height {
                break;
            }
            let n = src.width.min(self.width.saturating_sub(x)) as usize * 4;
            if n == 0 {
                return;
            }
            let s = (row * src.width * 4) as usize;
            let d = ((dy * self.width + x) * 4) as usize;
            self.rgba[d..d + n].copy_from_slice(&src.rgba[s..s + n]);
        }
    }
}

impl From<ove_render::Rgba> for Image {
    fn from(r: ove_render::Rgba) -> Image {
        Image { width: r.w, height: r.h, rgba: r.data }
    }
}

/// Gap between tiles of a contact sheet, in pixels.
pub const SHEET_GAP: u32 = 4;
const SHEET_BACKGROUND: [u8; 4] = [18, 18, 20, 255];

/// A contact sheet: `images` in rows of `columns`, each centred in a cell as large as the largest image.
pub fn sheet(images: &[Image], columns: u32) -> Image {
    if images.is_empty() {
        return Image::filled(1, 1, SHEET_BACKGROUND);
    }
    let columns = columns.clamp(1, images.len() as u32);
    let rows = (images.len() as u32).div_ceil(columns);
    let cw = images.iter().map(|i| i.width).max().unwrap_or(1);
    let ch = images.iter().map(|i| i.height).max().unwrap_or(1);
    let w = columns * cw + (columns + 1) * SHEET_GAP;
    let h = rows * ch + (rows + 1) * SHEET_GAP;
    let mut out = Image::filled(w, h, SHEET_BACKGROUND);
    for (n, img) in images.iter().enumerate() {
        let (c, r) = (n as u32 % columns, n as u32 / columns);
        let x = SHEET_GAP + c * (cw + SHEET_GAP) + (cw - img.width) / 2;
        let y = SHEET_GAP + r * (ch + SHEET_GAP) + (ch - img.height) / 2;
        out.blit(img, x, y);
    }
    out
}

/// Exposure statistics of a frame (all values 0–1).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    /// Mean Rec. 709 luma.
    pub mean_luma: f64,
    /// Share of pixels with luma below 0.06 (near black).
    pub black_fraction: f64,
    /// Share of pixels with luma at or above 0.98 (clipped white).
    pub clipped_fraction: f64,
    pub mean_rgb: [f64; 3],
}

pub fn frame_stats(img: &Image) -> Stats {
    let n = (img.width as u64 * img.height as u64).max(1) as f64;
    let (mut luma, mut black, mut clipped) = (0.0, 0u64, 0u64);
    let mut rgb = [0.0f64; 3];
    for p in img.rgba.chunks_exact(4) {
        let (r, g, b) = (p[0] as f64 / 255.0, p[1] as f64 / 255.0, p[2] as f64 / 255.0);
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        luma += y;
        rgb[0] += r;
        rgb[1] += g;
        rgb[2] += b;
        if y < 0.06 {
            black += 1;
        }
        if y >= 0.98 {
            clipped += 1;
        }
    }
    let r3 = |v: f64| (v * 10_000.0).round() / 10_000.0;
    Stats {
        mean_luma: r3(luma / n),
        black_fraction: r3(black as f64 / n),
        clipped_fraction: r3(clipped as f64 / n),
        mean_rgb: [r3(rgb[0] / n), r3(rgb[1] / n), r3(rgb[2] / n)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_keeps_flat_colour_and_size() {
        let img = Image::filled(64, 36, [200, 100, 50, 255]);
        for (w, h) in [(16, 9), (100, 57), (64, 36), (3, 2)] {
            let r = img.resize(w, h);
            assert_eq!((r.width, r.height), (w, h));
            assert!(r.rgba.chunks_exact(4).all(|p| p == [200, 100, 50, 255]), "{w}x{h}");
        }
        assert_eq!(img.resize_to_width(32).height, 18);
    }

    #[test]
    fn shrink_averages_areas() {
        // Left half black, right half white → 2×1 keeps them apart, 1×1 is grey.
        let mut img = Image::filled(4, 2, [0, 0, 0, 255]);
        for y in 0..2 {
            for x in 2..4 {
                img.put(x, y, [255, 255, 255, 255]);
            }
        }
        let two = img.resize(2, 1);
        assert_eq!((two.px(0, 0)[0], two.px(1, 0)[0]), (0, 255));
        assert_eq!(img.resize(1, 1).px(0, 0)[0], 128);
    }

    #[test]
    fn sheet_layout_and_stats() {
        let tiles: Vec<Image> = (0..5).map(|i| Image::filled(10, 6, [i * 50, 0, 0, 255])).collect();
        let s = sheet(&tiles, 3);
        assert_eq!((s.width, s.height), (3 * 10 + 4 * SHEET_GAP, 2 * 6 + 3 * SHEET_GAP));
        assert_eq!(s.px(SHEET_GAP + 14 + SHEET_GAP, SHEET_GAP), [50, 0, 0, 255]);
        let st = frame_stats(&Image::filled(4, 4, [255, 255, 255, 255]));
        assert_eq!((st.mean_luma, st.clipped_fraction, st.black_fraction), (1.0, 1.0, 0.0));
        let png = s.png();
        assert_eq!(&png[1..4], b"PNG");
    }
}
