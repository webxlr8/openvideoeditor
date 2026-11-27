//! Task 3.3: beat times mapped to the timeline agree with the renderer's own source-time mapping under any trim,
//! speed and direction (`OVE_PROPTEST_CASES` sets the number of cases, 10,000 by default).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use ove_analyze::beats::to_timeline;
use ove_core::model::Item;
use proptest::prelude::*;
use serde_json::json;

fn cases() -> u32 {
    std::env::var("OVE_PROPTEST_CASES").ok().and_then(|s| s.parse().ok()).unwrap_or(10_000)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases()))]

    #[test]
    fn timeline_beats_land_on_their_source_beats(
        start in 0.0f64..100.0,
        len in 0.5f64..30.0,
        trim in 0.0f64..20.0,
        speed in 0.25f64..4.0,
        reverse in any::<bool>(),
        period in 0.2f64..1.0,
    ) {
        let (start, len, trim, speed) = ((start * 1e3).round() / 1e3, (len * 1e3).round() / 1e3, (trim * 1e3).round() / 1e3, (speed * 1e3).round() / 1e3);
        let item = Item::from_json(&json!({"id": "a", "trackId": "t", "type": "audio", "startTime": start, "endTime": start + len,
            "trimStart": trim, "playbackSpeed": speed, "playbackDirection": if reverse { "reverse" } else { "forward" },
            "mediaItemId": "m"}), "a").unwrap();
        let source: Vec<f64> = (0..400).map(|k| k as f64 * period).collect();
        let tl = to_timeline(&item, &source);
        // Ascending, inside the item, spaced by the source spacing over the speed.
        prop_assert!(tl.windows(2).all(|w| w[1] > w[0]));
        prop_assert!(tl.iter().all(|t| *t >= start - 1e-6 && *t < start + len));
        for w in tl.windows(2) {
            prop_assert!((w[1] - w[0] - period / speed).abs() < 1e-5, "{} vs {}", w[1] - w[0], period / speed);
        }
        // Each timeline beat shows its source beat (the renderer's mapping, to a few microseconds).
        for t in &tl {
            let s = ove_render::eval::source_time(&item, (t * 1e6).round() as i64) as f64 / 1e6;
            let k = (s / period).round();
            prop_assert!((s - k * period).abs() < 5e-6 * speed.max(1.0) + 2e-6, "t {t} shows source {s}");
        }
        // Every source beat inside the visible part is on the timeline.
        let used = len * speed;
        let visible = source.iter().filter(|s| **s > trim + 1e-4 && **s < trim + used - 1e-4).count();
        prop_assert!(tl.len() >= visible, "{} of {visible}", tl.len());
    }
}

#[test]
fn retimed_item_beats_are_half_as_far_apart_from_second_10() {
    // The spec scenario: 2× speed starting at timeline second 10.
    let item = Item::from_json(
        &json!({"id": "a", "trackId": "t", "type": "audio", "startTime": 10, "endTime": 14, "playbackSpeed": 2, "mediaItemId": "m"}),
        "a",
    )
    .unwrap();
    let tl = to_timeline(&item, &[0.0, 0.5, 1.0, 1.5]);
    assert_eq!(tl, vec![10.0, 10.25, 10.5, 10.75]);
}
