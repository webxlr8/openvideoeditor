//! Writes (or with `--check`, verifies) the generated schema files:
//! `docs/spec/project.schema.json` and `docs/spec/project-schema-v1.md`.
#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let check = std::env::args().any(|a| a == "--check");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    // `{:#}` is serde_json's 2-space pretty printer, the same text as `to_string_pretty`.
    let json = format!("{:#}\n", ove_core::jsonschema::project_schema());
    let files = [
        (root.join("docs/spec/project.schema.json"), json),
        (root.join("docs/spec/project-schema-v1.md"), ove_core::jsonschema::schema_markdown()),
    ];
    let mut stale = false;
    for (path, content) in files {
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if current == content {
            continue;
        }
        if check {
            eprintln!("{} is out of date; run `cargo run -p ove-core --bin ove-schema`", path.display());
            stale = true;
        } else if let Err(e) = std::fs::write(&path, content) {
            eprintln!("cannot write {}: {e}", path.display());
            return ExitCode::FAILURE;
        } else {
            println!("wrote {}", path.display());
        }
    }
    if stale { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
