//! Main-window placement: first-launch default, restore, and clamping to connected displays.
//!
//! Pure logic (no Tauri types) so every scenario is unit-tested; `apply`/`capture` in lib.rs
//! translate to and from the window.

use serde::{Deserialize, Serialize};

/// Default window size in logical pixels (spec: Application window).
pub const DEFAULT_SIZE: (f64, f64) = (1440.0, 900.0);
/// Minimum window size in logical pixels.
pub const MIN_SIZE: (f64, f64) = (1024.0, 640.0);
/// How much of the window must overlap a display's work area to count as "on" it.
const MIN_VISIBLE_PX: i64 = 100;

/// Rectangle in physical (device) pixels in the desktop coordinate space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    fn right(&self) -> i64 {
        i64::from(self.x) + i64::from(self.width)
    }
    fn bottom(&self) -> i64 {
        i64::from(self.y) + i64::from(self.height)
    }
    fn overlap(&self, other: &Rect) -> (i64, i64) {
        let w = self.right().min(other.right()) - i64::from(self.x.max(other.x));
        let h = self.bottom().min(other.bottom()) - i64::from(self.y.max(other.y));
        (w.max(0), h.max(0))
    }
}

/// A connected display.
#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    pub name: String,
    pub work_area: Rect,
    pub scale: f64,
    pub primary: bool,
}

/// What is persisted between launches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedState {
    pub monitor: String,
    /// Outer position and size in physical pixels.
    pub rect: Rect,
    /// Scale factor of the display the window was on, to carry the logical size elsewhere.
    pub scale: f64,
    #[serde(default)]
    pub maximized: bool,
}

/// Where to put the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub rect: Rect,
    pub maximized: bool,
}

fn clamp_size(width: f64, height: f64, area: &Rect, scale: f64) -> (u32, u32) {
    let min_w = (MIN_SIZE.0 * scale).round();
    let min_h = (MIN_SIZE.1 * scale).round();
    let w = width.max(min_w).min(f64::from(area.width));
    let h = height.max(min_h).min(f64::from(area.height));
    (w.round() as u32, h.round() as u32)
}

fn centred(width: u32, height: u32, area: &Rect) -> Rect {
    Rect {
        x: area.x + ((area.width - width.min(area.width)) / 2) as i32,
        y: area.y + ((area.height - height.min(area.height)) / 2) as i32,
        width,
        height,
    }
}

/// Moves `rect` the least distance needed to lie fully inside `area` (size already ≤ area).
fn shift_inside(rect: Rect, area: &Rect) -> Rect {
    let max_x = area.right() - i64::from(rect.width);
    let max_y = area.bottom() - i64::from(rect.height);
    Rect {
        x: i64::from(rect.x).clamp(i64::from(area.x), max_x.max(i64::from(area.x))) as i32,
        y: i64::from(rect.y).clamp(i64::from(area.y), max_y.max(i64::from(area.y))) as i32,
        ..rect
    }
}

/// Decides the initial window placement.
///
/// - No saved state: 1440×900 logical (or the work area, if smaller), centred on the primary display.
/// - Saved display still connected and the window visibly on it: restore, clamped fully inside its
///   work area.
/// - Saved display gone (or window off-screen): the last logical size on the primary display,
///   clamped to its work area and centred.
pub fn initial_placement(saved: Option<&SavedState>, monitors: &[Monitor]) -> Placement {
    let Some(primary) = monitors.iter().find(|m| m.primary).or_else(|| monitors.first()) else {
        // No display information (should not happen): default size at the origin.
        return Placement {
            rect: Rect { x: 0, y: 0, width: DEFAULT_SIZE.0 as u32, height: DEFAULT_SIZE.1 as u32 },
            maximized: false,
        };
    };

    if let Some(saved) = saved {
        let on_saved_monitor = monitors.iter().find(|m| m.name == saved.monitor).filter(|m| {
            let (w, h) = saved.rect.overlap(&m.work_area);
            w >= MIN_VISIBLE_PX && h >= MIN_VISIBLE_PX
        });
        if let Some(m) = on_saved_monitor {
            let (w, h) = clamp_size(f64::from(saved.rect.width), f64::from(saved.rect.height), &m.work_area, m.scale);
            let rect = shift_inside(Rect { width: w, height: h, ..saved.rect }, &m.work_area);
            return Placement { rect, maximized: saved.maximized };
        }
        let ratio = primary.scale / saved.scale.max(0.1);
        let (w, h) = clamp_size(
            f64::from(saved.rect.width) * ratio,
            f64::from(saved.rect.height) * ratio,
            &primary.work_area,
            primary.scale,
        );
        return Placement { rect: centred(w, h, &primary.work_area), maximized: saved.maximized };
    }

    let (w, h) =
        clamp_size(DEFAULT_SIZE.0 * primary.scale, DEFAULT_SIZE.1 * primary.scale, &primary.work_area, primary.scale);
    Placement { rect: centred(w, h, &primary.work_area), maximized: false }
}

