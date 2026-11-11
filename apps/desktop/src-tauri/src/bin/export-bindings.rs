//! Regenerates `apps/desktop/ui/src/bindings.ts` from the Rust command/event definitions, and the effects catalogue
//! (`fx-catalog.json`) the browser mock serves.
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
    // The effects/transitions/layouts catalogue the browser mock serves.
    let files = [(path, fresh), ove_desktop_lib::fx::catalog_file()];
    if check {
        for (path, fresh) in &files {
            let committed = std::fs::read_to_string(path).unwrap_or_default();
            if committed != *fresh {
                eprintln!("{} is stale: run `cargo run -p ove-desktop --bin export-bindings`", path.display());
                std::process::exit(1);
            }
        }
        println!("bindings up to date");
        return;
    }
    for (path, fresh) in files {
        if let Err(e) = std::fs::write(&path, fresh) {
            eprintln!("could not write {}: {e}", path.display());
            std::process::exit(2);
        }
        println!("wrote {}", path.display());
    }
}
