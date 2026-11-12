//! Text and caption rasterisation: word layout with wrapping and alignment, stroke, shadow, background box and a
//! highlighted word. The default face is the bundled Inter (SIL OFL 1.1, assets/fonts); `TextStyle::font` picks any
//! family from the font library (see `fonts`).

use std::sync::OnceLock;

use ab_glyph::{Font, FontArc, PxScale, ScaleFont, point};

use crate::canvas::Rgba;
use crate::fonts::{FontSpec, face};

struct Fonts {
    regular: FontArc,
    bold: FontArc,
}

#[allow(clippy::expect_used, reason = "both fonts are compiled in and parsed by the crate's tests")]
fn fonts() -> &'static Fonts {
    static F: OnceLock<Fonts> = OnceLock::new();
    F.get_or_init(|| Fonts {
        regular: FontArc::try_from_slice(include_bytes!("../../../assets/fonts/Inter-Regular.otf")).expect("font"),
        bold: FontArc::try_from_slice(include_bytes!("../../../assets/fonts/Inter-Bold.otf")).expect("font"),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    /// Font size in output pixels.
    pub size: f32,
    pub color: [u8; 4],
    pub bold: bool,
    pub align: Align,
    pub line_height: f32,
    pub letter_spacing: f32,
    /// (colour, width in pixels)
    pub stroke: Option<([u8; 4], f32)>,
    /// (colour, dx, dy) in pixels
    pub shadow: Option<([u8; 4], f32, f32)>,
    /// (colour incl. opacity, padding in pixels)
    pub background: Option<([u8; 4], f32)>,
    /// Wrap width in pixels.
    pub max_width: f32,
    /// (word index, colour)
    pub highlight: Option<(usize, [u8; 4])>,
    /// Font family/weight/style; `None` is Inter regular or bold (by `bold`).
    pub font: Option<FontSpec>,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            size: 48.0,
            color: [255, 255, 255, 255],
            bold: false,
            align: Align::Center,
            line_height: 1.2,
            letter_spacing: 0.0,
            stroke: None,
            shadow: None,
            background: None,
            max_width: 1e9,
            highlight: None,
            font: None,
        }
    }
}

/// Parses `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb(r,g,b)`, `rgba(r,g,b,a)` and a few names.
pub fn parse_color(s: &str) -> Option<[u8; 4]> {
    let s = s.trim().to_ascii_lowercase();
    if let Some(h) = s.strip_prefix('#') {
        let v = |i: usize, n: usize| u8::from_str_radix(&h[i..i + n], 16).ok();
        return match h.len() {
            3 => Some([v(0, 1)? * 17, v(1, 1)? * 17, v(2, 1)? * 17, 255]),
            6 => Some([v(0, 2)?, v(2, 2)?, v(4, 2)?, 255]),
            8 => Some([v(0, 2)?, v(2, 2)?, v(4, 2)?, v(6, 2)?]),
            _ => None,
        };
    }
    if let Some(inner) = s.strip_prefix("rgba(").or_else(|| s.strip_prefix("rgb(")).and_then(|r| r.strip_suffix(')')) {
        let parts: Vec<f32> = inner.split(',').filter_map(|p| p.trim().parse().ok()).collect();
        if parts.len() >= 3 {
            let a = parts.get(3).map(|a| if *a <= 1.0 { a * 255.0 } else { *a }).unwrap_or(255.0);
            return Some([parts[0] as u8, parts[1] as u8, parts[2] as u8, a.clamp(0.0, 255.0) as u8]);
        }
        return None;
    }
    Some(match s.as_str() {
        "white" => [255, 255, 255, 255],
        "black" => [0, 0, 0, 255],
        "red" => [229, 57, 53, 255],
        "yellow" => [255, 221, 0, 255],
        "green" => [67, 160, 71, 255],
        "blue" => [30, 136, 229, 255],
        "transparent" => [0, 0, 0, 0],
        _ => return None,
    })
}

struct Placed {
    word: usize,
    x: f32,
    line: usize,
}

