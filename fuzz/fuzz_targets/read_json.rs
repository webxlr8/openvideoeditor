//! Any bytes given to the project reader yield a document or a structured error — never a panic.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        if let Ok(opened) = ove_core::migrate::open_json(text) {
            if let Some(doc) = opened.doc() {
                // A document that opened must serialise and reopen to the same thing.
                let again = ove_core::migrate::open_json(&doc.to_string_pretty()).expect("reopen");
                assert_eq!(again.doc(), Some(doc));
            }
        }
    }
});
