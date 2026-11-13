fn main() -> std::io::Result<()> {
    // `tauri::generate_context!` embeds the built UI. When the UI has not been built yet (plain
    // `cargo test --workspace`), embed a placeholder page so the crate still compiles.
    let dist = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui/dist");
    if !dist.join("index.html").exists() {
        std::fs::create_dir_all(&dist)?;
        std::fs::write(
            dist.join("index.html"),
            "<!doctype html><meta charset=utf-8><title>UI not built</title>\
             <body style=\"background:#141414;color:#ccc;font:14px sans-serif\">\
             The UI is not built. Run <code>pnpm build</code> in apps/desktop.</body>",
        )?;
    }
    // MSVC: one Windows manifest (common controls 6, the same one Tauri embeds by default) linked into every executable
    // of this package. Tauri's default reaches only the app, and the test executables then fail to start
    // (STATUS_ENTRYPOINT_NOT_FOUND: Tauri's dialogs import a function the old common controls do not have).
    // Other targets, including the MinGW build that ships, keep Tauri's default.
    let msvc = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if msvc {
        let attributes = tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        tauri_build::try_build(attributes).map_err(|e| std::io::Error::other(format!("{e:#}")))?;
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    } else {
        tauri_build::build();
    }
    Ok(())
}
