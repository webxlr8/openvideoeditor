// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// mimalloc instead of the system allocator: on Windows the system heap made projection print and apply about
// 4× slower than on Linux (5,000 items: print 122 ms → 24 ms, check + apply 204 ms → 50 ms).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> std::process::ExitCode {
    ove_desktop_lib::run()
}
