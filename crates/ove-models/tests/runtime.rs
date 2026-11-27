//! The inference runtime library in the store: a clean profile reports "runtime not installed" and downloads nothing
//! until the person agrees; then the wheel is verified, the library, licence and notices are extracted and checked,
//! and the wheel is deleted. Also the offline install from a folder, and the real wheels when they are at hand.

#![allow(
    clippy::unwrap_used,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions"
)]

mod common;

use std::io::Write;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use common::{COMMIT, Mode, Server, body, sha};
use ove_models::manifest::{RUNTIME_ID, RUNTIME_VERSION, RuntimePlatform, runtime_manifest};
use ove_models::{ArchiveMember, Ensure, Manifest, ModelError, ModelFile, Origin, RuntimeKind, Store, UrlOverride};

const WHEEL: &str = "rt-1.0-py3-none-any.whl";

struct Wheel {
    bytes: Vec<u8>,
    library: Vec<u8>,
    licence: Vec<u8>,
    notices: Vec<u8>,
}

/// A small wheel with a library, a licence and notices (deflated, as real wheels are).
fn wheel() -> Wheel {
    let (library, licence, notices) = (body(400_000, 21), b"MIT License\n".to_vec(), body(30_000, 22));
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (path, data) in [
            ("rt/capi/librt.bin", &library),
            ("rt/LICENSE", &licence),
            ("rt/ThirdPartyNotices.txt", &notices),
            ("rt/other.py", &b"print()".to_vec()),
        ] {
            z.start_file(path, opts).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }
    Wheel { bytes: buf.into_inner(), library, licence, notices }
}

fn manifest(w: &Wheel) -> Manifest {
    let url = format!("https://packages.invalid/repo/resolve/{COMMIT}/{WHEEL}");
    let member = |name: &str, path: &str, data: &[u8]| ModelFile {
        name: name.into(),
        url: url.clone(),
        sha256: sha(data),
        bytes: data.len() as u64,
        archive_member: Some(ArchiveMember {
            path: path.into(),
            archive_sha256: sha(&w.bytes),
            archive_bytes: w.bytes.len() as u64,
        }),
    };
    Manifest {
        id: RUNTIME_ID.into(),
        version: RUNTIME_VERSION.into(),
        purpose: "Inference runtime".into(),
        licence: "MIT".into(),
        files: vec![
            member("librt.bin", "rt/capi/librt.bin", &w.library),
            member("LICENSE", "rt/LICENSE", &w.licence),
            member("ThirdPartyNotices.txt", "rt/ThirdPartyNotices.txt", &w.notices),
        ],
        runtime: RuntimeKind::Library,
        in_process: false,
    }
}

fn store(dir: &Path, server: &Server, m: Manifest) -> Store {
    Store::with_catalog(dir, vec![m]).with_url_override(Some(UrlOverride::Mirror(server.base.clone())))
}

fn wheel_path() -> String {
    format!("/{RUNTIME_ID}/{RUNTIME_VERSION}/{WHEEL}")
}

#[test]
fn a_clean_profile_reports_runtime_not_installed_and_installs_after_consent() {
    let w = wheel();
    let server = Server::start();
    server.add(&wheel_path(), w.bytes.clone(), Mode::Normal);
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path(), &server, manifest(&w));

    // Not installed: features get "runtime not installed" with the size, host and install action.
    let e = ove_models::runtime::library_path(&s);
    if ove_models::runtime::library_from_env().is_none() {
        let e = e.unwrap_err();
        assert!(ove_models::runtime::is_missing(&e));
        let msg = e.to_string();
        assert!(msg.starts_with("runtime not installed"), "{msg}");
        assert!(msg.contains("packages.invalid") && msg.contains("ove models install onnxruntime --yes"), "{msg}");
    }
    // A feature or an agent never downloads native code without the person, however small it is.
    for origin in [Origin::Feature, Origin::Agent] {
        match s.ensure_installed(RUNTIME_ID, origin, &mut |_| {}, &AtomicBool::new(false)).unwrap() {
            Ensure::NeedsConsent(n) => {
                assert_eq!(n.download_bytes, w.bytes.len() as u64);
                assert_eq!(n.host, "packages.invalid");
            }
            other => panic!("expected consent, got {other:?}"),
        }
    }
    assert!(server.requests().is_empty(), "nothing was downloaded before consent");

    // The person agrees: one download of the wheel, three verified members, and the wheel is gone.
    let mut last = None;
    let got = s.ensure_installed(RUNTIME_ID, Origin::Person, &mut |p| last = Some(p.clone()), &AtomicBool::new(false));
    let Ensure::Ready(paths) = got.unwrap() else { panic!("not installed") };
    let folder = dir.path().join(RUNTIME_ID).join(RUNTIME_VERSION);
    assert_eq!(paths, ["librt.bin", "LICENSE", "ThirdPartyNotices.txt"].map(|n| folder.join(n)).to_vec());
    assert_eq!(std::fs::read(&paths[0]).unwrap(), w.library);
    assert_eq!(std::fs::read(&paths[1]).unwrap(), w.licence, "the licence sits next to the library");
    assert_eq!(std::fs::read(&paths[2]).unwrap(), w.notices, "the notices sit next to the library");
    assert!(!folder.join(WHEEL).exists(), "the wheel is deleted after extraction");
    assert!(!folder.join("other.py").exists(), "only the pinned members are extracted");
    assert_eq!(server.requests().len(), 1, "one download for the whole runtime");
    let last = last.unwrap();
    assert_eq!((last.done, last.total), (w.bytes.len() as u64, w.bytes.len() as u64));
    if ove_models::runtime::library_from_env().is_none() {
        assert_eq!(ove_models::runtime::library_path(&s).unwrap(), paths[0]);
    }
    // Installed: agents get it too, and nothing downloads again.
    assert!(matches!(
        s.ensure_installed(RUNTIME_ID, Origin::Agent, &mut |_| {}, &AtomicBool::new(false)).unwrap(),
        Ensure::Ready(_)
    ));
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn a_corrupt_wheel_is_deleted_and_nothing_is_installed() {
    let w = wheel();
    let server = Server::start();
    server.add(&wheel_path(), w.bytes.clone(), Mode::Corrupt);
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path(), &server, manifest(&w));
    let e = s.install(RUNTIME_ID, &mut |_| {}, &AtomicBool::new(false)).unwrap_err();
    assert_eq!(e, ModelError::Corrupt { model: RUNTIME_ID.into(), file: WHEEL.into() });
    let folder = dir.path().join(RUNTIME_ID).join(RUNTIME_VERSION);
    assert!(!folder.join(WHEEL).exists() && !folder.join("librt.bin").exists());
    assert!(!s.is_installed(RUNTIME_ID));
}

