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
