//! Generation tools against a local stand-in for the fal.ai queue (the same request and reply shapes as the
//! stand-in in `crates/ove-generate/tests`): quotes spend nothing, only a person's approval spends, quotes expire and
//! bind to their request, the session cap holds, and the placeholder fills and can be undone by the agent.
//! No real keys and no network: the provider address points at the stand-in on this machine.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use ove_core::ops::Op;
use ove_core::revision::{Actor, ActorKind, Source};
use ove_generate::Provider;
use ove_generate::keys::{KeyStore, MemoryKeyStore};
use ove_store::project::{Mode, Project};
use ove_tools::session::{QUOTE_TTL_MS, set_spend_cap_usd};
use ove_tools::{AppCtx, AppHost, BundleCtx, Fail, Receipt, Session, Surface, ToolCtx};
use serde_json::{Value, json};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-stand-in-image-bytes";

/// Bodies of the submissions the stand-in received (each one would cost money at the real provider).
static SUBMITTED: LazyLock<Mutex<Vec<String>>> = LazyLock::new(Mutex::default);

/// Starts the stand-in once and points the providers at it.
static STAND_IN: LazyLock<String> = LazyLock::new(|| {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let b2 = base.clone();
    std::thread::spawn(move || {
        for (n, c) in l.incoming().flatten().enumerate() {
            let base = b2.clone();
            std::thread::spawn(move || {
                let mut r = BufReader::new(c.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let mut parts = line.split_whitespace();
                let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
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
                let (ctype, out): (&str, Vec<u8>) = match (method.as_str(), path.as_str()) {
                    ("POST", p) if p.starts_with("/fal-ai/") => {
                        SUBMITTED.lock().unwrap().push(body);
                        let id = format!("r{n}");
                        let v = json!({"request_id": id, "status_url": format!("{base}/fal-ai/x/requests/{id}/status"),
                            "response_url": format!("{base}/fal-ai/x/requests/{id}")});
                        ("application/json", v.to_string().into_bytes())
                    }
                    ("GET", p) if p.ends_with("/status") => ("application/json", br#"{"status":"COMPLETED"}"#.to_vec()),
                    ("GET", "/files/out.png") => ("image/png", PNG.to_vec()),
                    ("GET", p) if p.starts_with("/fal-ai/x/requests/") => {
                        let v = json!({"images": [{"url": format!("{base}/files/out.png"), "width": 64, "height": 36, "content_type": "image/png"}], "seed": 1});
                        ("application/json", v.to_string().into_bytes())
                    }
                    _ => ("application/json", b"{}".to_vec()),
                };
                let mut w = c;
                let _ = write!(
                    w,
                    "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    out.len()
                );
                let _ = w.write_all(&out);
            });
        }
    });
    // SAFETY: set once, before any provider call of this test binary reads them; nothing else writes them.
    unsafe {
        std::env::set_var("OVE_FAL_QUEUE_URL", &base);
        std::env::set_var("OVE_GENERATE_POLL_MS", "10");
    }
    base
});

/// Submissions whose body mentions `prompt`.
fn submitted(prompt: &str) -> usize {
    SUBMITTED.lock().unwrap().iter().filter(|b| b.contains(prompt)).count()
}

fn clock() -> Arc<dyn ove_core::Clock> {
    Arc::new(ove_core::SystemClock)
}

struct Manual(AtomicI64);

impl ove_core::Clock for Manual {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    lib: PathBuf,
    bundle: PathBuf,
    config: PathBuf,
    keys: Arc<MemoryKeyStore>,
}

fn fixture() -> Fixture {
    LazyLock::force(&STAND_IN);
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, clock()).unwrap();
    let mut p = library.create("Demo").unwrap();
    let ops: Vec<Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "t_overlay", "name": "Overlay", "order": 0, "type": "video"}},
        {"op": "addTrack", "track": {"id": "t_main", "name": "Main", "order": 1, "type": "video"}},
        {"op": "addItem", "item": {"id": "a", "trackId": "t_main", "type": "text", "startTime": 0, "endTime": 2, "name": "A", "textContent": "a"}}
    ]))
    .unwrap();
    p.apply(ops, Source::Gui, None).unwrap();
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    let keys = Arc::new(MemoryKeyStore::default());
    keys.set(Provider::Fal, "standin-fal-key-0123456789abcdef").unwrap();
    let config = d.path().join("config");
    Fixture { lib, bundle, config, keys, _dir: d }
}

