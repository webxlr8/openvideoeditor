//! Transitions: the catalogue (docs/spec/presets.md, deduplicated), and how a transitioning layer is masked, moved
//! or post-processed.
//!
//! A transition is `transitionIn`/`transitionOut {type, duration, easing, direction}` on an item. Between two
//! adjacent items on one track it is a *join*: the outgoing item keeps playing under the incoming one for the
//! duration (except `fade`, which dips through what is below, and `audio-fade`, which only touches sound).
//! Elsewhere the item transitions against whatever is below it.

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::canvas::Rgba;
use crate::effects::{self};

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// Takes a `direction` (left, right, up, down).
    pub directional: bool,
    /// Only affects sound.
    pub audio_only: bool,
}

const fn t(id: &'static str, label: &'static str, directional: bool) -> TransitionSpec {
    TransitionSpec { id, label, directional, audio_only: false }
}

pub const TRANSITIONS: &[TransitionSpec] = &[
    t("fade", "Fade", false),
    t("dissolve", "Dissolve", false),
    t("wipe", "Wipe", true),
    t("wipe-left", "Wipe left", false),
    t("wipe-right", "Wipe right", false),
    t("wipe-up", "Wipe up", false),
    t("wipe-down", "Wipe down", false),
    t("push", "Push", true),
    t("push-left", "Push left", false),
    t("push-right", "Push right", false),
    t("push-up", "Push up", false),
    t("push-down", "Push down", false),
    t("fly", "Fly", true),
    t("zoom", "Zoom", false),
    t("scale", "Scale", false),
    t("cross-zoom", "Cross zoom", false),
    t("radial-wipe", "Radial wipe", false),
    t("ink-bleed", "Ink bleed", false),
    t("mask-glitch", "Mask glitch", false),
    t("glitch", "Glitch", false),
    t("chromatic-aberration", "Chromatic aberration", false),
    t("film-burn", "Film burn", false),
    t("light-leak", "Light leak", false),
    TransitionSpec { id: "audio-fade", label: "Audio fade", directional: false, audio_only: true },
];

pub const EASINGS: &[&str] = &["linear", "ease-in", "ease-out", "ease-in-out"];
pub const DIRECTIONS: &[&str] = &["left", "right", "up", "down"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

impl Dir {
    pub fn parse(s: Option<&str>) -> Dir {
        match s {
            Some("right") => Dir::Right,
            Some("up") => Dir::Up,
            Some("down") => Dir::Down,
            _ => Dir::Left,
        }
    }
}

/// Splits `wipe-left` style ids into a base kind and direction; plain ids take `direction` (default left).
pub fn split<'a>(kind: &'a str, direction: Option<&str>) -> (&'a str, Dir) {
    for (suffix, d) in [("-left", Dir::Left), ("-right", Dir::Right), ("-up", Dir::Up), ("-down", Dir::Down)] {
        if let Some(base) = kind.strip_suffix(suffix)
            && matches!(base, "wipe" | "push" | "fly")
        {
            return (base, d);
        }
    }
    (kind, Dir::parse(direction))
}

/// Pure opacity transitions: evaluated as opacity, no offscreen pass.
pub fn is_opacity_only(kind: &str) -> bool {
    matches!(kind, "fade" | "dissolve" | "crossfade" | "cross-dissolve" | "cross-fade")
}

/// Kinds that, between adjacent items, keep the outgoing item under the incoming one.
pub fn joins(kind: &str) -> bool {
    !matches!(kind, "fade" | "audio-fade")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The item appearing (or, for an exit without a join, the item disappearing with `q` falling).
    In,
    /// The outgoing item of a join.
    OutJoin,
}

/// A layer's transition at one instant.
#[derive(Debug, Clone, PartialEq)]
pub struct TransState {
    pub kind: String,
    pub dir: Dir,
    pub role: Role,
    /// Visibility of the incoming side, eased, 0..1.
    pub q: f32,
}

/// Placement change of a transitioning layer: offset in frame fractions, scale about the centre, opacity factor,
/// and zoom-blur strength (0..100).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    pub dx: f32,
    pub dy: f32,
    pub scale: f32,
    pub opacity: f32,
    pub blur: f32,
}

const STILL: Motion = Motion { dx: 0.0, dy: 0.0, scale: 1.0, opacity: 1.0, blur: 0.0 };

