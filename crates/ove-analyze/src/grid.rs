//! A coordinate grid for frames an agent inspects (unify-agent-tool-layer D6). Lines and labels are in the units item
//! transforms use: reference pixels from the canvas centre, x to the right and y down. The title-safe (80 %) and
//! action-safe (90 %) boxes are drawn too. Only analysis frames get it; export never draws it.

use crate::image::Image;

/// Digits, lower-case letters (upper case draws as lower case), `-`, `_`, `:` and `.` as 3×5 bitmaps, one row
/// per 3 bits (top row first). Our own glyphs.
const GLYPHS: &[(char, [u8; 5])] = &[
    ('0', [0b111, 0b101, 0b101, 0b101, 0b111]),
    ('1', [0b010, 0b110, 0b010, 0b010, 0b111]),
    ('2', [0b111, 0b001, 0b111, 0b100, 0b111]),
    ('3', [0b111, 0b001, 0b011, 0b001, 0b111]),
    ('4', [0b101, 0b101, 0b111, 0b001, 0b001]),
    ('5', [0b111, 0b100, 0b111, 0b001, 0b111]),
    ('6', [0b111, 0b100, 0b111, 0b101, 0b111]),
    ('7', [0b111, 0b001, 0b010, 0b010, 0b010]),
    ('8', [0b111, 0b101, 0b111, 0b101, 0b111]),
    ('9', [0b111, 0b101, 0b111, 0b001, 0b111]),
    ('-', [0b000, 0b000, 0b111, 0b000, 0b000]),
    ('a', [0b010, 0b101, 0b111, 0b101, 0b101]),
    ('b', [0b110, 0b101, 0b110, 0b101, 0b110]),
    ('c', [0b011, 0b100, 0b100, 0b100, 0b011]),
    ('d', [0b110, 0b101, 0b101, 0b101, 0b110]),
    ('e', [0b111, 0b100, 0b110, 0b100, 0b111]),
    ('f', [0b111, 0b100, 0b110, 0b100, 0b100]),
    ('g', [0b011, 0b100, 0b101, 0b101, 0b011]),
    ('h', [0b101, 0b101, 0b111, 0b101, 0b101]),
    ('i', [0b111, 0b010, 0b010, 0b010, 0b111]),
    ('j', [0b001, 0b001, 0b001, 0b101, 0b010]),
    ('k', [0b101, 0b101, 0b110, 0b101, 0b101]),
    ('l', [0b100, 0b100, 0b100, 0b100, 0b111]),
    ('m', [0b101, 0b111, 0b111, 0b101, 0b101]),
    ('n', [0b110, 0b101, 0b101, 0b101, 0b101]),
    ('o', [0b010, 0b101, 0b101, 0b101, 0b010]),
    ('p', [0b110, 0b101, 0b110, 0b100, 0b100]),
    ('q', [0b010, 0b101, 0b101, 0b110, 0b011]),
    ('r', [0b110, 0b101, 0b110, 0b101, 0b101]),
    ('s', [0b011, 0b100, 0b010, 0b001, 0b110]),
    ('t', [0b111, 0b010, 0b010, 0b010, 0b010]),
    ('u', [0b101, 0b101, 0b101, 0b101, 0b111]),
    ('v', [0b101, 0b101, 0b101, 0b101, 0b010]),
    ('w', [0b101, 0b101, 0b111, 0b111, 0b101]),
    ('x', [0b101, 0b101, 0b010, 0b101, 0b101]),
    ('y', [0b101, 0b101, 0b010, 0b010, 0b010]),
    ('z', [0b111, 0b001, 0b010, 0b100, 0b111]),
    ('_', [0b000, 0b000, 0b000, 0b000, 0b111]),
    (':', [0b000, 0b010, 0b000, 0b010, 0b000]),
    ('.', [0b000, 0b000, 0b000, 0b000, 0b010]),
];

const LINE: [u8; 4] = [255, 214, 0, 255];
const CENTRE: [u8; 4] = [255, 64, 64, 255];
const ACTION: [u8; 4] = [0, 200, 255, 255];
const TITLE: [u8; 4] = [0, 255, 140, 255];

/// Grid spacing in reference units: a twelfth of the canvas width (160 on a 1920-wide canvas, 90 on 1080).
pub fn step(reference: (u32, u32)) -> u32 {
    (reference.0 / 12).max(1)
}

/// Grid line positions in reference units from the centre, for one axis of `half` reference units.
pub fn lines(half: f64, step: u32) -> Vec<i64> {
    let n = (half / step as f64).floor() as i64;
    (-n..=n).map(|k| k * step as i64).collect()
}

fn blend(img: &mut Image, x: i64, y: i64, c: [u8; 4], a: f32) {
    if x < 0 || y < 0 || x >= img.width as i64 || y >= img.height as i64 {
        return;
    }
    let (x, y) = (x as u32, y as u32);
    let o = img.px(x, y);
    let mix = |i: usize| (o[i] as f32 * (1.0 - a) + c[i] as f32 * a).round() as u8;
    img.put(x, y, [mix(0), mix(1), mix(2), 255]);
}

