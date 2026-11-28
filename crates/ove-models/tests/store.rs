//! Downloader and store against a local HTTP server: good, corrupt, interrupted-then-resumed, no-space and cancel
//! cases, plus reinstall, remove and adopting files already on disk.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use common::{Mode, Server, body, manifest};
use ove_models::{ModelError, Progress, Store, UrlOverride};

fn store(dir: &std::path::Path, server: &Server, catalog: Vec<ove_models::Manifest>) -> Store {
    Store::with_catalog(dir, catalog).with_url_override(Some(UrlOverride::Mirror(server.base.clone())))
}

fn no_cancel() -> AtomicBool {
    AtomicBool::new(false)
}

#[test]
fn good_download_installs_every_file_and_records_it() {
    let server = Server::start();
    let (a, b) = (body(300_000, 1), body(70_000, 2));
    server.add("/demo/1/a.onnx", a.clone(), Mode::Normal);
    server.add("/demo/1/b.json", b.clone(), Mode::Normal);
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path(), &server, vec![manifest("demo", &[("a.onnx", &a), ("b.json", &b)])]);
    assert!(!s.is_installed("demo"));

    let mut seen: Vec<Progress> = Vec::new();
    let out = s.install("demo", &mut |p| seen.push(p.clone()), &no_cancel()).unwrap();
    assert!(!out.already);
    assert_eq!(out.paths, vec![dir.path().join("demo/1/a.onnx"), dir.path().join("demo/1/b.json")]);
    assert_eq!(std::fs::read(&out.paths[0]).unwrap(), a);
    assert_eq!(std::fs::read(&out.paths[1]).unwrap(), b);
    let last = seen.last().unwrap();
    assert_eq!((last.done, last.total), (370_000, 370_000));
    assert!(seen.windows(2).all(|w| w[0].done <= w[1].done), "progress goes forward");
    assert!(s.is_installed("demo"));
    let st = s.status("demo").unwrap();
    assert!(st.installed);
    assert_eq!(st.location.as_deref(), Some(dir.path().join("demo/1").as_path()));
    assert!(dir.path().join("installed.json").exists());
    assert!(!dir.path().join("demo/1/a.onnx.part").exists());

    // Installing again downloads nothing.
    let before = server.requests().len();
    let again = s.install("demo", &mut |_| {}, &no_cancel()).unwrap();
    assert!(again.already);
    assert_eq!(server.requests().len(), before);
}

#[test]
fn corrupt_download_is_deleted_and_named() {
    let server = Server::start();
    let a = body(200_000, 3);
    server.add("/bad/1/a.onnx", a.clone(), Mode::Corrupt);
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path(), &server, vec![manifest("bad", &[("a.onnx", &a)])]);
    let e = s.install("bad", &mut |_| {}, &no_cancel()).unwrap_err();
    assert_eq!(e, ModelError::Corrupt { model: "bad".into(), file: "a.onnx".into() });
    let msg = e.to_string();
    assert!(msg.contains("a.onnx") && msg.contains("corrupt"), "{msg}");
    assert!(!dir.path().join("bad/1/a.onnx").exists());
    assert!(!dir.path().join("bad/1/a.onnx.part").exists());
    assert!(!s.is_installed("bad"));
}

#[test]
fn interrupted_download_resumes_from_the_received_bytes() {
    let server = Server::start();
    let a = body(500_000, 4);
    // The connection drops at 60 %; the downloader asks for the rest with Range and finishes.
    server.add("/cut/1/a.onnx", a.clone(), Mode::CutFirstAfter(300_000));
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path(), &server, vec![manifest("cut", &[("a.onnx", &a)])]);
    let out = s.install("cut", &mut |_| {}, &no_cancel()).unwrap();
    assert_eq!(std::fs::read(&out.paths[0]).unwrap(), a);
    let reqs = server.requests();
    assert_eq!(reqs.len(), 2, "{reqs:?}");
    assert_eq!(reqs[0].range_start, None);
    assert_eq!(reqs[1].range_start, Some(300_000));
}