fn dir_vec(d: Dir) -> (f32, f32) {
    // Direction of travel: "left" moves content towards the left, so the incoming side enters from the right.
    match d {
        Dir::Left => (-1.0, 0.0),
        Dir::Right => (1.0, 0.0),
        Dir::Up => (0.0, -1.0),
        Dir::Down => (0.0, 1.0),
    }
}

impl TransState {
    pub fn motion(&self) -> Motion {
        let (base, _) = split(&self.kind, None);
        let (vx, vy) = dir_vec(self.dir);
        let q = self.q.clamp(0.0, 1.0);
        match self.role {
            Role::In => match base {
                "push" => Motion { dx: -vx * (1.0 - q), dy: -vy * (1.0 - q), ..STILL },
                "fly" => Motion {
                    dx: -vx * (1.0 - q) * 0.35,
                    dy: -vy * (1.0 - q) * 0.35,
                    scale: 0.85 + 0.15 * q,
                    opacity: q,
                    blur: 0.0,
                },
                "zoom" => Motion { scale: 0.6 + 0.4 * q, opacity: q, ..STILL },
                "scale" => Motion { scale: q.max(1e-3), ..STILL },
                "cross-zoom" => Motion { scale: 1.0 + (1.0 - q) * 0.6, opacity: q, blur: (1.0 - q) * 70.0, ..STILL },
                "glitch" | "chromatic-aberration" | "film-burn" | "light-leak" => Motion { opacity: q, ..STILL },
                k if is_opacity_only(k) => Motion { opacity: q, ..STILL },
                _ => STILL,
            },
            // The outgoing side moves on as the incoming side arrives.
            Role::OutJoin => match base {
                "push" => Motion { dx: vx * q, dy: vy * q, ..STILL },
                "cross-zoom" => Motion { scale: 1.0 + q * 0.6, blur: q * 70.0, ..STILL },
                _ => STILL,
            },
        }
    }

    /// Whether the incoming layer is revealed through a mask.
    pub fn has_mask(&self) -> bool {
        self.role == Role::In
            && matches!(split(&self.kind, None).0, "wipe" | "radial-wipe" | "ink-bleed" | "mask-glitch")
    }

    fn mask(&self, u: f32, v: f32, aspect: f32) -> f32 {
        let q = self.q.clamp(0.0, 1.0);
        // A threshold sweep with a soft edge `e` that is fully closed at q=0 and fully open at q=1.
        let sweep = |x: f32, e: f32| {
            let th = (1.0 - q) * (1.0 + 2.0 * e) - e;
            smooth(th - e, th + e, x)
        };
        match split(&self.kind, None).0 {
            "wipe" => match self.dir {
                Dir::Left => sweep(u, 0.02),
                Dir::Right => sweep(1.0 - u, 0.02),
                Dir::Up => sweep(v, 0.02),
                Dir::Down => sweep(1.0 - v, 0.02),
            },
            "radial-wipe" => {
                let (x, y) = ((u - 0.5) * aspect, v - 0.5);
                let a = (x.atan2(-y) / std::f32::consts::TAU).rem_euclid(1.0); // clockwise from 12 o'clock
                sweep(1.0 - a, 0.01)
            }
            "ink-bleed" => {
                let n = crate::effects::noise_fbm(u * 5.0 * aspect, v * 5.0, 4, 7);
                let (x, y) = ((u - 0.5) * aspect, v - 0.5);
                let centre = 1.0 - ((x * x + y * y).sqrt() / (0.5 * (aspect * aspect + 1.0).sqrt())).min(1.0);
                sweep(n * 0.55 + centre * 0.45, 0.04)
            }
            "mask-glitch" => {
                let (bx, by) = ((u * 16.0).floor() as i32, (v * 9.0 * (1.0 + (u * 3.0).floor() * 0.5)).floor() as i32);
                let h = crate::effects::hash(bx, by, 13);
                if h < q { 1.0 } else { 0.0 }
            }
            _ => 1.0,
        }
    }

