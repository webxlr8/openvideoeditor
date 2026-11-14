//! Any text given to the projection checker yields a plan or a refusal with diagnostics — never a panic.
#![no_main]
use libfuzzer_sys::fuzz_target;

const FIXTURE: &str = include_str!("../../crates/ove-core/tests/fixtures/all-types.json");

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    let doc = ove_core::migrate::open_json(FIXTURE).unwrap().doc().unwrap().clone();
    let parsed = ove_projection::parse::parse(text);
    if parsed.root.is_none() {
        assert!(!parsed.diagnostics.is_empty(), "no tree and no diagnostics");
    }
    let _ = ove_projection::plan(&ove_projection::ApplyInput {
        text,
        current: &doc,
        base_for: &|_| Ok(doc.clone()),
        bundle: None,
        prefer: Default::default(),
        now_ms: 1,
    });
});
