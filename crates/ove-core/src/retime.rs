//! Keyframe easing shared by the renderer and the edit planners.
//!
//! The split and trim retiming that lived here moved to the `ove-edit` crate (`ove_edit::retime`), where every edit
//! planner shares it.

/// Keyframe easing curves; the renderer uses this same function.
pub fn ease(easing: &str, p: f64) -> f64 {
    let p = p.clamp(0.0, 1.0);
    match easing {
        "ease-in" => p * p,
        "ease-out" => 1.0 - (1.0 - p) * (1.0 - p),
        "ease-in-out" => {
            if p < 0.5 {
                2.0 * p * p
            } else {
                1.0 - (-2.0 * p + 2.0).powi(2) / 2.0
            }
        }
        _ => p,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_ends_are_fixed() {
        for e in ["linear", "ease-in", "ease-out", "ease-in-out"] {
            assert_eq!(ease(e, 0.0), 0.0);
            assert_eq!(ease(e, 1.0), 1.0);
        }
        assert_eq!(ease("ease-in", 0.5), 0.25);
    }
}
