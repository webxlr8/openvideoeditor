//! The model worker: a child process (`ove model-worker`) that runs heavy models so the editor keeps its memory budget
//! and survives a model crash.
//!
//! Protocol on stdin/stdout, one frame per message: a 4-byte little-endian header length, the header as JSON, then the
//! raw little-endian tensor bytes the header describes (no base64). The worker loads each model on its first job,
//! serves one job at a time, and exits after [`DEFAULT_IDLE`] without work. [`WorkerClient`] starts it on demand,
//! fails the job when it dies, and starts a new one for the next job.

use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{ModelError, Result};
use crate::infer::{Backend, DType, Session, Tensor, builtin};

/// The worker exits after this long without a job.
pub const DEFAULT_IDLE: Duration = Duration::from_secs(120);
/// Largest header accepted (the tensors travel after it).
const MAX_HEADER: u32 = 1 << 20;
/// Largest tensor payload accepted in one frame.
const MAX_PAYLOAD: u64 = 4 << 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TensorHeader {
    name: String,
    dtype: DType,
    shape: Vec<usize>,
    bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
enum Request {
    /// Runs a model file (or a `builtin:` model) on the tensors that follow.
    Run {
        model: String,
        inputs: Vec<TensorHeader>,
    },
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(default)]
    outputs: Vec<TensorHeader>,
}

fn proto(e: impl std::fmt::Display) -> ModelError {
    ModelError::Inference(format!("model worker protocol: {e}"))
}

fn write_frame(w: &mut impl Write, header: &impl Serialize, tensors: &[Tensor]) -> Result<()> {
    let h = serde_json::to_vec(header).map_err(proto)?;
    w.write_all(&(h.len() as u32).to_le_bytes()).map_err(proto)?;
    w.write_all(&h).map_err(proto)?;
    for t in tensors {
        w.write_all(crate::infer::data_bytes(&t.data)).map_err(proto)?;
    }
    w.flush().map_err(proto)
}

fn headers(tensors: &[Tensor]) -> Vec<TensorHeader> {
    tensors
        .iter()
        .map(|t| TensorHeader {
            name: t.name.clone(),
            dtype: t.data.dtype(),
            shape: t.shape.clone(),
            bytes: (t.data.len() * if t.data.dtype() == DType::F32 { 4 } else { 8 }) as u64,
        })
        .collect()
}

/// Reads one frame header. `Ok(None)` at a clean end of stream.
fn read_header<T: for<'de> Deserialize<'de>>(r: &mut impl Read) -> Result<Option<T>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(proto(e)),
    }
    let len = u32::from_le_bytes(len);
    if len > MAX_HEADER {
        return Err(proto(format!("header of {len} bytes")));
    }
    let mut h = vec![0u8; len as usize];
    r.read_exact(&mut h).map_err(proto)?;
    serde_json::from_slice(&h).map(Some).map_err(proto)
}

fn read_tensors(r: &mut impl Read, hs: &[TensorHeader]) -> Result<Vec<Tensor>> {
    let total: u64 = hs.iter().map(|h| h.bytes).sum();
    if total > MAX_PAYLOAD {
        return Err(proto(format!("payload of {total} bytes")));
    }
    hs.iter()
        .map(|h| {
            let data = crate::infer::read_data(r, h.dtype, h.bytes as usize)?;
            let t = Tensor { name: h.name.clone(), shape: h.shape.clone(), data };
            t.check()?;
            Ok(t)
        })
        .collect()
}

/// Serves jobs from `input`, answering on `output`, until the input closes or nothing arrives for `idle`.
/// Models are loaded by `backend` on first use and kept; `builtin:` models need no backend.
pub fn serve(
    input: impl Read + Send + 'static,
    output: impl Write,
    idle: Duration,
    backend: Option<&dyn Backend>,
) -> Result<()> {
    let (tx, rx) = mpsc::sync_channel::<Result<(Request, Vec<Tensor>)>>(1);
    std::thread::spawn(move || {
        let mut r = BufReader::with_capacity(1 << 20, input);
        loop {
            let msg = match read_header::<Request>(&mut r) {
                Ok(None) => break,
                Ok(Some(req)) => {
                    let tensors = match &req {
                        Request::Run { inputs, .. } => read_tensors(&mut r, inputs),
                        Request::Ping => Ok(Vec::new()),
                    };
                    tensors.map(|t| (req, t))
                }
                Err(e) => Err(e),
            };
            let stop = msg.is_err();
            if tx.send(msg).is_err() || stop {
                break;
            }
        }
    });
    let mut out = BufWriter::with_capacity(1 << 20, output);
    let mut sessions: HashMap<String, Box<dyn Session>> = HashMap::new();
    loop {
        let (req, inputs) = match rx.recv_timeout(idle) {
            Ok(msg) => msg?,
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return Ok(()),
        };
        let result = match req {
            Request::Ping => Ok(Vec::new()),
            Request::Run { model, .. } => run_job(&mut sessions, backend, &model, &inputs),
        };
        match result {
            Ok(outputs) => {
                write_frame(&mut out, &Response { ok: true, error: None, outputs: headers(&outputs) }, &outputs)?
            }
            Err(e) => {
                write_frame(&mut out, &Response { ok: false, error: Some(e.to_string()), outputs: Vec::new() }, &[])?
            }
        }
    }
}

