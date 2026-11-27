//! Any bytes given to the revision record decoder yield a record or an error — never a panic — and a record that
//! decodes (with or without an actor) writes and reads back to the same record.
#![no_main]
use libfuzzer_sys::fuzz_target;
use ove_core::revision::RevisionRecord;

fuzz_target!(|data: &[u8]| {
    if let Ok(r) = serde_json::from_slice::<RevisionRecord>(data) {
        let text = serde_json::to_vec(&r).expect("write");
        let again: RevisionRecord = serde_json::from_slice(&text).expect("reread");
        assert_eq!(again.actor, r.actor);
        assert_eq!(again.rev, r.rev);
        assert_eq!(again.summary, r.summary);
    }
});
