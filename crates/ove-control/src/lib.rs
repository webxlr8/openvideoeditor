//! Control link between the command line (and the MCP server) and the running app (roadmap change 8).
//!
//! The app listens on a loopback TCP port and publishes `{pid, port, token, version}` in `<library>/.ove-control.json`
//! (owner-only permissions on Unix). A client sends one JSON line `{"token", "method", "params"}` and reads one JSON
//! line back: `{"ok": true, "result": …}` or `{"ok": false, "error": {"code", "message"}}`. Loopback plus the token
//! keeps other users and web pages out; the token changes every launch.
//!
//! Methods the app answers (see `docs/cli/app.md`): `context`, `open`, `seek`, `play`, `pause`, `apply`, `restore`.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// File name of the endpoint record in the library folder.
pub const ENDPOINT_FILE: &str = ".ove-control.json";

/// Where the running app can be reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub pid: u32,
    pub port: u16,
    pub token: String,
    pub version: String,
}

pub fn endpoint_path(library: &Path) -> PathBuf {
    library.join(ENDPOINT_FILE)
}

/// The published endpoint, if any (it may be stale: `call` reports `NotRunning` when nothing answers).
pub fn read_endpoint(library: &Path) -> Option<Endpoint> {
    let text = std::fs::read_to_string(endpoint_path(library)).ok()?;
    serde_json::from_str(&text).ok()
}

/// A refusal or failure from the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct Refusal {
    pub code: String,
    pub message: String,
}

impl Refusal {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Refusal { code: code.into(), message: message.into() }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CallError {
    #[error("the app is not running")]
    NotRunning,
    #[error("{0}")]
    Refused(Refusal),
    #[error("control link: {0}")]
    Io(String),
}

impl CallError {
    pub fn code(&self) -> &str {
        match self {
            CallError::NotRunning => "app.notRunning",
            CallError::Refused(r) => &r.code,
            CallError::Io(_) => "app.unreachable",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Request {
    token: String,
    method: String,
    #[serde(default)]
    params: Value,
}

/// Calls a method on the app serving `library`.
pub fn call(library: &Path, method: &str, params: Value, timeout: Duration) -> Result<Value, CallError> {
    let ep = read_endpoint(library).ok_or(CallError::NotRunning)?;
    call_endpoint(&ep, method, params, timeout)
}

pub fn call_endpoint(ep: &Endpoint, method: &str, params: Value, timeout: Duration) -> Result<Value, CallError> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, ep.port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(800)).map_err(|_| CallError::NotRunning)?;
    s.set_read_timeout(Some(timeout)).ok();
    s.set_write_timeout(Some(timeout)).ok();
    let req = Request { token: ep.token.clone(), method: method.into(), params };
    let mut line = serde_json::to_string(&req).map_err(|e| CallError::Io(e.to_string()))?;
    line.push('\n');
    s.write_all(line.as_bytes()).map_err(|e| CallError::Io(e.to_string()))?;
    let mut reply = String::new();
    BufReader::new(&s).read_line(&mut reply).map_err(|e| CallError::Io(e.to_string()))?;
    if reply.trim().is_empty() {
        return Err(CallError::Io("the app closed the connection".into()));
    }
    let v: Value = serde_json::from_str(&reply).map_err(|e| CallError::Io(format!("bad reply: {e}")))?;
    if v.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    } else {
        let r: Refusal = serde_json::from_value(v.get("error").cloned().unwrap_or(Value::Null))
            .unwrap_or_else(|_| Refusal::new("app.error", "the app refused the request"));
        Err(CallError::Refused(r))
    }
}

/// Handles one request: method, params → result or refusal.
pub type Handler = Arc<dyn Fn(&str, Value) -> Result<Value, Refusal> + Send + Sync>;

/// A running server. Dropping it stops accepting and removes the endpoint file (if it is still ours).
pub struct Server {
    endpoint: Endpoint,
    file: PathBuf,
    stop: Arc<AtomicBool>,
}

impl Server {
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop.
        let _ = TcpStream::connect_timeout(
            &SocketAddr::from((Ipv4Addr::LOCALHOST, self.endpoint.port)),
            Duration::from_millis(200),
        );
        if read_endpoint(self.file.parent().unwrap_or(Path::new("."))).is_some_and(|e| e.token == self.endpoint.token) {
            let _ = std::fs::remove_file(&self.file);
        }
    }
}

fn new_token() -> String {
    let mut b = [0u8; 24];
    if getrandom::fill(&mut b).is_err() {
        // Extremely unlikely; fall back to time + pid entropy.
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        b[..16].copy_from_slice(&t.to_le_bytes());
        b[16..20].copy_from_slice(&std::process::id().to_le_bytes());
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let mut f = o.open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Starts serving `library` on a loopback port and publishes the endpoint. One thread accepts; each connection is
/// handled on its own thread (requests are small and rare).
pub fn serve(library: &Path, version: &str, handler: Handler) -> std::io::Result<Server> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    let endpoint = Endpoint { pid: std::process::id(), port, token: new_token(), version: version.into() };
    let file = endpoint_path(library);
    write_private(&file, &serde_json::to_string_pretty(&endpoint).unwrap_or_default())?;
    let stop = Arc::new(AtomicBool::new(false));
    let (stop2, token) = (stop.clone(), endpoint.token.clone());
    std::thread::Builder::new().name("control-accept".into()).spawn(move || {
        for conn in listener.incoming() {
            if stop2.load(Ordering::SeqCst) {
                break;
            }
            let Ok(conn) = conn else { continue };
            let (h, token) = (handler.clone(), token.clone());
            let _ = std::thread::Builder::new().name("control-conn".into()).spawn(move || handle(conn, &token, &h));
        }
    })?;
    tracing::info!(port, library = %library.display(), "control link listening");
    Ok(Server { endpoint, file, stop })
}

fn handle(conn: TcpStream, token: &str, handler: &Handler) {
    conn.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let mut line = String::new();
    if BufReader::new(&conn).take_line(&mut line).is_err() {
        return;
    }
    let reply = match serde_json::from_str::<Request>(&line) {
        Err(e) => json!({"ok": false, "error": {"code": "request.invalid", "message": e.to_string()}}),
        Ok(r) if !constant_time_eq(&r.token, token) => {
            json!({"ok": false, "error": {"code": "token.invalid", "message": "wrong control token"}})
        }
        Ok(r) => match handler(&r.method, r.params) {
            Ok(v) => json!({"ok": true, "result": v}),
            Err(e) => json!({"ok": false, "error": e}),
        },
    };
    let mut w = &conn;
    let _ = w.write_all(format!("{reply}\n").as_bytes());
    let _ = w.flush();
}

/// Reads one line of at most 4 MiB.
trait TakeLine {
    fn take_line(&mut self, out: &mut String) -> std::io::Result<usize>;
}

impl<R: BufRead> TakeLine for R {
    fn take_line(&mut self, out: &mut String) -> std::io::Result<usize> {
        use std::io::Read;
        self.take(4 << 20).read_line(out)
    }
}