fn vline(img: &mut Image, x: i64, c: [u8; 4], a: f32) {
    for y in 0..img.height as i64 {
        blend(img, x, y, c, a);
    }
}

fn hline(img: &mut Image, y: i64, c: [u8; 4], a: f32) {
    for x in 0..img.width as i64 {
        blend(img, x, y, c, a);
    }
}

fn rect(img: &mut Image, x0: i64, y0: i64, x1: i64, y1: i64, c: [u8; 4]) {
    for x in x0..=x1 {
        blend(img, x, y0, c, 0.9);
        blend(img, x, y1, c, 0.9);
    }
    for y in y0..=y1 {
        blend(img, x0, y, c, 0.9);
        blend(img, x1, y, c, 0.9);
    }
}

/// Draws `text` with its top-left corner at (x, y), each glyph pixel `s`×`s`, white on a dark outline.
pub fn label(img: &mut Image, text: &str, x: i64, y: i64, s: i64) {
    for (pass, colour) in [(0, [0, 0, 0, 255]), (1, [255, 255, 255, 255])] {
        let mut cx = x;
        for ch in text.chars() {
            let ch = ch.to_ascii_lowercase();
            let Some((_, rows)) = GLYPHS.iter().find(|(g, _)| *g == ch) else {
                cx += 4 * s;
                continue;
            };
            for (r, bits) in rows.iter().enumerate() {
                for col in 0..3 {
                    if bits & (0b100 >> col) == 0 {
                        continue;
                    }
                    let (px, py) = (cx + col * s, y + r as i64 * s);
                    let grow = if pass == 0 { 1 } else { 0 };
                    for dy in -grow..s + grow {
                        for dx in -grow..s + grow {
                            blend(img, px + dx, py + dy, colour, 1.0);
                        }
                    }
                }
            }
            cx += 4 * s;
        }
    }
}

/// Width in pixels of a label drawn at glyph size `s`.
pub fn label_width(text: &str, s: i64) -> i64 {
    text.chars().count() as i64 * 4 * s - s
}

/// Draws the grid onto a frame of a canvas whose reference size is `reference` (the units item transforms use).
pub fn overlay(img: &mut Image, reference: (u32, u32)) {
    let (w, h) = (img.width as f64, img.height as f64);
    let k = w / reference.0 as f64;
    let st = step(reference);
    let to_x = |u: i64| (w / 2.0 + u as f64 * k).round() as i64;
    let to_y = |u: i64| (h / 2.0 + u as f64 * k).round() as i64;
    let s = ((img.width / 480).max(1)) as i64;
    let xs = lines(reference.0 as f64 / 2.0, st);
    let ys = lines(reference.1 as f64 / 2.0, st);
    for &u in &xs {
        vline(img, to_x(u), if u == 0 { CENTRE } else { LINE }, if u == 0 { 0.8 } else { 0.45 });
    }
    for &u in &ys {
        hline(img, to_y(u), if u == 0 { CENTRE } else { LINE }, if u == 0 { 0.8 } else { 0.45 });
    }
    // Safe areas: action 90 %, title 80 % of the frame.
    for (share, c) in [(0.05, ACTION), (0.10, TITLE)] {
        let (mx, my) = ((w * share).round() as i64, (h * share).round() as i64);
        rect(img, mx, my, img.width as i64 - 1 - mx, img.height as i64 - 1 - my, c);
    }
    // Labels: x values along the top, y values down the left, each next to its line.
    for &u in &xs {
        let t = u.to_string();
        let x = to_x(u) + 2 * s;
        if x + label_width(&t, s) < img.width as i64 {
            label(img, &t, x, 2 * s, s);
        }
    }
    for &u in &ys {
        if u == 0 {
            continue;
        }
        label(img, &u.to_string(), 2 * s, to_y(u) + 2 * s, s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_fall_on_multiples_of_the_step() {
        assert_eq!(step((1920, 1080)), 160);
        assert_eq!(step((1080, 1920)), 90);
        assert_eq!(lines(960.0, 160), vec![-960, -800, -640, -480, -320, -160, 0, 160, 320, 480, 640, 800, 960]);
    }

    #[test]
    fn the_line_for_a_transform_value_is_where_the_renderer_puts_that_centre() {
        // 640 px wide frame of a 1920-unit canvas: x = -320 is 640/2 − 320 × (640/1920) ≈ 213.
        let mut img = Image::filled(640, 360, [0, 0, 0, 255]);
        overlay(&mut img, (1920, 1080));
        let at = |x: u32| img.px(x, 100);
        assert_ne!(at(213), [0, 0, 0, 255], "a line at x = -320");
        assert_eq!(at(240), [0, 0, 0, 255], "nothing between lines");
    }

    #[test]
    fn glyphs_are_distinct() {
        for (i, a) in GLYPHS.iter().enumerate() {
            for b in &GLYPHS[i + 1..] {
                assert_ne!(a.1, b.1, "{} and {}", a.0, b.0);
            }
        }
    }
}
