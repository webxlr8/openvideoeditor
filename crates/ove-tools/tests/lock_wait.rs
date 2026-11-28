//! Waiting for a project's write lock (`open_write_waiting`). A holder takes the OS lock first and writes
//! `.lock.holder` just after; it removes that file just before it lets go. A writer that polls between those steps
//! finds the lock busy and no holder file. It must keep waiting then, not report `project.locked` at once.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ove_tools::bundle::{WAIT_FOR_SESSION, open_write_waiting};

/// A library with one empty project; returns its bundle.
fn fixture() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let library = ove_store::Library::open(&d.path().join("lib"), Arc::new(ove_core::SystemClock)).unwrap();
    let p = library.create("Demo").unwrap();
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    (d, bundle)
}

/// Takes the project's OS lock the way a holder does, but writes no holder file: a holder caught between its
/// lock and its holder file, or one that stopped there.
fn take_bare_lock(bundle: &Path) -> File {
    let f = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(bundle.join(".lock")).unwrap();
    f.try_lock().unwrap();
    f
}

#[test]
fn a_busy_lock_with_no_holder_file_is_waited_for() {
    let (_d, bundle) = fixture();
    let lock = take_bare_lock(&bundle);
    // The holder goes away without ever writing its file (as when it dies mid-acquire: the OS frees the lock).
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        drop(lock);
    });
    let t0 = Instant::now();
    let p =
        open_write_waiting(&bundle).unwrap_or_else(|f| panic!("{}: {} after {:?}", f.code, f.message, t0.elapsed()));
    assert!(t0.elapsed() >= Duration::from_millis(250), "it waited for the holder: {:?}", t0.elapsed());
    t.join().unwrap();
    drop(p);
}

#[test]
fn a_holder_that_never_writes_its_file_ends_the_wait_with_a_clear_error() {
    let (_d, bundle) = fixture();
    let _lock = take_bare_lock(&bundle);
    let t0 = Instant::now();
    let f = open_write_waiting(&bundle).err().expect("the lock stays busy");
    let waited = t0.elapsed();
    assert_eq!(f.code, "project.locked", "{}", f.message);
    assert!(waited >= WAIT_FOR_SESSION, "waited the full time: {waited:?}");
    assert!(waited < WAIT_FOR_SESSION + Duration::from_secs(3), "and not much longer: {waited:?}");
    assert!(f.message.contains("another process"), "{}", f.message);
    assert!(f.message.contains(&format!("waited {} s", WAIT_FOR_SESSION.as_secs())), "{}", f.message);
}

#[test]
fn a_holder_that_names_the_app_is_reported_at_once() {
    let (_d, bundle) = fixture();
    let _lock = take_bare_lock(&bundle);
    let app = ove_store::lock::Holder { pid: 1, process: "ove-desktop".into(), started_at: 0, host: "here".into() };
    std::fs::write(bundle.join(".lock.holder"), serde_json::to_vec(&app).unwrap()).unwrap();
    let t0 = Instant::now();
    let f = open_write_waiting(&bundle).err().expect("the app holds it");
    assert_eq!(f.code, "project.locked");
    assert!(f.message.contains("ove-desktop"), "{}", f.message);
    assert!(t0.elapsed() < Duration::from_secs(2), "no wait for the app: {:?}", t0.elapsed());
}
