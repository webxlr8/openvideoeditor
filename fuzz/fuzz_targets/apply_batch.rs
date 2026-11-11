//! Any op list applied to a fixture document is either rejected (document unchanged) or applied with invariants
//! holding and the inverse restoring the original — never a panic.
#![no_main]
use libfuzzer_sys::fuzz_target;
use ove_core::ops::{Op, apply_batch};

const FIXTURE: &str = include_str!("../../crates/ove-core/tests/fixtures/all-types.json");

fuzz_target!(|data: &[u8]| {
    let Ok(ops) = serde_json::from_slice::<Vec<Op>>(data) else { return };
    let doc = ove_core::migrate::open_json(FIXTURE).unwrap().doc().unwrap().clone();
    if let Ok(applied) = apply_batch(&doc, &ops) {
        let errors = ove_core::invariants::errors(ove_core::invariants::check_document(&applied.doc));
        assert!(errors.is_empty(), "{errors:?}");
        let back = apply_batch(&applied.doc, &applied.inverse).expect("inverse applies");
        assert_eq!(back.doc, doc);
    }
});