#[test]
fn a_partial_file_from_an_earlier_run_is_resumed() {
    let server = Server::start();
    let a = body(400_000, 5);
    server.add("/part/1/a.onnx", a.clone(), Mode::Normal);
    let dir = tempfile::tempdir().unwrap();
    // An earlier run stopped at 60 %.
    std::fs::create_dir_all(dir.path().join("part/1")).unwrap();
    std::fs::write(dir.path().join("part/1/a.onnx.part"), &a[..240_000]).unwrap();
    let s = store(dir.path(), &server, vec![manifest("part", &[("a.onnx", &a)])]);
    let mut first = None;
    let out = s
        .install(
            "part",
            &mut |p| {
                first.get_or_insert(p.done);
            },
            &no_cancel(),
        )
        .unwrap();
    assert_eq!(first, Some(240_000), "progress starts at the bytes already received");
    assert_eq!(std::fs::read(&out.paths[0]).unwrap(), a);
    assert_eq!(server.requests().iter().map(|r| r.range_start).collect::<Vec<_>>(), vec![Some(240_000)]);
}

#[test]
fn a_server_that_ignores_range_restarts_cleanly() {
    let server = Server::start();
    let a = body(150_000, 6);
    server.add("/norange/1/a.onnx", a.clone(), Mode::IgnoreRange);
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("norange/1")).unwrap();
    std::fs::write(dir.path().join("norange/1/a.onnx.part"), &a[..50_000]).unwrap();
    let s = store(dir.path(), &server, vec![manifest("norange", &[("a.onnx", &a)])]);
    let out = s.install("norange", &mut |_| {}, &no_cancel()).unwrap();
    assert_eq!(std::fs::read(&out.paths[0]).unwrap(), a);
}

#[test]
fn not_enough_space_refuses_before_downloading() {
    let server = Server::start();
    let a = body(1_000_000, 7);
    server.add("/big/1/a.onnx", a.clone(), Mode::Normal);
    let dir = tempfile::tempdir().unwrap();
    // 1.05 MB free: less than the size plus 10 % (1.1 MB).
    let s = store(dir.path(), &server, vec![manifest("big", &[("a.onnx", &a)])]).with_free_space(|_| Ok(1_050_000));
    let e = s.install("big", &mut |_| {}, &no_cancel()).unwrap_err();
    assert_eq!(e, ModelError::NoSpace { model: "big".into(), needed: 1_100_000, free: 1_050_000 });
    let msg = e.to_string();
    assert!(msg.contains("needs 1.1 MB") && msg.contains("1.0 MB are free"), "{msg}");
    assert!(server.requests().is_empty(), "nothing was downloaded");

    // Enough space: the same install goes ahead.
    let s = store(dir.path(), &server, vec![manifest("big", &[("a.onnx", &a)])]).with_free_space(|_| Ok(1_100_000));
    s.install("big", &mut |_| {}, &no_cancel()).unwrap();
}

#[test]
fn cancel_stops_within_a_second_and_leaves_nothing_visible() {
    let server = Server::start();
    let a = body(2_000_000, 8);
    server.add("/slow/1/a.onnx", a.clone(), Mode::StallAfter(100_000));
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path(), &server, vec![manifest("slow", &[("a.onnx", &a)])]);
    let cancel = Arc::new(AtomicBool::new(false));
    let c = cancel.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        c.store(true, Ordering::SeqCst);
        Instant::now()
    });
    let e = s.install("slow", &mut |_| {}, &cancel).unwrap_err();
    let stopped = Instant::now();
    let cancelled_at = canceller.join().unwrap();
    assert_eq!(e, ModelError::Cancelled);
    let took = stopped.duration_since(cancelled_at);
    assert!(took < Duration::from_secs(1), "stopped {took:?} after the cancel");
    assert!(!s.is_installed("slow"));
    assert!(!dir.path().join("slow/1/a.onnx").exists());
    assert!(!dir.path().join("slow/1/a.onnx.part").exists());
}