/// An MCP agent session (headless) over the fixture, with the given way of asking the person.
fn agent(f: &Fixture, clock: Arc<dyn ove_core::Clock>) -> BundleCtx {
    let mut c = BundleCtx::open(Some(&f.lib), Surface::Mcp, Source::Mcp).unwrap();
    c.actor = Some(Actor {
        kind: ActorKind::Mcp,
        session: Some("s-gen".into()),
        turn: Some("t1".into()),
        client: Some("stand-in agent".into()),
    });
    c.session = Session::with(Some(f.config.clone()), clock, f.keys.clone());
    c
}

/// The generation worker, run at once (it has its own context, as `ove generate _run` does).
fn worker(f: &Fixture) -> ove_tools::bundle::Runner {
    let (lib, keys) = (f.lib.clone(), f.keys.clone());
    Arc::new(move |bundle: &Path, id: &str| {
        let mut w = BundleCtx::open(Some(&lib), Surface::Cli, Source::Cli)?;
        ove_tools::tools::generate::run_job(&mut w, bundle, id, keys.as_ref()).map(|_| ())
    })
}

/// A person who approves, keeping the cards they were shown.
fn approving(cards: &Arc<Mutex<Vec<Value>>>) -> ove_tools::bundle::Confirm {
    let cards = cards.clone();
    Box::new(move |card: &Value| {
        cards.lock().unwrap().push(card.clone());
        Ok(true)
    })
}

fn call(ctx: &mut dyn ToolCtx, name: &str, a: Value) -> Result<Receipt, Fail> {
    ove_tools::call(ctx, name, a.as_object().unwrap())
}

fn project(f: &Fixture) -> Project {
    Project::open(&f.bundle, Mode::ReadOnly, clock()).unwrap()
}

fn jobs(f: &Fixture) -> usize {
    ove_generate::job::list(&f.bundle).len()
}

