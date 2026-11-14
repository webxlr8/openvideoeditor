//! Runs the scripted demo assistant on a local port:
//! `cargo run -p ove-agent --example mock_server -- 127.0.0.1:8787 [chunk-delay-ms]`
//! then start the app with `OVE_ANTHROPIC_BASE_URL=http://127.0.0.1:8787 ANTHROPIC_API_KEY=demo-key-not-real-123456`
//! (a release build also needs `OVE_DEV_ENDPOINTS=1` to accept the override).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "developer example, not shipped: failing loudly on a broken setup is the point"
)]

use ove_agent::mock::{MockServer, Reply, count_tokens_reply, demo_brain};
use std::time::Duration;

fn main() {
    let mut args = std::env::args().skip(1);
    let addr = args.next().unwrap_or_else(|| "127.0.0.1:8787".into());
    let delay = args.next().and_then(|d| d.parse().ok()).map(Duration::from_millis);
    let s = MockServer::start_routed_on(&addr, move |n, path, req| {
        if let Some(r) = count_tokens_reply(path) {
            return r;
        }
        match (demo_brain(n, req), delay) {
            (Reply::Stream { chunks, .. }, Some(delay)) => Reply::Stream { chunks, delay },
            (reply, _) => reply,
        }
    });
    println!("mock Messages API at {}", s.url);
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