fn run_job(
    sessions: &mut HashMap<String, Box<dyn Session>>,
    backend: Option<&dyn Backend>,
    model: &str,
    inputs: &[Tensor],
) -> Result<Vec<Tensor>> {
    if !sessions.contains_key(model) {
        let session = match builtin(model) {
            Some(s) => s,
            None => backend
                .ok_or_else(|| {
                    ModelError::RuntimeMissing(
                        "runtime not installed: no inference runtime is available in the model worker".into(),
                    )
                })?
                .load(Path::new(model))?,
        };
        sessions.insert(model.to_string(), session);
    }
    match sessions.get_mut(model) {
        Some(s) => s.run(inputs),
        None => Err(ModelError::Inference(format!("model {model} is not loaded"))),
    }
}

struct Running {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

/// Starts the worker on demand and sends it jobs, one at a time.
pub struct WorkerClient {
    program: PathBuf,
    args: Vec<String>,
    envs: Vec<(String, String)>,
    running: Mutex<Option<Running>>,
    pid: AtomicU32,
}

impl std::fmt::Debug for WorkerClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerClient").field("program", &self.program).field("args", &self.args).finish()
    }
}

impl WorkerClient {
    /// A client that runs `program args…` as the worker (normally `ove model-worker`).
    pub fn new(program: impl Into<PathBuf>, args: &[&str]) -> WorkerClient {
        WorkerClient {
            program: program.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            envs: Vec::new(),
            running: Mutex::new(None),
            pid: AtomicU32::new(0),
        }
    }

    /// An environment variable for the worker (for example where the runtime library is).
    pub fn env(mut self, key: &str, value: &str) -> WorkerClient {
        self.envs.push((key.into(), value.into()));
        self
    }

    fn lock(&self) -> MutexGuard<'_, Option<Running>> {
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The worker's process id while one is (or was last) running; 0 before the first job.
    pub fn pid(&self) -> u32 {
        self.pid.load(Ordering::SeqCst)
    }

    /// True while a worker process is alive.
    pub fn is_running(&self) -> bool {
        let mut g = self.lock();
        match g.as_mut() {
            Some(r) => matches!(r.child.try_wait(), Ok(None)),
            None => false,
        }
    }

    fn spawn(&self) -> Result<Running> {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
        for (k, v) in &self.envs {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().map_err(|e| {
            ModelError::Inference(format!("could not start the model worker {}: {e}", self.program.display()))
        })?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            return Err(ModelError::Inference("the model worker has no pipes".into()));
        };
        self.pid.store(child.id(), Ordering::SeqCst);
        Ok(Running {
            child,
            stdin: BufWriter::with_capacity(1 << 20, stdin),
            stdout: BufReader::with_capacity(1 << 20, stdout),
        })
    }

    /// Runs `model` (a model file path, or a `builtin:` model) on `inputs` in the worker. If the worker dies during
    /// the job, the job fails and the next job starts a new worker.
    pub fn run(&self, model: &str, inputs: &[Tensor]) -> Result<Vec<Tensor>> {
        for t in inputs {
            t.check()?;
        }
        let mut g = self.lock();
        // A worker that exited after being idle (or crashed earlier) is replaced.
        if let Some(r) = g.as_mut()
            && !matches!(r.child.try_wait(), Ok(None))
        {
            *g = None;
        }
        if g.is_none() {
            *g = Some(self.spawn()?);
        }
        let Some(r) = g.as_mut() else { return Err(ModelError::Inference("the model worker did not start".into())) };
        let result = exchange(r, model, inputs);
        if let Err(e) = &result {
            // Any protocol failure means the worker is gone or unusable: stop it so the next job starts afresh.
            if matches!(e, ModelError::Inference(_)) {
                let _ = r.child.kill();
                let _ = r.child.wait();
                *g = None;
                return Err(ModelError::Inference(format!("the model worker stopped during the job ({e})")));
            }
        }
        result
    }

    /// Stops the worker now (it would also exit by itself when idle).
    pub fn shutdown(&self) {
        if let Some(mut r) = self.lock().take() {
            drop(r.stdin);
            let _ = r.child.wait();
        }
    }
}

impl Drop for WorkerClient {
    fn drop(&mut self) {
        if let Some(mut r) = self.lock().take() {
            let _ = r.child.kill();
            let _ = r.child.wait();
        }
    }
}

fn exchange(r: &mut Running, model: &str, inputs: &[Tensor]) -> Result<Vec<Tensor>> {
    let req = Request::Run { model: model.to_string(), inputs: headers(inputs) };
    write_frame(&mut r.stdin, &req, inputs)?;
    let resp: Response = read_header(&mut r.stdout)?.ok_or_else(|| proto("the worker closed its output"))?;
    let outputs = read_tensors(&mut r.stdout, &resp.outputs)?;
    if resp.ok {
        Ok(outputs)
    } else {
        // The job failed inside a healthy worker: report it, keep the worker.
        Err(ModelError::JobFailed(resp.error.unwrap_or_else(|| "the model failed".into())))
    }
}

/// A session whose model runs in the worker.
pub struct WorkerSession<'a> {
    pub client: &'a WorkerClient,
    pub model: String,
}

impl Session for WorkerSession<'_> {
    fn run(&mut self, inputs: &[Tensor]) -> Result<Vec<Tensor>> {
        self.client.run(&self.model, inputs)
    }
}
