//! Layouts: 15 presets of normalised slot rectangles (x, y, w, h in 0..1 of the frame) that items fill.
//!
//! An item joins a layout with `layout {preset, slot}`. Items on a `layout` track without that field are assigned
//! automatically: slots in start order among the items showing at that moment, and the preset picked by how many
//! there are (2 → split-horizontal, 3 → 3up-horizontal, 4+ → grid-2x2) unless one of them names a preset.
//! A slotted item covers its rectangle (scaled to fill, centred, cropped to the slot); its transform then offsets
//! and scales it within the slot.

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub slots: &'static [[f32; 4]],
}

const PIP: f32 = 0.3;
const M: f32 = 0.04;

const fn l(id: &'static str, label: &'static str, group: &'static str, slots: &'static [[f32; 4]]) -> LayoutSpec {
    LayoutSpec { id, label, group, slots }
}

const T: f32 = 1.0 / 3.0;

#[rustfmt::skip]
pub const LAYOUTS: &[LayoutSpec] = &[
    l("split-horizontal", "Split side by side", "split", &[[0.0, 0.0, 0.5, 1.0], [0.5, 0.0, 0.5, 1.0]]),
    l("split-vertical", "Split top and bottom", "split", &[[0.0, 0.0, 1.0, 0.5], [0.0, 0.5, 1.0, 0.5]]),
    l("3up-horizontal", "Three across", "split", &[[0.0, 0.0, T, 1.0], [T, 0.0, T, 1.0], [2.0 * T, 0.0, T, 1.0]]),
    l("3up-vertical", "Three stacked", "split", &[[0.0, 0.0, 1.0, T], [0.0, T, 1.0, T], [0.0, 2.0 * T, 1.0, T]]),
    l("4up-horizontal", "Four across", "split", &[[0.0, 0.0, 0.25, 1.0], [0.25, 0.0, 0.25, 1.0], [0.5, 0.0, 0.25, 1.0], [0.75, 0.0, 0.25, 1.0]]),
    l("4up-vertical", "Four stacked", "split", &[[0.0, 0.0, 1.0, 0.25], [0.0, 0.25, 1.0, 0.25], [0.0, 0.5, 1.0, 0.25], [0.0, 0.75, 1.0, 0.25]]),
    l("grid-2x2", "Grid 2×2", "grid", &[[0.0, 0.0, 0.5, 0.5], [0.5, 0.0, 0.5, 0.5], [0.0, 0.5, 0.5, 0.5], [0.5, 0.5, 0.5, 0.5]]),
    l("pip-bottom-right", "Picture in picture, bottom right", "pip", &[[0.0, 0.0, 1.0, 1.0], [1.0 - M - PIP, 1.0 - M - PIP, PIP, PIP]]),
    l("pip-bottom-left", "Picture in picture, bottom left", "pip", &[[0.0, 0.0, 1.0, 1.0], [M, 1.0 - M - PIP, PIP, PIP]]),
    l("pip-top-right", "Picture in picture, top right", "pip", &[[0.0, 0.0, 1.0, 1.0], [1.0 - M - PIP, M, PIP, PIP]]),
    l("pip-top-left", "Picture in picture, top left", "pip", &[[0.0, 0.0, 1.0, 1.0], [M, M, PIP, PIP]]),
    l("hero-top", "Hero on top", "hero", &[[0.0, 0.0, 1.0, 2.0 * T], [0.0, 2.0 * T, 0.5, T], [0.5, 2.0 * T, 0.5, T]]),
    l("hero-bottom", "Hero at the bottom", "hero", &[[0.0, T, 1.0, 2.0 * T], [0.0, 0.0, 0.5, T], [0.5, 0.0, 0.5, T]]),
    l("hero-left", "Hero on the left", "hero", &[[0.0, 0.0, 2.0 * T, 1.0], [2.0 * T, 0.0, T, 0.5], [2.0 * T, 0.5, T, 0.5]]),
    l("hero-right", "Hero on the right", "hero", &[[T, 0.0, 2.0 * T, 1.0], [0.0, 0.0, T, 0.5], [0.0, 0.5, T, 0.5]]),
];

pub fn find(id: &str) -> Option<&'static LayoutSpec> {
    LAYOUTS.iter().find(|l| l.id == id)
}

/// The preset used for `n` automatically arranged items on a layout track.
pub fn auto_preset(n: usize) -> &'static str {
    match n {
        0 | 1 => "",
        2 => "split-horizontal",
        3 => "3up-horizontal",
        _ => "grid-2x2",
    }
}

/// Slot rectangle of `slot` in `preset` (slots past the end wrap around).
pub fn slot_rect(preset: &str, slot: usize) -> Option<[f32; 4]> {
    let l = find(preset)?;
    Some(l.slots[slot % l.slots.len()])
}

/// Scale and crop (top, right, bottom, left in percent) that make a `fw`×`fh` source cover a slot of `sw`×`sh`
/// output pixels, centred. `zoom` (the item's own scale, ≥ 0) enlarges the source within the slot.
pub fn cover(fw: f64, fh: f64, sw: f64, sh: f64, zoom: f64) -> (f64, [f32; 4]) {
    let s = (sw / fw).max(sh / fh) * zoom.max(1e-3);
    let vis_w = (sw / s).min(fw);
    let vis_h = (sh / s).min(fh);
    let cx = ((fw - vis_w) / 2.0 / fw * 100.0) as f32;
    let cy = ((fh - vis_h) / 2.0 / fh * 100.0) as f32;
    (s, [cy, cx, cy, cx])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fifteen_layouts_with_slots_inside_the_frame() {
        assert_eq!(LAYOUTS.len(), 15);
        for l in LAYOUTS {
            assert!(!l.slots.is_empty());
            for s in l.slots {
                assert!(s[0] >= 0.0 && s[1] >= 0.0 && s[0] + s[2] <= 1.0001 && s[1] + s[3] <= 1.0001, "{}", l.id);
            }
        }
        assert_eq!(slot_rect("grid-2x2", 5), Some([0.5, 0.0, 0.5, 0.5]));
    }

    #[test]
    fn cover_fills_the_slot() {
        // 16:9 source into a half-width slot of a 16:9 frame (960×1080 at 1080p): crop the sides.
        let (s, crop) = cover(1920.0, 1080.0, 960.0, 1080.0, 1.0);
        assert!((s - 1.0).abs() < 1e-9);
        assert!((crop[1] - 25.0).abs() < 1e-4 && crop[0].abs() < 1e-4);
    }
}