#[test]
fn a_quote_spends_nothing_and_an_approved_request_fills_a_placeholder_the_agent_can_undo() {
    let f = fixture();
    let cards = Arc::new(Mutex::new(Vec::new()));
    let mut ctx = agent(&f, clock());
    ctx.confirm = Some(approving(&cards));
    ctx.runner = Some(worker(&f));
    let rev0 = project(&f).rev();
    let req = json!({"project": "Demo", "prompt": "a lighthouse at dawn (fill test)", "aspect": "16:9"});

    // Step 1: a quote. No job, no revision, nothing sent to the provider, nobody asked.
    let q = call(&mut ctx, "generate_image", req.clone()).unwrap();
    let quote = q.json["quoteId"].as_str().unwrap().to_string();
    assert_eq!(q.json["estimateUsd"], 0.025);
    assert_eq!(q.json["spent"], false);
    assert_eq!(q.json["plan"]["model"], "fal:flux-dev");
    assert_eq!((project(&f).rev(), jobs(&f), submitted("fill test")), (rev0, 0, 0));
    assert!(cards.lock().unwrap().is_empty());

    // Step 2: the same request with the quote; the person approves the card.
    let mut confirmed = req.clone();
    confirmed["confirm"] = quote.clone().into();
    let r = call(&mut ctx, "generate_image", confirmed.clone()).unwrap();
    let card = cards.lock().unwrap()[0].clone();
    assert_eq!(card["quoteId"], quote.as_str());
    assert_eq!(card["estimateUsd"], 0.025);
    assert_eq!(card["prompt"], "a lighthouse at dawn (fill test)");
    assert_eq!(card["project"]["name"], "Demo");
    assert_eq!(card["actor"]["client"], "stand-in agent");
    let item = r.json["itemId"].as_str().unwrap().to_string();
    let line = r.json["line"].as_str().unwrap();
    assert!(line.contains(&format!("id=\"{item}\"")) && line.contains("pending"), "{line}");
    assert_eq!(r.json["spentUsd"], 0.025);
    assert_eq!(submitted("fill test"), 1, "one submission after approval");

    // The worker ran: the placeholder is filled in place.
    let p = project(&f);
    let it = p.doc().item(&item).unwrap();
    assert!(!it.is_pending(), "filled");
    let media = it.fields["mediaItemId"].as_str().unwrap().to_string();
    assert!(p.doc().project.media.contains_key(&media));
    let job = ove_generate::job::load(&f.bundle, r.json["jobId"].as_str().unwrap()).unwrap();
    assert_eq!(job.status, ove_generate::Status::Succeeded);
    // Placeholder and fill are both the agent's revisions.
    let recs = p.records().unwrap();
    let mine: Vec<_> =
        recs.iter().filter(|x| x.actor.as_ref().and_then(|a| a.session.as_deref()) == Some("s-gen")).collect();
    assert_eq!(mine.len(), 2, "placeholder and fill");
    drop(p);

    // The same quote cannot be spent twice.
    assert_eq!(call(&mut ctx, "generate_image", confirmed).unwrap_err().code, "quote.used");

    // The agent undoes its own changes: the clip is gone, the person's item stays.
    let u = call(&mut ctx, "undo_my_changes", json!({"project": "Demo"})).unwrap();
    assert_eq!(u.json["undoneRevs"].as_array().unwrap().len(), 2);
    let p = project(&f);
    assert!(p.doc().item(&item).is_none());
    assert!(p.doc().item("a").is_some());

    // Status of the finished job.
    let s = call(&mut ctx, "generation_status", json!({"id": job.id})).unwrap();
    assert_eq!(s.json["status"], "succeeded");
}

#[test]
fn the_placeholder_is_one_revision_the_agent_can_undo_before_it_fills() {
    let f = fixture();
    let cards = Arc::new(Mutex::new(Vec::new()));
    let started: Arc<Mutex<Vec<String>>> = Arc::default();
    let mut ctx = agent(&f, clock());
    ctx.confirm = Some(approving(&cards));
    let s2 = started.clone();
    ctx.runner = Some(Arc::new(move |_: &Path, id: &str| {
        s2.lock().unwrap().push(id.to_string());
        Ok(())
    }));
    let req = json!({"project": "Demo", "prompt": "a red kite (undo test)"});
    let q = call(&mut ctx, "generate_image", req.clone()).unwrap();
    let mut c = req.clone();
    c["confirm"] = q.json["quoteId"].clone();
    let before = project(&f).rev();
    let r = call(&mut ctx, "generate_image", c).unwrap();
    assert_eq!(started.lock().unwrap().as_slice(), [r.json["jobId"].as_str().unwrap()]);
    let p = project(&f);
    assert_eq!(p.records().unwrap().iter().filter(|x| x.rev > before).count(), 1, "one revision");
    let item = r.json["itemId"].as_str().unwrap();
    assert!(p.doc().item(item).unwrap().is_pending());
    drop(p);
    let u = call(&mut ctx, "undo_timeline", json!({"project": "Demo"})).unwrap();
    assert_eq!(u.json["changed"], true);
    assert!(project(&f).doc().item(item).is_none());
    assert_eq!(submitted("undo test"), 0, "the worker never ran");
}