#[test]
fn one_download_per_model_at_a_time() {
    let server = Server::start();
    let a = body(1_000_000, 9);
    server.add("/one/1/a.onnx", a.clone(), Mode::StallAfter(10_000));
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let catalog = vec![manifest("one", &[("a.onnx", &a)])];
    let cancel = Arc::new(AtomicBool::new(false));
    let (c, base, cat) = (cancel.clone(), server.base.clone(), catalog.clone());
    let r = root.clone();
    let first = std::thread::spawn(move || {
        let s = Store::with_catalog(r, cat).with_url_override(Some(UrlOverride::Mirror(base)));
        s.install("one", &mut |_| {}, &c)
    });
    // Wait until the first download is under way.
    let t0 = Instant::now();
    while server.requests().is_empty() && t0.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    let s = store(&root, &server, catalog);
    assert_eq!(s.install("one", &mut |_| {}, &no_cancel()).unwrap_err(), ModelError::Busy("one".into()));
    cancel.store(true, Ordering::SeqCst);
    assert_eq!(first.join().unwrap().unwrap_err(), ModelError::Cancelled);
}

#[test]
fn remove_deletes_files_and_reports_the_space_freed() {
    let server = Server::start();
    let a = body(123_456, 10);
    server.add("/rm/1/a.onnx", a.clone(), Mode::Normal);
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path(), &server, vec![manifest("rm", &[("a.onnx", &a)])]);
    s.install("rm", &mut |_| {}, &no_cancel()).unwrap();
    assert_eq!(s.remove("rm").unwrap(), Some(123_456));
    assert!(!s.is_installed("rm"));
    assert!(!dir.path().join("rm").exists(), "the model's folders are gone");
    assert_eq!(s.remove("rm").unwrap(), None);
    assert_eq!(s.remove("nope").unwrap_err(), ModelError::UnknownModel("nope".into()));
}

#[test]
fn a_file_deleted_behind_the_store_counts_as_not_installed() {
    let server = Server::start();
    let a = body(10_000, 11);
    server.add("/gone/1/a.onnx", a.clone(), Mode::Normal);
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path(), &server, vec![manifest("gone", &[("a.onnx", &a)])]);
    let out = s.install("gone", &mut |_| {}, &no_cancel()).unwrap();
    std::fs::remove_file(&out.paths[0]).unwrap();
    assert!(!s.is_installed("gone"));
    // Installing again fetches it.
    assert!(!s.install("gone", &mut |_| {}, &no_cancel()).unwrap().already);
}

#[test]
fn files_already_on_disk_are_adopted_in_place_without_downloading() {
    let server = Server::start();
    let a = body(90_000, 12);
    let dir = tempfile::tempdir().unwrap();
    // A file from before the store, at the store root (where whisper models used to be saved).
    std::fs::write(dir.path().join("ggml-demo.bin"), &a).unwrap();
    let s = store(dir.path(), &server, vec![manifest("legacy", &[("ggml-demo.bin", &a)])]);
    assert!(s.adopt_in_place("legacy", dir.path()).unwrap());
    assert_eq!(s.installed_paths("legacy").unwrap(), vec![dir.path().join("ggml-demo.bin")]);
    assert!(s.install("legacy", &mut |_| {}, &no_cancel()).unwrap().already);
    assert!(server.requests().is_empty());
    let index = std::fs::read_to_string(dir.path().join("installed.json")).unwrap();
    assert!(index.contains("\"path\": \"ggml-demo.bin\"") && index.contains("\"adopted\": true"), "{index}");

    // A damaged file is not adopted.
    let mut damaged = a.clone();
    damaged[5] ^= 1;
    let other = tempfile::tempdir().unwrap();
    std::fs::write(other.path().join("ggml-demo.bin"), &damaged).unwrap();
    let s2 = store(other.path(), &server, vec![manifest("legacy", &[("ggml-demo.bin", &a)])]);
    assert!(!s2.adopt_in_place("legacy", other.path()).unwrap());
    assert!(!s2.is_installed("legacy"));
}

#[test]
fn a_new_manifest_version_needs_a_new_install() {
    let server = Server::start();
    let a = body(5_000, 13);
    server.add("/ver/1/a.onnx", a.clone(), Mode::Normal);
    let dir = tempfile::tempdir().unwrap();
    store(dir.path(), &server, vec![manifest("ver", &[("a.onnx", &a)])])
        .install("ver", &mut |_| {}, &no_cancel())
        .unwrap();
    let mut v2 = manifest("ver", &[("a.onnx", &a)]);
    v2.version = "2".into();
    assert!(!store(dir.path(), &server, vec![v2]).is_installed("ver"));
}

