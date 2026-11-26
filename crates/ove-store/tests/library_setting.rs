//! The library chosen in Settings (add-settings-and-templates 1.3), kept for the app and the command line. Its own
//! test binary: it sets process-wide environment variables.
#![allow(clippy::unwrap_used, reason = "test code")]

use std::path::Path;

use ove_store::library::{
    LIBRARY_ENV, SETTINGS_DIR_ENV, library_setting, library_setting_file, resolve_library, set_library_setting,
};

#[test]
fn the_library_chosen_in_settings_is_kept_and_used() {
    let d = tempfile::tempdir().unwrap();
    // SAFETY: this test binary has one test, so nothing else reads the environment meanwhile.
    unsafe {
        std::env::set_var(SETTINGS_DIR_ENV, d.path().join("settings"));
        std::env::remove_var(LIBRARY_ENV);
    }
    assert_eq!(library_setting_file().unwrap(), d.path().join("settings").join("library.json"));
    assert_eq!(library_setting(), None);

    let chosen = d.path().join("My Library");
    set_library_setting(Some(&chosen)).unwrap();
    assert!(chosen.is_dir(), "created when chosen");
    assert_eq!(library_setting(), Some(chosen.clone()));
    assert_eq!(resolve_library(None).unwrap(), chosen, "the app and the command line use it");

    // The environment variable still wins.
    let env = d.path().join("From env");
    // SAFETY: as above.
    unsafe { std::env::set_var(LIBRARY_ENV, &env) };
    assert_eq!(resolve_library(None).unwrap(), env);
    // SAFETY: as above.
    unsafe { std::env::remove_var(LIBRARY_ENV) };

    assert_eq!(set_library_setting(Some(Path::new("relative/dir"))).unwrap_err().code(), "library.relative");
    assert_eq!(library_setting(), Some(chosen), "a refused choice changes nothing");
    set_library_setting(None).unwrap();
    assert_eq!(library_setting(), None, "forgotten: back to the default");
    set_library_setting(None).unwrap();
}