#[test]
fn a_hostile_agent_cannot_spend_without_a_person() {
    let f = fixture();
    // Headless, no app, a client without confirmation prompts: nobody can be asked.
    let mut ctx = agent(&f, clock());
    let ran: Arc<Mutex<Vec<String>>> = Arc::default();
    let r2 = ran.clone();
    ctx.runner = Some(Arc::new(move |_: &Path, id: &str| {
        r2.lock().unwrap().push(id.to_string());
        Ok(())
    }));
    let rev0 = project(&f).rev();
    let req = json!({"project": "Demo", "prompt": "a hostile request (hostile test)"});

    // 1. It gets a quote and confirms it itself.
    let q = call(&mut ctx, "generate_image", req.clone()).unwrap();
    let quote = q.json["quoteId"].as_str().unwrap().to_string();
    let mut confirmed = req.clone();
    confirmed["confirm"] = quote.clone().into();
    let e = call(&mut ctx, "generate_image", confirmed.clone()).unwrap_err();
    assert_eq!(e.code, "confirm.required");
    // 2. It claims the approval in arguments of its own.
    for extra in [json!({"approved": true}), json!({"yes": true}), json!({"confirmedBy": "person"})] {
        let mut a = confirmed.clone();
        a.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        assert_eq!(call(&mut ctx, "generate_image", a).unwrap_err().code, "argument.unknown");
    }
    // 3. It calls the app's confirmation as a tool.
    for name in ["confirm_spend", "approve_quote"] {
        let e = call(&mut ctx, name, json!({"quoteId": quote})).unwrap_err();
        assert_eq!(e.code, "tool.unknown", "{name}");
    }
    // 4. It makes up a quote id, or reuses one for another request.
    let mut forged = req.clone();
    forged["confirm"] = "q_0_1".into();
    assert_eq!(call(&mut ctx, "generate_image", forged).unwrap_err().code, "quote.unknown");
    let mut other = json!({"project": "Demo", "prompt": "something dearer (hostile test)"});
    other["confirm"] = quote.clone().into();
    assert_eq!(call(&mut ctx, "generate_image", other).unwrap_err().code, "quote.mismatch");
    // 5. A person who declines.
    ctx.confirm = Some(Box::new(|_: &Value| Ok(false)));
    assert_eq!(call(&mut ctx, "generate_image", confirmed).unwrap_err().code, "confirm.declined");

    // Nothing happened anywhere.
    assert_eq!(project(&f).rev(), rev0);
    assert_eq!(jobs(&f), 0);
    assert!(ran.lock().unwrap().is_empty());
    assert_eq!(submitted("hostile test"), 0);
    assert_eq!(ctx.session.spent_usd(), 0.0);
}

#[test]
fn an_expired_quote_is_refused_and_nobody_is_asked() {
    let f = fixture();
    let manual = Arc::new(Manual(AtomicI64::new(ove_tools::fail::now_ms())));
    let cards = Arc::new(Mutex::new(Vec::new()));
    let mut ctx = agent(&f, manual.clone());
    ctx.confirm = Some(approving(&cards));
    ctx.runner = Some(worker(&f));
    let req = json!({"project": "Demo", "prompt": "a slow decision (expiry test)"});
    let q = call(&mut ctx, "generate_image", req.clone()).unwrap();
    assert_eq!(q.json["expiresAt"].as_i64().unwrap() - manual.0.load(Ordering::SeqCst), QUOTE_TTL_MS);
    manual.0.fetch_add(QUOTE_TTL_MS, Ordering::SeqCst);
    let mut c = req.clone();
    c["confirm"] = q.json["quoteId"].clone();
    assert_eq!(call(&mut ctx, "generate_image", c).unwrap_err().code, "quote.expired");
    assert!(cards.lock().unwrap().is_empty());
    assert_eq!((jobs(&f), submitted("expiry test")), (0, 0));

    // A person who takes longer than the quote's life: the approval comes too late.
    let q = call(&mut ctx, "generate_image", req.clone()).unwrap();
    let m2 = manual.clone();
    ctx.confirm = Some(Box::new(move |_: &Value| {
        m2.0.fetch_add(QUOTE_TTL_MS + 1, Ordering::SeqCst);
        Ok(true)
    }));
    let mut c = req.clone();
    c["confirm"] = q.json["quoteId"].clone();
    assert_eq!(call(&mut ctx, "generate_image", c).unwrap_err().code, "quote.expired");
    assert_eq!((jobs(&f), submitted("expiry test")), (0, 0));
}

