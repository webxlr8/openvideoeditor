//! Property tests for batch apply (task 2.3): every random batch is either rejected with the document unchanged,
//! or applied with all invariants holding and `apply(inverse(batch))` restoring the original exactly.

use ove_core::diff::diff_documents;
use ove_core::invariants::check_document;
use ove_core::migrate::{Opened, open_json};
use ove_core::ops::apply_batch;
use ove_core::schema::Severity;
use ove_core::testgen::{arb_doc_and_batch, arb_document};
use proptest::prelude::*;

fn cases() -> u32 {
    std::env::var("OVE_PROPTEST_CASES").ok().and_then(|s| s.parse().ok()).unwrap_or(10_000)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), ..ProptestConfig::default() })]

    #[test]
    fn batch_is_atomic_and_invertible((doc, ops) in arb_doc_and_batch()) {
        let before = doc.clone();
        match apply_batch(&doc, &ops) {
            Err(e) => {
                prop_assert_eq!(&doc, &before);
                prop_assert!(!e.code.is_empty());
            }
            Ok(applied) => {
                let errs: Vec<_> = check_document(&applied.doc).into_iter().filter(|i| i.severity == Severity::Error).collect();
                prop_assert!(errs.is_empty(), "invariants violated: {:?}", errs);
                let restored = apply_batch(&applied.doc, &applied.inverse).expect("inverse applies");
                prop_assert_eq!(&restored.doc, &before);
            }
        }
    }

    #[test]
    fn diff_transforms_one_document_into_another((a, ops) in arb_doc_and_batch()) {
        if let Ok(b) = apply_batch(&a, &ops) {
            let diff = diff_documents(&a, &b.doc);
            let got = apply_batch(&a, &diff).expect("diff applies");
            prop_assert!(got.doc.content_eq(&b.doc));
            let back = apply_batch(&b.doc, &diff_documents(&b.doc, &a)).expect("reverse diff applies");
            prop_assert!(back.doc.content_eq(&a));
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases() / 10, ..ProptestConfig::default() })]

    #[test]
    fn json_round_trip(doc in arb_document()) {
        let text = doc.to_string_pretty();
        let back = match open_json(&text).expect("valid") { Opened::Current(d) => d, other => panic!("{other:?}") };
        prop_assert_eq!(&back, &doc);
        prop_assert_eq!(back.to_string_pretty(), text);
    }
}