#[test]
fn a_large_model_asked_for_by_an_agent_downloads_nothing() {
    use ove_models::{Ensure, Origin};
    let server = Server::start();
    let big = body(50_000_001, 14);
    server.add("/large/1/w.bin", big.clone(), Mode::Normal);
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path(), &server, vec![manifest("large", &[("w.bin", &big)])]);
    for origin in [Origin::Agent, Origin::Feature] {
        match s.ensure_installed("large", origin, &mut |_| {}, &no_cancel()).unwrap() {
            Ensure::NeedsConsent(n) => {
                assert_eq!((n.id.as_str(), n.bytes), ("large", 50_000_001));
                assert_eq!(n.install, "ove models install large --yes");
                assert!(n.message.contains("not installed") && n.message.contains("50.1 MB"), "{}", n.message);
            }
            other => panic!("expected NeedsConsent, got {other:?}"),
        }
    }
    assert!(server.requests().is_empty(), "nothing was downloaded");
    assert!(!dir.path().join("large").exists());

    // The person confirms: it downloads.
    let mut last = None;
    let got = s.ensure_installed("large", Origin::Person, &mut |p| last = Some(p.clone()), &no_cancel()).unwrap();
    assert_eq!(got, Ensure::Ready(vec![dir.path().join("large/1/w.bin")]));
    assert_eq!(last.map(|p| p.done), Some(50_000_001));
    // Once installed, everyone gets it.
    assert!(matches!(s.ensure_installed("large", Origin::Agent, &mut |_| {}, &no_cancel()).unwrap(), Ensure::Ready(_)));
}

#[test]
fn a_small_model_downloads_for_any_origin_with_progress() {
    use ove_models::{Ensure, Origin};
    let server = Server::start();
    let small = body(2_000_000, 15);
    server.add("/small/1/m.onnx", small.clone(), Mode::Normal);
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path(), &server, vec![manifest("small", &[("m.onnx", &small)])]);
    let mut steps = 0;
    let got = s.ensure_installed("small", Origin::Agent, &mut |_| steps += 1, &no_cancel()).unwrap();
    assert!(matches!(got, Ensure::Ready(_)));
    assert!(steps > 2, "progress was reported ({steps} updates)");
}

#[test]
fn files_left_in_the_root_by_earlier_versions_are_adopted() {
    let server = Server::start();
    let a = body(40_000, 16);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ggml-old.bin"), &a).unwrap();
    let s =
        store(dir.path(), &server, vec![manifest("old", &[("ggml-old.bin", &a)]), manifest("other", &[("x.bin", &a)])]);
    assert_eq!(s.adopt_root_files(), vec!["old".to_string()]);
    assert!(s.is_installed("old") && !s.is_installed("other"));
    assert!(s.adopt_root_files().is_empty(), "a second call finds it recorded");
}

#[test]
fn removal_waits_for_the_job_that_uses_the_model() {
    let server = Server::start();
    let a = body(20_000, 17);
    server.add("/busy/1/a.onnx", a.clone(), Mode::Normal);
    let dir = tempfile::tempdir().unwrap();
    let s = Arc::new(store(dir.path(), &server, vec![manifest("busy", &[("a.onnx", &a)])]));
    s.install("busy", &mut |_| {}, &no_cancel()).unwrap();
    let lease = s.lease("busy");
    assert!(s.in_use("busy"));
    let s2 = s.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let remover = std::thread::spawn(move || s2.remove_when_free("busy", &mut || tx.send(()).unwrap()));
    rx.recv_timeout(Duration::from_secs(5)).expect("the removal said it waits");
    std::thread::sleep(Duration::from_millis(200));
    assert!(s.is_installed("busy"), "nothing is removed while the job runs");
    drop(lease);
    assert_eq!(remover.join().unwrap().unwrap(), Some(20_000));
    assert!(!s.is_installed("busy") && !s.in_use("busy"));
    // Not in use: removed at once, without waiting.
    s.install("busy", &mut |_| {}, &no_cancel()).unwrap();
    let mut waited = false;
    assert_eq!(s.remove_when_free("busy", &mut || waited = true).unwrap(), Some(20_000));
    assert!(!waited);
}
