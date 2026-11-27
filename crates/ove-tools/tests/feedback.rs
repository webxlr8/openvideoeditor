//! `send_feedback`: reports go to a local queue only, exact duplicates are dropped, and a session files at most 8.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::Path;
use std::sync::Arc;

use ove_core::revision::{Actor, ActorKind, Source};
use ove_tools::tools::feedback;
use ove_tools::{BundleCtx, Fail, Receipt, Session, Surface};
use serde_json::{Value, json};

fn agent(lib: &Path, config: &Path, session: &str) -> BundleCtx {
    let mut c = BundleCtx::open(Some(lib), Surface::Mcp, Source::Mcp).unwrap();
    c.actor = Some(Actor {
        kind: ActorKind::Mcp,
        session: Some(session.into()),
        turn: None,
        client: Some("stand-in agent".into()),
    });
    c.session = Session::with(
        Some(config.to_path_buf()),
        Arc::new(ove_core::SystemClock),
        Arc::new(ove_generate::keys::MemoryKeyStore::default()),
    );
    c
}

fn call(ctx: &mut BundleCtx, name: &str, a: Value) -> Result<Receipt, Fail> {
    ove_tools::call(ctx, name, a.as_object().unwrap())
}

fn report(n: usize) -> Value {
    json!({"category": "bug", "severity": "medium", "summary": format!("Report {n}: export stops at 99% on a long project")})
}

#[test]
fn reports_are_queued_locally_deduplicated_and_limited_to_eight_per_session() {
    let d = tempfile::tempdir().unwrap();
    let (lib, config) = (d.path().join("lib"), d.path().join("config"));
    ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let mut ctx = agent(&lib, &config, "s1");

    // Tools called before go into the report, with the last refusal's code.
    assert_eq!(call(&mut ctx, "show_project", json!({"project": "Nope"})).unwrap_err().code, "project.notFound");
    let first = call(&mut ctx, "send_feedback", report(1)).unwrap();
    assert_eq!((first.json["stored"].as_bool(), first.json["sent"].as_bool()), (Some(true), Some(false)));
    let queued = feedback::list(&config);
    assert_eq!(queued.len(), 1);
    let q = &queued[0];
    assert_eq!(q.id, first.json["id"].as_str().unwrap());
    assert_eq!((q.category.as_str(), q.severity.as_str(), q.surface.as_str()), ("bug", "medium", "mcp"));
    assert_eq!(q.client.as_deref(), Some("stand-in agent"));
    assert_eq!(q.recent_tools, ["show_project"]);
    assert_eq!(q.last_error.as_deref(), Some("project.notFound"));
    assert!(!q.app_version.is_empty() && !q.os.is_empty());

    // An exact duplicate (spacing aside) is not stored again, and does not count.
    let mut same = report(1);
    same["summary"] = "Report 1:  export stops at 99%\non a long project".into();
    let dup = call(&mut ctx, "send_feedback", same).unwrap();
    assert_eq!((dup.json["stored"].as_bool(), dup.json["duplicate"].as_bool()), (Some(false), Some(true)));
    assert_eq!(dup.json["id"], first.json["id"]);
    assert_eq!(feedback::list(&config).len(), 1);

    // Seven more fit; the ninth is refused and nothing new is stored.
    for n in 2..=8 {
        assert_eq!(call(&mut ctx, "send_feedback", report(n)).unwrap().json["stored"], true, "report {n}");
    }
    let e = call(&mut ctx, "send_feedback", report(9)).unwrap_err();
    assert_eq!(e.code, "feedback.limit");
    assert_eq!(feedback::list(&config).len(), 8);

    // Another session has its own allowance, but a report already queued is still a duplicate.
    let mut other = agent(&lib, &config, "s2");
    assert_eq!(call(&mut other, "send_feedback", report(3)).unwrap().json["duplicate"], true);
    assert_eq!(call(&mut other, "send_feedback", report(9)).unwrap().json["stored"], true);
    assert_eq!(feedback::list(&config).len(), 9);

    // Bad input is refused before anything is stored.
    for bad in [
        json!({"category": "rant", "severity": "low", "summary": "x"}),
        json!({"category": "bug", "severity": "urgent", "summary": "x"}),
        json!({"category": "bug", "severity": "low", "summary": "   "}),
        json!({"category": "bug", "severity": "low", "summary": "x".repeat(2001)}),
        json!({"category": "bug", "severity": "low", "summary": "x", "url": "https://example.invalid"}),
    ] {
        assert!(call(&mut other, "send_feedback", bad.clone()).is_err(), "{bad}");
    }
    assert_eq!(feedback::list(&config).len(), 9);
}
