//! Viewer guides drawn into agent frames (`look --overlay`, add-viewer-interaction-and-text-fills design D4): grids,
//! the action-safe and title-safe areas, a centre cross and format masks — the same guides, with the same numbers,
//! as the app's viewer (apps/desktop/ui/src/overlay/guides.ts). Only analysis frames get them; export never draws
//! them.

use crate::image::Image;

/// Inset of the action-safe area from each edge, as a fraction of the frame.
pub const ACTION_SAFE: f64 = 0.035;
/// Inset of the title-safe area from each edge.
pub const TITLE_SAFE: f64 = 0.05;
/// The format masks.
pub const MASKS: [&str; 5] = ["2.39:1", "1.85:1", "1:1", "9:16", "4:5"];

const GRID: [u8; 4] = [255, 255, 255, 255];
const ACTION: [u8; 4] = [0, 200, 255, 255];
const TITLE: [u8; 4] = [0, 255, 140, 255];
const CENTRE: [u8; 4] = [255, 64, 64, 255];
const MASK_EDGE: [u8; 4] = [255, 255, 255, 255];

/// One guide.
#[derive(Debug, Clone, PartialEq)]
pub enum Guide {
    /// The action-safe and title-safe areas.
    Safe,
    /// A grid of `n` divisions per side (3 is the rule of thirds).
    Grid(u32),
    /// A cross at the centre.
    Centre,
    /// Everything outside the centred `w:h` area darkened.
    Mask { name: String, ratio: f64 },
}

/// Parses `safe,grid,thirds,grid:4,centre,mask:9:16` (comma-separated). `grid` alone is the rule of thirds.
pub fn parse(spec: &str) -> Result<Vec<Guide>, String> {
    let mut out = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (name, arg) = part.split_once(':').unwrap_or((part, ""));
        let guide = match (name, arg) {
            ("safe", "") => Guide::Safe,
            ("thirds", "") | ("grid", "") => Guide::Grid(3),
            ("grid", n) => match n.parse::<u32>() {
                Ok(n) if (2..=5).contains(&n) => Guide::Grid(n),
                _ => return Err(format!("grid takes 2 to 5 divisions, not {n}")),
            },
            ("centre" | "center", "") => Guide::Centre,
            ("mask", m) if MASKS.contains(&m) => {
                let (a, b) = m.split_once(':').unwrap_or((m, "1"));
                let ratio = a.parse::<f64>().unwrap_or(1.0) / b.parse::<f64>().unwrap_or(1.0);
                Guide::Mask { name: m.to_string(), ratio }
            }
            ("mask", m) => return Err(format!("there is no mask {m}; masks: {}", MASKS.join(", "))),
            _ => {
                return Err(format!(
                    "there is no overlay {part}; overlays: safe, grid, grid:<2-5>, thirds, centre, mask:<{}>",
                    MASKS.join("|")
                ));
            }
        };
        if !out.contains(&guide) {
            out.push(guide);
        }
    }
    if out.is_empty() {
        return Err("name at least one overlay".into());
    }
    Ok(out)
}

/// The part of a `w`×`h` frame a mask of `ratio` (width / height) keeps: `[x0, y0, x1, y1]`, centred.
pub fn mask_rect(ratio: f64, w: f64, h: f64) -> [f64; 4] {
    if ratio >= w / h {
        let mh = w / ratio;
        [0.0, (h - mh) / 2.0, w, (h + mh) / 2.0]
    } else {
        let mw = h * ratio;
        [(w - mw) / 2.0, 0.0, (w + mw) / 2.0, h]
    }
}

fn blend(img: &mut Image, x: i64, y: i64, c: [u8; 4], a: f32) {
    if x < 0 || y < 0 || x >= img.width as i64 || y >= img.height as i64 {
        return;
    }
    let (x, y) = (x as u32, y as u32);
    let o = img.px(x, y);
    let mix = |i: usize| (o[i] as f32 * (1.0 - a) + c[i] as f32 * a).round() as u8;
    img.put(x, y, [mix(0), mix(1), mix(2), o[3]]);
}

fn rect(img: &mut Image, [x0, y0, x1, y1]: [f64; 4], c: [u8; 4], a: f32) {
    let (x0, y0) = (x0.round() as i64, y0.round() as i64);
    let (x1, y1) = (x1.round() as i64 - 1, y1.round() as i64 - 1);
    for x in x0..=x1 {
        blend(img, x, y0, c, a);
        blend(img, x, y1, c, a);
    }
    for y in y0 + 1..y1 {
        blend(img, x0, y, c, a);
        blend(img, x1, y, c, a);
    }
}

/// Draws `guides` over `img`.
pub fn draw(img: &mut Image, guides: &[Guide]) {
    let (w, h) = (img.width as f64, img.height as f64);
    // Masks first, so the lines stay visible over the darkened area.
    for g in guides {
        if let Guide::Mask { ratio, .. } = g {
            let [x0, y0, x1, y1] = mask_rect(*ratio, w, h);
            for y in 0..img.height {
                for x in 0..img.width {
                    let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
                    if cx < x0 || cx >= x1 || cy < y0 || cy >= y1 {
                        blend(img, x as i64, y as i64, [0, 0, 0, 255], 0.6);
                    }
                }
            }
            rect(img, [x0, y0, x1, y1], MASK_EDGE, 0.7);
        }
    }
    for g in guides {
        match g {
            Guide::Grid(n) => {
                for k in 1..*n {
                    let x = (w * k as f64 / *n as f64).round() as i64;
                    let y = (h * k as f64 / *n as f64).round() as i64;
                    for yy in 0..img.height as i64 {
                        blend(img, x, yy, GRID, 0.5);
                    }
                    for xx in 0..img.width as i64 {
                        blend(img, xx, y, GRID, 0.5);
                    }
                }
            }
            Guide::Safe => {
                for (f, c) in [(ACTION_SAFE, ACTION), (TITLE_SAFE, TITLE)] {
                    rect(img, [w * f, h * f, w * (1.0 - f), h * (1.0 - f)], c, 0.9);
                }
            }
            Guide::Centre => {
                let (cx, cy) = ((w / 2.0).floor() as i64, (h / 2.0).floor() as i64);
                let arm = (w.min(h) * 0.03).max(4.0) as i64;
                for d in -arm..=arm {
                    blend(img, cx + d, cy, CENTRE, 0.9);
                    blend(img, cx, cy + d, CENTRE, 0.9);
                }
            }
            Guide::Mask { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_overlays() {
        assert_eq!(parse("safe,grid").unwrap(), vec![Guide::Safe, Guide::Grid(3)]);
        assert_eq!(parse("grid:4, centre, thirds").unwrap(), vec![Guide::Grid(4), Guide::Centre, Guide::Grid(3)]);
        assert_eq!(parse("mask:9:16").unwrap(), vec![Guide::Mask { name: "9:16".into(), ratio: 0.5625 }]);
        assert!(parse("mask:3:1").unwrap_err().contains("masks: 2.39:1"));
        assert!(parse("grid:9").is_err());
        assert!(parse("sparkles").unwrap_err().contains("overlays: safe"));
        assert!(parse("").is_err());
    }

    #[test]
    fn masks_keep_the_centre() {
        assert_eq!(mask_rect(1.0, 1920.0, 1080.0), [420.0, 0.0, 1500.0, 1080.0]);
        let [_, y0, _, y1] = mask_rect(2.39, 1920.0, 1080.0);
        assert!((y1 - y0 - 1920.0 / 2.39).abs() < 1e-9);
    }
}
