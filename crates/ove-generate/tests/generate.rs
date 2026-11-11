//! Generation against local stand-ins for fal.ai, OpenAI and ElevenLabs: plan → placeholder → run → finish, cancel,
//! auth errors. Environment variables point the providers at the stand-in, so the tests run serially.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_generate::keys::{KeyStore, MemoryKeyStore};
use ove_generate::providers::b64_encode;
use ove_generate::{Kind, Probed, Provider, Request, Status, execute, finish_ops, new_job, plan};
use serde_json::{Value, json};

type Seen = Arc<Mutex<Vec<(String, String, BTreeMap<String, String>, String)>>>;

/// A tiny HTTP server: `route(method, path, body) -> (status, content type, body)`.
fn serve(
    route: impl Fn(&str, &str, &str, &Seen) -> (u16, &'static str, Vec<u8>) + Send + Sync + 'static,
) -> (String, Seen) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let seen: Seen = Arc::default();
    let s2 = seen.clone();
    let route = Arc::new(route);
    std::thread::spawn(move || {
        for c in l.incoming().flatten() {
            let (route, seen) = (route.clone(), s2.clone());
            std::thread::spawn(move || {
                let mut r = BufReader::new(c.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let mut parts = line.split_whitespace();
                let (method, target) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
                let mut headers = BTreeMap::new();
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                    if let Some((k, v)) = h.split_once(':') {
                        headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
                    }
                }
                let len: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
                let mut body = vec![0; len];
                r.read_exact(&mut body).unwrap();
                let body = String::from_utf8_lossy(&body).to_string();
                seen.lock().unwrap().push((method.clone(), target.clone(), headers, body.clone()));
                let path = target.split('?').next().unwrap_or("").to_string();
                let (code, ctype, out) = route(&method, &path, &body, &seen);
                let mut w = c;
                let _ = write!(
                    w,
                    "HTTP/1.1 {code} X\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    out.len()
                );
                let _ = w.write_all(&out);
            });
        }
    });
    (base, seen)
}

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-fake-png-bytes-for-tests-0123456789";

fn doc() -> Document {
    let ops: Vec<Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "t_overlay", "name": "Overlay", "order": 0, "type": "video"}},
        {"op": "addTrack", "track": {"id": "t_main", "name": "Main", "order": 1, "type": "video"}},
        {"op": "addTrack", "track": {"id": "t_audio", "name": "Audio", "order": 2, "type": "audio"}},
        {"op": "addItem", "item": {"id": "t", "trackId": "t_overlay", "type": "text", "startTime": 0, "endTime": 2, "textContent": "x"}}
    ]))
    .unwrap();
    apply_batch(&Document::new("p", "P", 1), &ops).unwrap().doc
}

