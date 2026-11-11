fn main() {
    // `tauri::generate_context!` embeds the built UI. When the UI has not been built yet (plain
    // `cargo test --workspace`), embed a placeholder page so the crate still compiles.
    let dist = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui/dist");
    if !dist.join("index.html").exists() {
        std::fs::create_dir_all(&dist).expect("create ui/dist");
        std::fs::write(
            dist.join("index.html"),
            "<!doctype html><meta charset=utf-8><title>UI not built</title>\
             <body style=\"background:#141414;color:#ccc;font:14px sans-serif\">\
             The UI is not built. Run <code>pnpm build</code> in apps/desktop.</body>",
        )
        .expect("write placeholder ui/dist/index.html");
    }
    tauri_build::build();
}