#[test]
fn the_session_cap_refuses_spending_above_it() {
    let f = fixture();
    set_spend_cap_usd(&f.config, 0.04).unwrap();
    let cards = Arc::new(Mutex::new(Vec::new()));
    let mut ctx = agent(&f, clock());
    ctx.confirm = Some(approving(&cards));
    ctx.runner = Some(worker(&f));
    let one = |ctx: &mut BundleCtx, prompt: &str| -> Result<Receipt, Fail> {
        let req = json!({"project": "Demo", "prompt": prompt});
        let q = call(ctx, "generate_image", req.clone())?;
        let mut c = req;
        c["confirm"] = q.json["quoteId"].clone();
        call(ctx, "generate_image", c)
    };
    one(&mut ctx, "first (cap test)").unwrap();
    // 0.025 + 0.025 > 0.04: refused at the quote, nothing spent.
    let e = one(&mut ctx, "second (cap test)").unwrap_err();
    assert_eq!(e.code, "spend.cap");
    assert_eq!(e.details["capUsd"], 0.04);
    assert_eq!(submitted("second (cap test)"), 0);
    // A quote made under a higher cap is refused when the cap is lowered before it is confirmed.
    set_spend_cap_usd(&f.config, 1.0).unwrap();
    let req = json!({"project": "Demo", "prompt": "third (cap test)"});
    let q = call(&mut ctx, "generate_image", req.clone()).unwrap();
    set_spend_cap_usd(&f.config, 0.03).unwrap();
    let mut c = req;
    c["confirm"] = q.json["quoteId"].clone();
    assert_eq!(call(&mut ctx, "generate_image", c).unwrap_err().code, "spend.cap");
    assert_eq!(cards.lock().unwrap().len(), 1, "only the first request reached the person");
    assert_eq!(ctx.session.spent_usd(), 0.025);
}

#[test]
fn list_models_shows_prices_keys_and_the_cap() {
    let f = fixture();
    set_spend_cap_usd(&f.config, 2.5).unwrap();
    let mut ctx = agent(&f, clock());
    let r = call(&mut ctx, "list_models", json!({"kind": "image"})).unwrap();
    let models = r.json["models"].as_array().unwrap();
    assert!(models.iter().all(|m| m["kind"] == "image"));
    let flux = models.iter().find(|m| m["id"] == "fal:flux-dev").unwrap();
    assert_eq!((flux["hasKey"].as_bool(), flux["priceText"].as_str()), (Some(true), Some("$0.025/image")));
    assert_eq!(r.json["capUsd"], 2.5);
    assert_eq!(call(&mut ctx, "list_models", json!({"kind": "hologram"})).unwrap_err().code, "argument.invalid");
}

#[test]
fn cancel_and_status_find_jobs_by_id() {
    let f = fixture();
    let cards = Arc::new(Mutex::new(Vec::new()));
    let mut ctx = agent(&f, clock());
    ctx.confirm = Some(approving(&cards));
    ctx.runner = Some(Arc::new(|_: &Path, _: &str| Ok(())));
    let req = json!({"project": "Demo", "prompt": "never runs (cancel test)"});
    let q = call(&mut ctx, "generate_image", req.clone()).unwrap();
    let mut c = req;
    c["confirm"] = q.json["quoteId"].clone();
    let id = call(&mut ctx, "generate_image", c).unwrap().json["jobId"].as_str().unwrap().to_string();
    let s = call(&mut ctx, "generation_status", json!({"id": id})).unwrap();
    assert_eq!(s.json["status"], "queued");
    let c = call(&mut ctx, "cancel_generation", json!({"id": id})).unwrap();
    assert_eq!(c.json["cancelRequested"], true);
    assert!(ove_generate::job::cancel_requested(&f.bundle, &id));
    assert_eq!(call(&mut ctx, "generation_status", json!({"id": "g_nope"})).unwrap_err().code, "job.notFound");
    assert_eq!(call(&mut ctx, "generation_status", json!({"id": "../x"})).unwrap_err().code, "argument.invalid");
}

