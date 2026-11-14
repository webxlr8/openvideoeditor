//! Golden corpus (tasks 1.2, 2.1, 3.1, 3.2): valid documents print byte-for-byte to their expected projections;
//! invalid projections report the expected diagnostic codes at the expected positions.
//! Regenerate expectations with `OVE_UPDATE_GOLDEN=1` and review the diff.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::{Path, PathBuf};

use ove_core::migrate::open_json;
use ove_core::model::Document;
use ove_projection::{ApplyInput, Prefer, plan, print};

fn dir(sub: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus").join(sub)
}

fn update() -> bool {
    std::env::var("OVE_UPDATE_GOLDEN").is_ok_and(|v| v == "1")
}

fn files(sub: &str, ext: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir(sub))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == ext))
        .collect();
    v.sort();
    v
}

fn load(p: &Path) -> Document {
    open_json(&std::fs::read_to_string(p).unwrap()).unwrap().doc().unwrap().clone()
}

fn golden(path: &Path, got: &str) {
    if update() {
        std::fs::write(path, got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(path)
        .unwrap_or_else(|_| panic!("missing {}; run with OVE_UPDATE_GOLDEN=1", path.display()));
    assert_eq!(got, want, "{} differs", path.display());
}

#[test]
fn valid_documents_print_to_golden_projections() {
    let docs = files("valid", "json");
    assert!(docs.len() >= 4);
    for p in docs {
        let doc = load(&p);
        let text = print(&doc, &Default::default());
        golden(&p.with_extension("tsx"), &text);
        // And the golden projection applies onto its own document as a no-op.
        let plan = plan(&ApplyInput {
            text: &text,
            current: &doc,
            base_for: &|_| Ok(doc.clone()),
            bundle: None,
            prefer: Prefer::File,
            now_ms: 1,
        })
        .unwrap();
        assert!(plan.ops.is_empty(), "{}: {:?}", p.display(), plan.ops);
    }
}

#[test]
fn invalid_projections_report_expected_diagnostics() {
    let base = load(&dir("valid").join("all-types.json"));
    let cases = files("invalid", "tsx");
    assert!(!cases.is_empty());
    let mut codes = std::collections::BTreeSet::new();
    for p in cases {
        let text = std::fs::read_to_string(&p).unwrap();
        let out = plan(&ApplyInput {
            text: &text,
            current: &base,
            base_for: &|_| Ok(base.clone()),
            bundle: None,
            prefer: Prefer::File,
            now_ms: 1,
        });
        let lines: Vec<String> = match out {
            Err(r) if r.diagnostics.is_empty() => vec![format!("refused {}", r.code)],
            Err(r) => r
                .diagnostics
                .iter()
                .map(|d| {
                    format!("{} {}:{} {}", if d.is_error() { "error" } else { "warning" }, d.line, d.column, d.code)
                })
                .collect(),
            Ok(plan) => plan
                .warnings
                .iter()
                .map(|d| format!("warning {}:{} {}", d.line, d.column, d.code))
                .chain(std::iter::once("applies".to_string()))
                .collect(),
        };
        for l in &lines {
            codes.insert(l.split_whitespace().last().unwrap().to_string());
        }
        golden(&p.with_extension("diag"), &(lines.join("\n") + "\n"));
    }
    // Every diagnostic code the checker can produce is covered by at least one case.
    for code in [
        "syntax",
        "structure.root",
        "structure.element",
        "structure.text",
        "structure.attribute",
        "literal.only",
        "attr.duplicate",
        "attr.required",
        "attr.reserved",
        "attr.unknown",
        "enum.known",
        "value.type",
        "value.range",
        "value.nonNegative",
        "src.unresolved",
        "item.id.unique",
        "track.id.unique",
        "item.time.order",
        "object.key.unknown",
        "rev.newer",
    ] {
        assert!(codes.contains(code), "no corpus case covers {code}");
    }
}