    /// Masks the layer's alpha in place.
    pub fn apply_mask(&self, img: &mut Rgba) {
        if !self.has_mask() {
            return;
        }
        let (w, h) = (img.w as usize, img.h as f32);
        let aspect = img.w as f32 / img.h as f32;
        use rayon::prelude::*;
        img.data.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
            let v = (y as f32 + 0.5) / h;
            for x in 0..w {
                let u = (x as f32 + 0.5) / w as f32;
                let m = self.mask(u, v, aspect);
                let a = &mut row[x * 4 + 3];
                *a = (*a as f32 * m + 0.5) as u8;
            }
        });
    }

    /// The effect run over the composited frame while this (incoming) layer transitions: (effect id, params,
    /// amount). Strongest at the middle of the transition.
    pub fn post_effect(&self) -> Option<(&'static str, Map<String, Value>, f32)> {
        if self.role != Role::In {
            return None;
        }
        let s = (std::f32::consts::PI * self.q.clamp(0.0, 1.0)).sin();
        let m = |v: Value| v.as_object().cloned().unwrap_or_default();
        match split(&self.kind, None).0 {
            "film-burn" => Some((
                "light-leak",
                m(json!({"intensity": 1.0, "spread": 0.45 + s, "streaks": 0.9, "position": [0.85, 0.35],
                    "hotColor": [1.0, 0.98, 0.85], "midColor": [1.0, 0.45, 0.1], "fringeColor": [0.7, 0.1, 0.05]})),
                s,
            )),
            "light-leak" => Some(("light-leak", m(json!({"intensity": 0.6, "spread": 0.6 + 0.4 * s})), s)),
            "glitch" => Some(("glitch", m(json!({"intensity": s, "speed": 3.0})), s)),
            "mask-glitch" => Some(("glitch", m(json!({"intensity": s * 0.6, "speed": 4.0, "mirrorAmount": 0.0})), s)),
            "chromatic-aberration" => Some(("chromatic-aberration", m(json!({"strength": s})), 1.0)),
            _ => None,
        }
    }
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Runs a transition's post effect on the canvas (time `t` seconds feeds animated effects).
pub fn apply_post(canvas: &mut Rgba, st: &TransState, t: f32) {
    if let Some((id, params, amount)) = st.post_effect() {
        effects::apply(canvas, id, &params, amount, t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(kind: &str, q: f32) -> TransState {
        let (_, dir) = split(kind, None);
        TransState { kind: kind.into(), dir, role: Role::In, q }
    }

    #[test]
    fn catalogue_is_deduplicated_and_complete() {
        let mut ids: Vec<_> = TRANSITIONS.iter().map(|t| t.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), TRANSITIONS.len());
        for id in [
            "audio-fade",
            "chromatic-aberration",
            "cross-zoom",
            "dissolve",
            "fade",
            "film-burn",
            "fly",
            "glitch",
            "ink-bleed",
            "light-leak",
            "mask-glitch",
            "push",
            "radial-wipe",
            "wipe",
            "zoom",
            "push-left",
            "push-right",
            "push-up",
            "push-down",
            "wipe-left",
            "wipe-right",
            "wipe-up",
            "wipe-down",
            "scale",
        ] {
            assert!(ids.contains(&id), "{id}");
        }
    }

    #[test]
    fn masks_open_from_closed_to_full() {
        for k in ["wipe-left", "wipe-right", "wipe-up", "wipe-down", "radial-wipe", "ink-bleed", "mask-glitch"] {
            for (q, want) in [(0.0, 0u8), (1.0, 255u8)] {
                let mut img = Rgba::filled(32, 18, [200, 200, 200, 255]);
                st(k, q).apply_mask(&mut img);
                assert!(img.data.chunks(4).all(|p| p[3] == want), "{k} at {q}");
            }
            let mut img = Rgba::filled(32, 18, [200, 200, 200, 255]);
            st(k, 0.5).apply_mask(&mut img);
            let open = img.data.chunks(4).filter(|p| p[3] > 128).count() as f32 / (32.0 * 18.0);
            assert!((0.15..0.85).contains(&open), "{k} half way: {open}");
        }
        // wipe-left: the edge travels left, so the right side opens first.
        let mut img = Rgba::filled(32, 18, [0, 0, 0, 255]);
        st("wipe-left", 0.3).apply_mask(&mut img);
        assert!(img.px(31, 9)[3] == 255 && img.px(0, 9)[3] == 0);
    }

    #[test]
    fn push_moves_both_sides_the_same_way() {
        let a = TransState { kind: "push-left".into(), dir: Dir::Left, role: Role::In, q: 0.25 }.motion();
        let b = TransState { kind: "push-left".into(), dir: Dir::Left, role: Role::OutJoin, q: 0.25 }.motion();
        assert_eq!((a.dx, b.dx), (0.75, -0.25));
        assert_eq!(split("push", Some("down")), ("push", Dir::Down));
        assert_eq!(split("wipe-up", Some("down")), ("wipe", Dir::Up));
    }
}
