//! Golden frames for the 41 catalogue effects: each effect at its defaults over the sample frame at 1 s, compared with
//! the stored frame with a tolerance of one 8-bit step per channel, so the same file serves macOS, Windows and Linux.
//!
//! The golden file `tests/golden/effects-48x27.pam` holds the frames stacked top to bottom in catalogue order (one
//! 48×27 band per effect). Regenerate it with `OVE_UPDATE_GOLDEN=1` after checking the new frames by eye;
//! `OVE_GOLDEN_DUMP=<dir>` also writes the current strip there.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

mod golden_util;

use golden_util::{TOLERANCE, golden_or_bless, max_diff, write_pam};
use ove_render::effects::{self, EFFECTS};
use serde_json::Map;

const W: u32 = 48;
const H: u32 = 27;
const GOLDEN: &str = "effects-48x27.pam";

/// Every effect's frame, stacked top to bottom in catalogue order.
fn strip() -> Vec<u8> {
    let base = effects::sample_image(W, H);
    let mut out = Vec::with_capacity((W * H * 4) as usize * EFFECTS.len());
    for spec in EFFECTS {
        let mut img = base.clone();
        assert!(effects::apply(&mut img, spec.id, &Map::new(), 1.0, 1.0), "{} did not run", spec.id);
        out.extend_from_slice(&img.data);
    }
    out
}

#[test]
fn effects_match_their_golden_frames() {
    let got = strip();
    let h = H * EFFECTS.len() as u32;
    if let Some(dir) = std::env::var_os("OVE_GOLDEN_DUMP") {
        write_pam(&std::path::PathBuf::from(dir).join(GOLDEN), W, h, &got).unwrap();
    }
    let (gw, gh, want) = golden_or_bless(GOLDEN, W, h, &got);
    assert_eq!((gw, gh), (W, h), "the effect catalogue changed size: bless {GOLDEN} again");
    let band = (W * H * 4) as usize;
    let failed: Vec<String> = EFFECTS
        .iter()
        .enumerate()
        .filter_map(|(i, spec)| {
            let d = max_diff(&got[i * band..(i + 1) * band], &want[i * band..(i + 1) * band]);
            (d > TOLERANCE).then(|| format!("{} (max channel difference {d})", spec.id))
        })
        .collect();
    assert!(failed.is_empty(), "effect output changed: {}", failed.join(", "));
}

/// The comparison is not too loose: changing one effect's parameter is caught.
#[test]
fn a_changed_effect_is_caught() {
    let base = effects::sample_image(W, H);
    let mut a = base.clone();
    effects::apply(&mut a, "posterize", &Map::new(), 1.0, 1.0);
    let mut b = base.clone();
    let p = serde_json::json!({"levels": 3});
    effects::apply(&mut b, "posterize", p.as_object().unwrap(), 1.0, 1.0);
    assert!(max_diff(&a.data, &b.data) > TOLERANCE);
}