#[test]
fn providers_end_to_end() {
    unsafe { std::env::set_var("OVE_GENERATE_POLL_MS", "10") };
    let polls = Arc::new(Mutex::new(0));
    let p2 = polls.clone();
    let (base, seen) = serve(move |m, path, body, _| {
        let b: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        match (m, path) {
            // fal queue
            ("POST", p) if p.starts_with("/fal-ai/") => {
                let id = if p.contains("never") { "stuck" } else { "r1" };
                let host = std::env::var("OVE_FAL_QUEUE_URL").unwrap();
                (200, "application/json", serde_json::to_vec(&json!({"request_id": id, "status_url": format!("{host}/fal-ai/x/requests/{id}/status"), "response_url": format!("{host}/fal-ai/x/requests/{id}"), "cancel_url": format!("{host}/fal-ai/x/requests/{id}/cancel")})).unwrap())
            }
            ("GET", "/fal-ai/x/requests/r1/status") => {
                let mut n = p2.lock().unwrap();
                *n += 1;
                let st = match *n { 1 => "IN_QUEUE", 2 => "IN_PROGRESS", _ => "COMPLETED" };
                (200, "application/json", serde_json::to_vec(&json!({"status": st})).unwrap())
            }
            ("GET", "/fal-ai/x/requests/stuck/status") => (200, "application/json", br#"{"status":"IN_QUEUE","queue_position":3}"#.to_vec()),
            ("PUT", "/fal-ai/x/requests/stuck/cancel") => (202, "application/json", br#"{"status":"CANCELLATION_REQUESTED"}"#.to_vec()),
            ("GET", "/fal-ai/x/requests/r1") => {
                let host = std::env::var("OVE_FAL_QUEUE_URL").unwrap();
                (200, "application/json", serde_json::to_vec(&json!({"images": [{"url": format!("{host}/files/out.png"), "width": 1024, "height": 576, "content_type": "image/png"}], "seed": 7})).unwrap())
            }
            ("GET", "/files/out.png") => (200, "image/png", PNG.to_vec()),
            // OpenAI
            ("POST", "/v1/images/generations") => (200, "application/json", serde_json::to_vec(&json!({"data": [{"b64_json": b64_encode(PNG)}]})).unwrap()),
            // ElevenLabs
            ("GET", "/v2/voices") => (200, "application/json", br#"{"voices":[{"voice_id":"abcdefghijklmnopqrst","name":"Narrator","category":"premade","labels":{"accent":"british"}}],"has_more":false,"total_count":1}"#.to_vec()),
            ("POST", p) if p.starts_with("/v1/text-to-speech/") || p == "/v1/sound-generation" || p == "/v1/music" => {
                let _ = b;
                (200, "audio/mpeg", vec![0xFF; 4096])
            }
            _ => (404, "application/json", br#"{"detail":"not found"}"#.to_vec()),
        }
    });
    unsafe {
        std::env::set_var("OVE_FAL_QUEUE_URL", &base);
        std::env::set_var("OVE_OPENAI_URL", &base);
        std::env::set_var("OVE_ELEVENLABS_URL", &base);
    }
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path();
    let keys = MemoryKeyStore::default();
    let d = doc();

    // Without a key: refused with a pointer to settings, nothing sent.
    let req = Request { prompt: "a lighthouse at dawn, wide".into(), ..Default::default() };
    let p = plan(&d, Kind::Image, &req, "i_gen").unwrap();
    assert_eq!(p.track_id, "t_overlay");
    assert!((p.start - 2.0).abs() < 1e-9, "after the last overlay item");
    assert_eq!(p.estimate_usd, Some(0.025));
    let d1 = apply_batch(&d, &p.ops).unwrap().doc;
    assert!(d1.item("i_gen").unwrap().is_pending());
    let mut job = new_job(&p, &req, "g_1", 1);
    let e = execute(bundle, &mut job, &keys, &|| false).unwrap_err();
    assert_eq!(e.code, "key.missing");
    assert_eq!(job.status, Status::Failed);

    // fal image: queue → progress → result downloaded into the bundle; placeholder filled.
    keys.set(Provider::Fal, "fal-test-key-0123456789").unwrap();
    let mut job = new_job(&p, &req, "g_2", 1);
    execute(bundle, &mut job, &keys, &|| false).unwrap();
    assert_eq!(job.status, Status::Succeeded);
    assert_eq!(job.output.as_deref(), Some("media/generated/g_2.png"));
    assert_eq!(std::fs::read(bundle.join("media/generated/g_2.png")).unwrap(), PNG);
    let saved = ove_generate::job::load(bundle, "g_2").unwrap();
    assert_eq!(saved.status, Status::Succeeded);
    let submit = seen.lock().unwrap().iter().find(|s| s.0 == "POST" && s.1 == "/fal-ai/flux/dev").cloned().unwrap();
    assert_eq!(submit.2.get("authorization").map(String::as_str), Some("Key fal-test-key-0123456789"));
    let sent: Value = serde_json::from_str(&submit.3).unwrap();
    assert_eq!(sent["image_size"], "landscape_16_9");
    let fin = finish_ops(
        &d1,
        &job,
        job.output.as_deref().unwrap(),
        &Probed { width: Some(1024), height: Some(576), ..Default::default() },
        2,
    );
    let d2 = apply_batch(&d1, &fin).unwrap().doc;
    let it = d2.item("i_gen").unwrap();
    assert!(!it.is_pending());
    let mid = it.media_id().unwrap().to_string();
    assert!(d2.registry[&mid].owned_by_bundle);
    assert_eq!(d2.registry[&mid].native_path, "media/generated/g_2.png");

    // fal video from a first frame: the image-to-video endpoint gets a data URI and a string duration.
    let frame = bundle.join("first.png");
    std::fs::write(&frame, PNG).unwrap();
    let vreq =
        Request { prompt: "slow push in".into(), image: Some(frame), duration: Some(10.0), ..Default::default() };
    let vp = plan(&d2, Kind::Video, &vreq, "i_vid").unwrap();
    assert_eq!(vp.estimate_usd, Some(0.56));
    assert!(
        plan(&d2, Kind::Video, &Request { duration: Some(7.0), ..vreq.clone() }, "x").unwrap_err().contains("5 or 10")
    );
    let (endpoint, input) = ove_generate::providers::fal::input(&vp.model, &vreq, "16:9").unwrap();
    assert!(endpoint.ends_with("image-to-video"));
    assert!(input["image_url"].as_str().unwrap().starts_with("data:image/png;base64,"));
    assert_eq!(input["duration"], "10");

    // OpenAI image: base64 in the reply.
    keys.set(Provider::OpenAi, "sk-test-0123456789abcdef").unwrap();
    let oreq = Request {
        prompt: "poster".into(),
        model: Some("openai:gpt-image-1".into()),
        aspect: Some("1:1".into()),
        ..Default::default()
    };
    let op = plan(&d2, Kind::Image, &oreq, "i_o").unwrap();
    let mut oj = new_job(&op, &oreq, "g_3", 1);
    execute(bundle, &mut oj, &keys, &|| false).unwrap();
    assert_eq!(std::fs::read(bundle.join("media/generated/g_3.png")).unwrap(), PNG);

    // ElevenLabs voice by name, then music and sound effects.
    keys.set(Provider::ElevenLabs, "el-test-0123456789abcdef").unwrap();
    let vr = Request { prompt: "Welcome to the harbour.".into(), voice: Some("narrator".into()), ..Default::default() };
    let vpl = plan(&d2, Kind::Voice, &vr, "i_v").unwrap();
    assert_eq!(vpl.track_id, "t_audio");
    let mut vj = new_job(&vpl, &vr, "g_4", 1);
    execute(bundle, &mut vj, &keys, &|| false).unwrap();
    assert_eq!(vj.details["voiceId"], "abcdefghijklmnopqrst");
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|s| s.1.starts_with("/v1/text-to-speech/abcdefghijklmnopqrst?output_format=mp3_44100_128"))
    );
    for (kind, path) in [(Kind::Music, "/v1/music"), (Kind::Sfx, "/v1/sound-generation")] {
        let r = Request { prompt: "calm piano".into(), duration: Some(12.0), ..Default::default() };
        let pl = plan(&d2, kind, &r, "i_a").unwrap();
        let mut j = new_job(&pl, &r, &format!("g_{}", kind.as_str()), 1);
        execute(bundle, &mut j, &keys, &|| false).unwrap();
        let body = seen.lock().unwrap().iter().rev().find(|s| s.1.starts_with(path)).map(|s| s.3.clone()).unwrap();
        let v: Value = serde_json::from_str(&body).unwrap();
        if kind == Kind::Music {
            assert_eq!(v["music_length_ms"], 12000);
        } else {
            assert_eq!(v["duration_seconds"], 12.0);
        }
        // Audio takes its real length when finished.
        let d3 = apply_batch(&d2, &pl.ops).unwrap().doc;
        let ops = finish_ops(
            &d3,
            &j,
            j.output.as_deref().unwrap(),
            &Probed { duration_us: Some(11_500_000), ..Default::default() },
            3,
        );
        let d4 = apply_batch(&d3, &ops).unwrap().doc;
        assert_eq!(d4.item("i_a").unwrap().end - d4.item("i_a").unwrap().start, 11_500_000);
    }

    // Cancel while queued: the cancel URL is called and the job ends cancelled.
    let creq =
        Request { prompt: "never finishes".into(), model: Some("fal:fal-ai/never/ends".into()), ..Default::default() };
    let cp = plan(&d2, Kind::Image, &creq, "i_c").unwrap();
    let mut cj = new_job(&cp, &creq, "g_5", 1);
    let b2 = bundle.to_path_buf();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(80));
        ove_generate::job::request_cancel(&b2, "g_5").unwrap();
    });
    let e = execute(bundle, &mut cj, &keys, &|| false).unwrap_err();
    assert_eq!(e.code, "cancelled");
    assert_eq!(cj.status, Status::Cancelled);
    assert!(seen.lock().unwrap().iter().any(|s| s.0 == "PUT" && s.1.ends_with("/stuck/cancel")));
    assert_eq!(ove_generate::job::list(bundle).len(), 7);
}