/// Lays out words: returns (word positions, line widths, line count).
fn layout(words: &[String], st: &TextStyle, font: &FontArc) -> (Vec<Placed>, Vec<f32>, Vec<Vec<f32>>) {
    let sf = font.as_scaled(PxScale::from(st.size));
    let width = |w: &str| -> f32 {
        let mut x = 0.0;
        let mut prev = None;
        for c in w.chars() {
            let id = font.glyph_id(c);
            if let Some(p) = prev {
                x += sf.kern(p, id);
            }
            x += sf.h_advance(id) + st.letter_spacing;
            prev = Some(id);
        }
        x
    };
    let space = sf.h_advance(font.glyph_id(' ')) + st.letter_spacing;
    let mut placed = Vec::new();
    let mut lines: Vec<f32> = vec![0.0];
    let mut advances: Vec<Vec<f32>> = Vec::new();
    let mut line = 0;
    for (i, w) in words.iter().enumerate() {
        if w == "\n" {
            line += 1;
            lines.push(0.0);
            continue;
        }
        let ww = width(w);
        let cur = lines[line];
        if cur > 0.0 && cur + space + ww > st.max_width {
            line += 1;
            lines.push(0.0);
        }
        let x = if lines[line] > 0.0 { lines[line] + space } else { 0.0 };
        placed.push(Placed { word: i, x, line });
        lines[line] = x + ww;
        advances.push(vec![ww]);
    }
    (placed, lines, advances)
}

/// Splits text into words, keeping explicit line breaks as `"\n"` tokens.
pub fn words_of(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, para) in text.split('\n').enumerate() {
        if i > 0 {
            out.push("\n".to_string());
        }
        out.extend(para.split_whitespace().map(str::to_string));
    }
    out
}

/// Renders words into a tight RGBA image (with padding for stroke, shadow and background).
pub fn render_words(words: &[String], st: &TextStyle) -> Rgba {
    render_words_partial(words, st, usize::MAX)
}