#[test]
fn a_member_that_does_not_match_its_pin_is_refused() {
    let w = wheel();
    let server = Server::start();
    server.add(&wheel_path(), w.bytes.clone(), Mode::Normal);
    let dir = tempfile::tempdir().unwrap();
    let mut m = manifest(&w);
    m.files[0].sha256 = sha(b"some other library");
    let s = store(dir.path(), &server, m);
    let e = s.install(RUNTIME_ID, &mut |_| {}, &AtomicBool::new(false)).unwrap_err();
    assert_eq!(e, ModelError::Corrupt { model: RUNTIME_ID.into(), file: "librt.bin".into() });
    let folder = dir.path().join(RUNTIME_ID).join(RUNTIME_VERSION);
    assert!(!folder.join("librt.bin").exists() && !folder.join("librt.bin.part").exists());
    assert!(!s.is_installed(RUNTIME_ID));
}

#[test]
fn an_offline_install_from_a_folder_needs_no_network() {
    let w = wheel();
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path(), &server, manifest(&w));
    let offline = tempfile::tempdir().unwrap();

    // The folder does not hold the wheel yet.
    let e = s.install_from_folder(RUNTIME_ID, offline.path(), &mut |_| {}).unwrap_err();
    assert!(e.to_string().contains(WHEEL), "{e}");
    // A damaged copy is refused.
    std::fs::write(offline.path().join(WHEEL), &w.bytes[..w.bytes.len() - 1]).unwrap();
    assert!(matches!(s.install_from_folder(RUNTIME_ID, offline.path(), &mut |_| {}), Err(ModelError::Corrupt { .. })));
    // The right wheel installs.
    std::fs::write(offline.path().join(WHEEL), &w.bytes).unwrap();
    let out = s.install_from_folder(RUNTIME_ID, offline.path(), &mut |_| {}).unwrap();
    assert!(!out.already);
    assert_eq!(std::fs::read(&out.paths[0]).unwrap(), w.library);
    assert!(s.is_installed(RUNTIME_ID));
    assert!(server.requests().is_empty());
    assert!(offline.path().join(WHEEL).exists(), "the person's copy is left alone");
}

/// The real ONNX Runtime 1.32.0 wheels: set `OVE_ORT_WHEEL_DIR` to a folder holding the wheel for this platform under
/// its file name. Checks the pinned wheel and member hashes end to end, offline.
#[test]
#[ignore = "needs the real wheel (OVE_ORT_WHEEL_DIR)"]
fn the_pinned_wheel_installs_offline() {
    let folder = std::env::var("OVE_ORT_WHEEL_DIR").expect("set OVE_ORT_WHEEL_DIR");
    let platform = RuntimePlatform::current().expect("a shipped platform");
    let dir = tempfile::tempdir().unwrap();
    let s = Store::with_catalog(dir.path(), vec![runtime_manifest(platform)]);
    let out = s.install_from_folder(RUNTIME_ID, Path::new(&folder), &mut |_| {}).unwrap();
    assert_eq!(out.paths.len(), 3);
    let m = runtime_manifest(platform);
    for (p, f) in out.paths.iter().zip(&m.files) {
        assert_eq!(std::fs::metadata(p).unwrap().len(), f.bytes, "{}", f.name);
    }
    assert!(s.is_installed(RUNTIME_ID));
}
