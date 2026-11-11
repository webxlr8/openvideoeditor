//! Writes (or with `--check`, verifies) the generated schema files:
//! `docs/spec/project.schema.json` and `docs/spec/project-schema-v1.md`.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let check = std::env::args().any(|a| a == "--check");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut json = serde_json::to_string_pretty(&ove_core::jsonschema::project_schema()).expect("json");
    json.push('\n');
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
        } else {
            std::fs::write(&path, content).expect("write");
            println!("wrote {}", path.display());
        }
    }
    if stale { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
