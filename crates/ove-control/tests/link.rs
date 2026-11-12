#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::sync::Arc;
use std::time::Duration;

use ove_control::{CallError, Refusal, call, read_endpoint, serve};
use serde_json::json;

#[test]
fn request_reply_token_and_cleanup() {
    let d = tempfile::tempdir().unwrap();
    assert!(matches!(call(d.path(), "context", json!({}), Duration::from_secs(1)), Err(CallError::NotRunning)));
    let server = serve(
        d.path(),
        "0.1.0",
        Arc::new(|m, p| match m {
            "echo" => Ok(json!({"got": p})),
            _ => Err(Refusal::new("method.unknown", format!("no method {m}"))),
        }),
    )
    .unwrap();
    let ep = read_endpoint(d.path()).unwrap();
    assert_eq!(&ep, server.endpoint());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(ove_control::endpoint_path(d.path())).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "endpoint file is private");
    }
    let v = call(d.path(), "echo", json!({"a": 1}), Duration::from_secs(2)).unwrap();
    assert_eq!(v, json!({"got": {"a": 1}}));
    match call(d.path(), "nope", json!(null), Duration::from_secs(2)) {
        Err(CallError::Refused(r)) => assert_eq!(r.code, "method.unknown"),
        other => panic!("{other:?}"),
    }
    // A wrong token is refused.
    let mut bad = ep.clone();
    bad.token = "0".repeat(48);
    match ove_control::call_endpoint(&bad, "echo", json!({}), Duration::from_secs(2)) {
        Err(CallError::Refused(r)) => assert_eq!(r.code, "token.invalid"),
        other => panic!("{other:?}"),
    }
    drop(server);
    assert!(read_endpoint(d.path()).is_none(), "endpoint removed on shutdown");
    assert!(matches!(call(d.path(), "echo", json!({}), Duration::from_secs(1)), Err(CallError::NotRunning)));
}

#[test]
fn a_panicking_handler_is_a_refusal_and_the_server_keeps_going() {
    let d = tempfile::tempdir().unwrap();
    let _server = serve(
        d.path(),
        "0.1.0",
        Arc::new(|m, _| match m {
            "boom" => panic!("handler bug"),
            _ => Ok(json!("fine")),
        }),
    )
    .unwrap();
    match call(d.path(), "boom", json!({}), Duration::from_secs(2)) {
        Err(CallError::Refused(r)) => {
            assert_eq!(r.code, "app.internal");
            assert!(r.message.contains("boom"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(call(d.path(), "ok", json!({}), Duration::from_secs(2)).unwrap(), json!("fine"));
}

#[test]
fn connections_over_the_cap_are_refused_then_served_again() {
    let d = tempfile::tempdir().unwrap();
    let server = serve(d.path(), "0.1.0", Arc::new(|_, _| Ok(json!("fine")))).unwrap();
    let addr = std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, server.endpoint().port));
    // Idle connections hold every slot (their handlers wait for a request line).
    let held: Vec<_> = (0..ove_control::MAX_CONNECTIONS).map(|_| std::net::TcpStream::connect(addr).unwrap()).collect();
    std::thread::sleep(Duration::from_millis(200));
    match call(d.path(), "x", json!({}), Duration::from_secs(2)) {
        Err(CallError::Refused(r)) => assert_eq!(r.code, "server.busy"),
        other => panic!("{other:?}"),
    }
    drop(held);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match call(d.path(), "x", json!({}), Duration::from_secs(2)) {
            Ok(v) => break assert_eq!(v, json!("fine")),
            Err(_) if std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => panic!("slots were not freed: {e:?}"),
        }
    }
}

#[cfg(unix)]
#[test]
fn endpoint_file_is_private_even_when_it_existed() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let file = ove_control::endpoint_path(d.path());
    let tmp = file.with_extension("json.tmp");
    for p in [&file, &tmp] {
        std::fs::write(p, "{}").unwrap();
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    let _server = serve(d.path(), "0.1.0", Arc::new(|_, _| Ok(json!(null)))).unwrap();
    assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(read_endpoint(d.path()).is_some());
}