/// The app as the in-app assistant sees it: the open project, a chat session, and the question card.
struct InApp {
    project: Project,
    session: Session,
    answer: bool,
    asked: Vec<Value>,
    started: Vec<String>,
}

impl AppHost for InApp {
    fn with_open(&mut self, _: bool, f: &mut dyn FnMut(&mut Project) -> Result<(), Fail>) -> Result<(), Fail> {
        f(&mut self.project)
    }
    fn session(&self) -> Option<Session> {
        Some(self.session.clone())
    }
    fn ask_spend(&mut self, card: &Value) -> Result<bool, Fail> {
        self.asked.push(card.clone());
        Ok(self.answer)
    }
    fn start_generation(&mut self, _: &Path, job: &str) -> Result<(), Fail> {
        self.started.push(job.to_string());
        Ok(())
    }
}

#[test]
fn in_app_the_question_card_decides() {
    let f = fixture();
    let project = Project::open(&f.bundle, Mode::Write, clock()).unwrap();
    let session = Session::with(Some(f.config.clone()), clock(), f.keys.clone());
    let mut host = InApp { project, session, answer: false, asked: vec![], started: vec![] };
    let req = json!({"prompt": "in the app (in-app test)"});
    let quote = {
        let mut ctx = AppCtx::assistant_in(&mut host, "chat-1", "turn-1");
        call(&mut ctx, "generate_image", req.clone()).unwrap().json["quoteId"].as_str().unwrap().to_string()
    };
    let mut c = req.clone();
    c["confirm"] = quote.into();
    {
        let mut ctx = AppCtx::assistant_in(&mut host, "chat-1", "turn-1");
        assert_eq!(call(&mut ctx, "generate_image", c.clone()).unwrap_err().code, "confirm.declined");
    }
    assert!(host.started.is_empty());
    // A new quote (a declined one stays unused, but ask again from the start), approved this time.
    host.answer = true;
    let quote = {
        let mut ctx = AppCtx::assistant_in(&mut host, "chat-1", "turn-1");
        call(&mut ctx, "generate_image", req.clone()).unwrap().json["quoteId"].as_str().unwrap().to_string()
    };
    c["confirm"] = quote.into();
    let r = {
        let mut ctx = AppCtx::assistant_in(&mut host, "chat-1", "turn-1");
        call(&mut ctx, "generate_image", c).unwrap()
    };
    assert_eq!(host.asked.len(), 2);
    assert_eq!(host.started.as_slice(), [r.json["jobId"].as_str().unwrap()]);
    let item = r.json["itemId"].as_str().unwrap();
    let rec = host.project.latest_record().unwrap().unwrap();
    let actor = rec.actor.unwrap();
    assert_eq!((actor.kind, actor.session.as_deref()), (ActorKind::Assistant, Some("chat-1")));
    assert!(host.project.doc().item(item).unwrap().is_pending());
    // The job remembers who asked, so the fill is theirs too.
    let job = ove_generate::job::load(&f.bundle, r.json["jobId"].as_str().unwrap()).unwrap();
    assert_eq!(job.actor.unwrap().session.as_deref(), Some("chat-1"));
}

#[test]
fn generation_is_unavailable_without_a_session() {
    let f = fixture();
    let mut project = Project::open(&f.bundle, Mode::Write, clock()).unwrap();
    let mut host = ove_tools::ProjectHost { project: &mut project };
    let mut ctx = AppCtx::assistant(&mut host);
    let e = call(&mut ctx, "generate_image", json!({"prompt": "x"})).unwrap_err();
    assert_eq!(e.code, "tool.unavailable");
}