/// Like [`render_words`], laying out every word but drawing only the first `reveal` characters (typewriter).
pub fn render_words_partial(words: &[String], st: &TextStyle, reveal: usize) -> Rgba {
    let f = fonts();
    let chosen = st.font.as_ref().map(face);
    let font = match &chosen {
        Some(c) => c,
        None if st.bold => &f.bold,
        None => &f.regular,
    };
    let (placed, lines, _) = layout(words, st, font);
    let sf = font.as_scaled(PxScale::from(st.size));
    let lh = st.size * st.line_height;
    let stroke_w = st.stroke.map(|s| s.1).unwrap_or(0.0);
    let (sdx, sdy) = st.shadow.map(|s| (s.1, s.2)).unwrap_or((0.0, 0.0));
    let pad = st.background.map(|b| b.1).unwrap_or(0.0) + stroke_w + sdx.abs().max(sdy.abs()) + 2.0;
    let text_w = lines.iter().cloned().fold(0.0, f32::max);
    let text_h = lh * lines.len() as f32;
    let (w, h) = ((text_w + 2.0 * pad).ceil().max(1.0) as u32, (text_h + 2.0 * pad).ceil().max(1.0) as u32);
    let n = (w * h) as usize;
    let mut fill = vec![0f32; n];
    let mut hi = vec![0f32; n];
    let mut stroke = vec![0f32; n];
    let mut shadow = vec![0f32; n];
    let line_x = |line: usize| match st.align {
        Align::Left => pad,
        Align::Center => pad + (text_w - lines[line]) / 2.0,
        Align::Right => pad + text_w - lines[line],
    };
    let offsets: Vec<(f32, f32)> = if stroke_w > 0.0 {
        (0..16)
            .map(|k| k as f32 * std::f32::consts::TAU / 16.0)
            .map(|a| (a.cos() * stroke_w, a.sin() * stroke_w))
            .collect()
    } else {
        Vec::new()
    };
    let raster = |mask: &mut Vec<f32>, word: &str, x0: f32, base: f32| {
        let mut x = x0;
        let mut prev = None;
        for c in word.chars() {
            let id = font.glyph_id(c);
            if let Some(p) = prev {
                x += sf.kern(p, id);
            }
            let g = id.with_scale_and_position(PxScale::from(st.size), point(x, base));
            if let Some(o) = font.outline_glyph(g) {
                let b = o.px_bounds();
                o.draw(|gx, gy, cov| {
                    let px = b.min.x as i32 + gx as i32;
                    let py = b.min.y as i32 + gy as i32;
                    if px >= 0 && py >= 0 && (px as u32) < w && (py as u32) < h {
                        let i = (py as u32 * w + px as u32) as usize;
                        mask[i] = (mask[i] + cov).min(1.0);
                    }
                });
            }
            x += sf.h_advance(id) + st.letter_spacing;
            prev = Some(id);
        }
    };
    let mut budget = reveal;
    for p in &placed {
        if budget == 0 {
            break;
        }
        let x = line_x(p.line) + p.x;
        let base = pad + p.line as f32 * lh + (lh - st.size) / 2.0 + sf.ascent();
        let full = &words[p.word];
        let n = full.chars().count();
        let word: &str =
            if n <= budget { full } else { &full[..full.char_indices().nth(budget).map_or(full.len(), |c| c.0)] };
        budget = budget.saturating_sub(n + 1);
        let is_hi = st.highlight.is_some_and(|(i, _)| i == p.word);
        raster(if is_hi { &mut hi } else { &mut fill }, word, x, base);
        for (ox, oy) in &offsets {
            raster(&mut stroke, word, x + ox, base + oy);
        }
        if st.shadow.is_some() {
            raster(&mut shadow, word, x + sdx, base + sdy);
        }
    }
    let mut img = Rgba::new(w, h);
    let paint = |img: &mut Rgba, mask: &[f32], c: [u8; 4]| {
        for (i, &m) in mask.iter().enumerate() {
            let a = m * c[3] as f32 / 255.0;
            if a <= 0.0 {
                continue;
            }
            let d = &mut img.data[i * 4..i * 4 + 4];
            let da = d[3] as f32 / 255.0;
            let oa = a + da * (1.0 - a);
            for k in 0..3 {
                let v = (c[k] as f32 * a + d[k] as f32 * da * (1.0 - a)) / oa.max(1e-6);
                d[k] = v.clamp(0.0, 255.0) as u8;
            }
            d[3] = (oa * 255.0).clamp(0.0, 255.0) as u8;
        }
    };
    if let Some((c, _)) = st.background {
        let full = vec![1f32; n];
        paint(&mut img, &full, c);
    }
    if let Some((c, _, _)) = st.shadow {
        paint(&mut img, &shadow, c);
    }
    if let Some((c, _)) = st.stroke {
        paint(&mut img, &stroke, c);
    }
    paint(&mut img, &fill, st.color);
    if let Some((_, c)) = st.highlight {
        paint(&mut img, &hi, c);
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        assert_eq!(parse_color("#fff"), Some([255, 255, 255, 255]));
        assert_eq!(parse_color("#FFDD00"), Some([255, 221, 0, 255]));
        assert_eq!(parse_color("rgba(0, 0, 0, 0.5)"), Some([0, 0, 0, 127]));
        assert_eq!(parse_color("nope"), None);
    }

    #[test]
    fn text_renders_ink_and_wraps() {
        let st = TextStyle { size: 40.0, ..Default::default() };
        let img = render_words(&words_of("Hello world"), &st);
        assert!(img.w > 150 && img.h > 40, "{}x{}", img.w, img.h);
        let ink = img.data.chunks(4).filter(|p| p[3] > 128).count();
        assert!(ink > 300, "ink {ink}");
        let narrow = render_words(&words_of("Hello world"), &TextStyle { max_width: 120.0, ..st.clone() });
        assert!(narrow.h > img.h, "wrapped to two lines");
        let boxed = render_words(&words_of("Hi"), &TextStyle { background: Some(([0, 0, 0, 255], 10.0)), ..st });
        assert_eq!(boxed.px(1, 1), [0, 0, 0, 255]);
    }
}
