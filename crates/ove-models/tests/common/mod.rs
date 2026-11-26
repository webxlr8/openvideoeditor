//! A small HTTP/1.1 file server for download tests: serves registered bodies, honours `Range`, logs each request,
//! and can misbehave on purpose (corrupt bytes, cut the connection, ignore ranges, stall).
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ove_models::{Manifest, ModelFile, RuntimeKind};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Normal,
    /// Serves the body with one byte changed.
    Corrupt,
    /// The first response stops after this many bytes and closes; later requests are normal.
    CutFirstAfter(usize),
    /// Ignores `Range` and always sends the whole body with 200.
    IgnoreRange,
    /// Sends this many bytes, then stalls for a long time.
    StallAfter(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub path: String,
    pub range_start: Option<u64>,
}

#[derive(Default)]
struct State {
    routes: HashMap<String, (Vec<u8>, Mode)>,
    log: Vec<Request>,
    served: HashMap<String, usize>,
}

pub struct Server {
    pub base: String,
    state: Arc<Mutex<State>>,
}

impl Server {
    pub fn start() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(State::default()));
        let st = state.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let st = st.clone();
                std::thread::spawn(move || {
                    let _ = serve(stream, &st);
                });
            }
        });
        Server { base, state }
    }

    pub fn add(&self, path: &str, body: Vec<u8>, mode: Mode) {
        self.state.lock().unwrap().routes.insert(path.into(), (body, mode));
    }

    pub fn requests(&self) -> Vec<Request> {
        self.state.lock().unwrap().log.clone()
    }
}

fn serve(mut stream: TcpStream, state: &Mutex<State>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
    let mut range_start = None;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h == "\r\n" {
            break;
        }
        let lower = h.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("range: bytes=") {
            range_start = v.trim().trim_end_matches('-').parse::<u64>().ok();
        }
    }
    let (route, nth) = {
        let mut s = state.lock().unwrap();
        s.log.push(Request { path: path.clone(), range_start });
        let n = s.served.entry(path.clone()).or_insert(0);
        *n += 1;
        let nth = *n;
        (s.routes.get(&path).cloned(), nth)
    };
    let Some((mut body, mode)) = route else {
        stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
        return Ok(());
    };
    if mode == Mode::Corrupt {
        let mid = body.len() / 2;
        body[mid] ^= 0xff;
    }
    let total = body.len();
    let start = match (mode, range_start) {
        (Mode::IgnoreRange, _) | (_, None) => 0,
        (_, Some(s)) => s as usize,
    };
    if start >= total && range_start.is_some() && mode != Mode::IgnoreRange {
        let head = format!(
            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{total}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(head.as_bytes())?;
        return Ok(());
    }
    let head = if start > 0 {
        format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{}/{total}\r\nConnection: close\r\n\r\n",
            total - start,
            total - 1
        )
    } else {
        format!("HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n")
    };
    stream.write_all(head.as_bytes())?;
    let rest = &body[start..];
    match mode {
        Mode::CutFirstAfter(n) if nth == 1 => {
            stream.write_all(&rest[..n.min(rest.len())])?;
            stream.flush()?;
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        Mode::StallAfter(n) => {
            stream.write_all(&rest[..n.min(rest.len())])?;
            stream.flush()?;
            std::thread::sleep(Duration::from_secs(30));
        }
        _ => {
            for chunk in rest.chunks(64 * 1024) {
                stream.write_all(chunk)?;
            }
        }
    }
    Ok(())
}

/// Deterministic test bytes.
pub fn body(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed).wrapping_add((i >> 8) as u8)).collect()
}

pub fn sha(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}

pub const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

/// A manifest whose files are `bodies`, at pinned (unused) URLs; tests serve them through a mirror override.
pub fn manifest(id: &str, files: &[(&str, &[u8])]) -> Manifest {
    Manifest {
        id: id.into(),
        version: "1".into(),
        purpose: format!("test model {id}"),
        licence: "MIT".into(),
        files: files
            .iter()
            .map(|(name, b)| ModelFile {
                name: (*name).into(),
                url: format!("https://models.invalid/repo/resolve/{COMMIT}/{name}"),
                sha256: sha(b),
                bytes: b.len() as u64,
            })
            .collect(),
        runtime: RuntimeKind::Onnx,
        in_process: false,
    }
}