#[test]
fn auth_errors_name_the_fix() {
    let (base, _) = serve(|_, _, _, _| (401, "application/json", br#"{"detail":"Invalid API key"}"#.to_vec()));
    let e = ove_generate::providers::http_error("x", ureq::get(&format!("{base}/a")).call().unwrap_err());
    assert_eq!(e.code, "provider.auth");
    assert!(e.message.contains("Invalid API key") && e.message.contains("Settings"), "{}", e.message);
}

#[test]
fn catalog_and_plans() {
    use ove_generate::catalog::{find, models};
    for k in [Kind::Image, Kind::Video, Kind::Voice, Kind::Music, Kind::Sfx] {
        assert_eq!(models().iter().filter(|m| m.kind == k && m.default).count(), 1, "{k:?}");
    }
    assert!(find(Some("fal:owner/some-new-model"), Kind::Image).is_ok(), "fal pass-through");
    assert!(find(Some("elevenlabs:music"), Kind::Image).is_err());
    let d = doc();
    assert!(plan(&d, Kind::Image, &Request { prompt: " ".into(), ..Default::default() }, "x").is_err());
    let p = plan(&d, Kind::Voice, &Request { prompt: "x".repeat(150), ..Default::default() }, "x").unwrap();
    assert!((p.seconds - 10.0).abs() < 1e-9);
    assert_eq!(p.estimate_usd, Some(0.045));
    // No tracks: an Overlay track is created with the placeholder.
    let empty = Document::new("q", "Q", 1);
    let p = plan(&empty, Kind::Image, &Request { prompt: "x".into(), ..Default::default() }, "x").unwrap();
    assert_eq!(p.ops.len(), 2);
    assert!(apply_batch(&empty, &p.ops).is_ok());
}
