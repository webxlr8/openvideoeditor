//! Writes (or with `--check`, verifies) the committed copy of the generated typings: `docs/spec/timeline.d.ts`.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let check = std::env::args().any(|a| a == "--check");
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/spec/timeline.d.ts");
    let want = ove_projection::typings::typings();
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    if have == want {
        return ExitCode::SUCCESS;
    }
    if check {
        eprintln!("{} is out of date; run `cargo run -p ove-projection --bin ove-typings`", path.display());
        return ExitCode::FAILURE;
    }
    std::fs::write(&path, want).expect("write");
    println!("wrote {}", path.display());
    ExitCode::SUCCESS
}
