//! Regenerates `apps/desktop/ui/src/bindings.ts` from the Rust command/event definitions.
//! `--check` exits 1 if the committed file is stale (used by CI).

fn main() {
    let path = ove_desktop_lib::bindings_path();
    let check = std::env::args().any(|a| a == "--check");
    let fresh = match ove_desktop_lib::render_bindings() {
        Ok(text) => text,
        Err(e) => {
            eprintln!("could not generate bindings: {e}");
            std::process::exit(2);
        }
    };
    if check {
        let committed = std::fs::read_to_string(&path).unwrap_or_default();
        if committed != fresh {
            eprintln!("{} is stale: run `cargo run -p ove-desktop --bin export-bindings`", path.display());
            std::process::exit(1);
        }
        println!("bindings up to date");
        return;
    }
    if let Err(e) = std::fs::write(&path, fresh) {
        eprintln!("could not write {}: {e}", path.display());
        std::process::exit(2);
    }
    println!("wrote {}", path.display());
}