/// Reads saved state; any read or parse problem means "no saved state".
pub fn load(path: &std::path::Path) -> Option<SavedState> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Writes saved state atomically (temp file + rename).
pub fn save(path: &std::path::Path, state: &SavedState) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(state).map_err(std::io::Error::other)?)?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(name: &str, x: i32, w: u32, h: u32, scale: f64, primary: bool) -> Monitor {
        Monitor { name: name.into(), work_area: Rect { x, y: 0, width: w, height: h }, scale, primary }
    }

    #[test]
    fn first_launch_centres_default_size_on_primary() {
        let p = initial_placement(None, &[monitor("A", 0, 2560, 1415, 1.0, true)]);
        assert_eq!(p.rect, Rect { x: 560, y: 257, width: 1440, height: 900 });
    }

    #[test]
    fn first_launch_scales_to_hidpi() {
        // 2× display with a 3024×1890 physical work area: 1440×900 logical = 2880×1800 physical.
        let p = initial_placement(None, &[monitor("A", 0, 3024, 1890, 2.0, true)]);
        assert_eq!((p.rect.width, p.rect.height), (2880, 1800));
        assert_eq!((p.rect.x, p.rect.y), (72, 45));
    }

    #[test]
    fn first_launch_on_small_display_uses_work_area() {
        let p = initial_placement(None, &[monitor("A", 0, 1280, 760, 1.0, true)]);
        assert_eq!(p.rect, Rect { x: 0, y: 0, width: 1280, height: 760 });
    }

    #[test]
    fn restores_on_connected_display() {
        let saved = SavedState {
            monitor: "B".into(),
            rect: Rect { x: 2700, y: 100, width: 1200, height: 800 },
            scale: 1.0,
            maximized: false,
        };
        let monitors = [monitor("A", 0, 2560, 1415, 1.0, true), monitor("B", 2560, 1920, 1050, 1.0, false)];
        let p = initial_placement(Some(&saved), &monitors);
        assert_eq!(p.rect, saved.rect);
    }

    #[test]
    fn restore_is_pulled_fully_inside_work_area() {
        let saved = SavedState {
            monitor: "A".into(),
            rect: Rect { x: 2000, y: 900, width: 1200, height: 800 },
            scale: 1.0,
            maximized: false,
        };
        let p = initial_placement(Some(&saved), &[monitor("A", 0, 2560, 1415, 1.0, true)]);
        assert_eq!(p.rect, Rect { x: 1360, y: 615, width: 1200, height: 800 });
    }

    #[test]
    fn disconnected_display_moves_to_primary_keeping_logical_size() {
        let saved = SavedState {
            monitor: "Gone".into(),
            rect: Rect { x: 5000, y: 200, width: 2400, height: 1600 },
            scale: 2.0,
            maximized: true,
        };
        let p = initial_placement(Some(&saved), &[monitor("A", 0, 1920, 1050, 1.0, true)]);
        // 1200×800 logical on a 1× display, centred, fully visible.
        assert_eq!(p.rect, Rect { x: 360, y: 125, width: 1200, height: 800 });
        assert!(p.maximized);
    }

    #[test]
    fn disconnected_display_clamps_to_primary_work_area() {
        let saved = SavedState {
            monitor: "Gone".into(),
            rect: Rect { x: -3000, y: 0, width: 3000, height: 2000 },
            scale: 1.0,
            maximized: false,
        };
        let p = initial_placement(Some(&saved), &[monitor("A", 0, 1920, 1050, 1.0, true)]);
        assert_eq!(p.rect, Rect { x: 0, y: 0, width: 1920, height: 1050 });
    }

    #[test]
    fn window_off_screen_on_known_display_is_recentred() {
        // Same display name but the saved rectangle no longer overlaps it (resolution changed).
        let saved = SavedState {
            monitor: "A".into(),
            rect: Rect { x: 4000, y: 3000, width: 1200, height: 800 },
            scale: 1.0,
            maximized: false,
        };
        let p = initial_placement(Some(&saved), &[monitor("A", 0, 1920, 1050, 1.0, true)]);
        assert_eq!(p.rect, Rect { x: 360, y: 125, width: 1200, height: 800 });
    }

    #[test]
    fn enforces_minimum_size() {
        let saved = SavedState {
            monitor: "A".into(),
            rect: Rect { x: 10, y: 10, width: 300, height: 200 },
            scale: 1.0,
            maximized: false,
        };
        let p = initial_placement(Some(&saved), &[monitor("A", 0, 1920, 1050, 1.0, true)]);
        assert_eq!((p.rect.width, p.rect.height), (1024, 640));
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = std::env::temp_dir().join(format!("ove-ws-{}", std::process::id()));
        let path = dir.join("window-state.json");
        let state = SavedState {
            monitor: "A".into(),
            rect: Rect { x: 1, y: 2, width: 3, height: 4 },
            scale: 1.5,
            maximized: true,
        };
        save(&path, &state).unwrap();
        assert_eq!(load(&path), Some(state));
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(load(&path), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
