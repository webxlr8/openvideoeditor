//! Spike bench: runs the exported Silero VAD and Beat This models on the fixtures written by `export_models.py`
//! and prints one JSON line per model: load time, parity with the reference outputs, real-time factor and peak
//! resident memory of this process.
//!
//! Usage: model-spike-bench <dir with exports and fixtures> <vad|beat> [threads] [onnxruntime library path]
//! Run one model per process so the peak memory belongs to that model alone.

use std::path::{Path, PathBuf};
use std::time::Instant;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const VAD_ROW: usize = 576;
const VAD_STATE: usize = 2 * 128;
const BEAT_FRAMES: usize = 1500;
const BEAT_BINS: usize = 128;

trait Runtime {
    fn name(&self) -> &'static str;
    fn load(&mut self, model: &Path, threads: usize) -> Res<()>;
    /// One VAD step: returns the probability and the new state.
    fn vad_step(&mut self, row: &[f32], state: &[f32]) -> Res<(f32, Vec<f32>)>;
    /// The beat model on one chunk: returns beat logits followed by downbeat logits.
    fn beat(&mut self, spect: &[f32]) -> Res<Vec<f32>>;
}

fn read_f32(path: &Path) -> Res<Vec<f32>> {
    let bytes = std::fs::read(path)?;
    Ok(bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
}

fn max_abs_diff(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "length mismatch");
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
}

fn main() -> Res<()> {
    let args: Vec<String> = std::env::args().collect();
    let usage = "usage: model-spike-bench <dir> <vad|beat> [threads] [dylib]";
    let dir = PathBuf::from(args.get(1).ok_or(usage)?);
    let model = args.get(2).ok_or(usage)?.clone();
    let threads: usize = args.get(3).map(|s| s.parse()).transpose()?.unwrap_or(1);
    let dylib = args.get(4).cloned();
    let mut rt = make_runtime(dylib)?;
    match model.as_str() {
        "vad" => vad(rt.as_mut(), &dir, threads),
        "beat" => beat(rt.as_mut(), &dir, threads),
        _ => Err(usage.into()),
    }
}

fn vad(rt: &mut dyn Runtime, dir: &Path, threads: usize) -> Res<()> {
    // Silero VAD: three fixtures, each 938 steps of 32 ms.
    let t = Instant::now();
    rt.load(&dir.join("silero_vad_16k.onnx"), threads)?;
    let load_ms = t.elapsed().as_secs_f64() * 1e3;
    let mut worst = 0f32;
    let (mut audio_s, mut run_s) = (0f64, 0f64);
    for fx in ["speech", "speech_noise", "music"] {
        let rows = read_f32(&dir.join(format!("vad_{fx}.in.f32")))?;
        let reference = read_f32(&dir.join(format!("vad_{fx}.ref.f32")))?;
        let mut state = vec![0f32; VAD_STATE];
        let mut probs = Vec::with_capacity(reference.len());
        let t = Instant::now();
        for row in rows.chunks_exact(VAD_ROW) {
            let (p, s) = rt.vad_step(row, &state)?;
            probs.push(p);
            state = s;
        }
        run_s += t.elapsed().as_secs_f64();
        audio_s += probs.len() as f64 * 0.032;
        worst = worst.max(max_abs_diff(&probs, &reference));
    }
    println!(
        "{{\"runtime\":\"{}\",\"model\":\"silero_vad_16k\",\"threads\":{threads},\"load_ms\":{load_ms:.1},\"max_abs_diff\":{worst:.3e},\"audio_s\":{audio_s:.1},\"run_s\":{run_s:.4},\"rtf\":{:.5},\"x_realtime\":{:.0},\"peak_rss_mb\":{:.1}}}",
        rt.name(),
        run_s / audio_s,
        audio_s / run_s,
        peak_rss_mb()
    );
    Ok(())
}

fn beat(rt: &mut dyn Runtime, dir: &Path, threads: usize) -> Res<()> {
    // Beat This small0: three fixtures of 1500 frames (30 s each).
    let t = Instant::now();
    rt.load(&dir.join("beat_this_small0.onnx"), threads)?;
    let load_ms = t.elapsed().as_secs_f64() * 1e3;
    let mut worst = 0f32;
    let (mut audio_s, mut run_s) = (0f64, 0f64);
    for fx in ["drums100", "drums128", "speech"] {
        let spect = read_f32(&dir.join(format!("beat_{fx}.in.f32")))?;
        assert_eq!(spect.len(), BEAT_FRAMES * BEAT_BINS);
        let reference = read_f32(&dir.join(format!("beat_{fx}.ref.f32")))?;
        let t = Instant::now();
        let got = rt.beat(&spect)?;
        run_s += t.elapsed().as_secs_f64();
        audio_s += 30.0;
        worst = worst.max(max_abs_diff(&got, &reference));
    }
    println!(
        "{{\"runtime\":\"{}\",\"model\":\"beat_this_small0\",\"threads\":{threads},\"load_ms\":{load_ms:.1},\"max_abs_diff\":{worst:.3e},\"audio_s\":{audio_s:.1},\"run_s\":{run_s:.4},\"rtf\":{:.5},\"x_realtime\":{:.0},\"peak_rss_mb\":{:.1}}}",
        rt.name(),
        run_s / audio_s,
        audio_s / run_s,
        peak_rss_mb()
    );
    Ok(())
}

#[cfg(feature = "ort")]
fn make_runtime(dylib: Option<String>) -> Res<Box<dyn Runtime>> {
    Ok(Box::new(ort_rt::OrtRuntime::new(dylib)?))
}

#[cfg(all(feature = "tract", not(feature = "ort")))]
fn make_runtime(_dylib: Option<String>) -> Res<Box<dyn Runtime>> {
    Ok(Box::new(tract_rt::TractRuntime::default()))
}

#[cfg(not(any(feature = "ort", feature = "tract")))]
fn make_runtime(_dylib: Option<String>) -> Res<Box<dyn Runtime>> {
    // Baseline build: only exists so the binary-size difference of each runtime can be measured.
    println!("{{\"runtime\":\"none\",\"peak_rss_mb\":{:.1}}}", peak_rss_mb());
    std::process::exit(0)
}

#[cfg(feature = "ort")]
mod ort_rt {
    use super::*;
    use ort::session::Session;
    use ort::value::Tensor;

    pub struct OrtRuntime {
        env: ort::environment::Environment,
        session: Option<Session>,
    }

    impl OrtRuntime {
        pub fn new(dylib: Option<String>) -> Res<Self> {
            let path = dylib.ok_or("the ort build needs the onnxruntime library path as the third argument")?;
            let env = ort::init_from(path)?.build()?;
            Ok(Self { env, session: None })
        }

        fn session(&mut self) -> Res<&mut Session> {
            self.session.as_mut().ok_or_else(|| "no model loaded".into())
        }
    }

    impl Runtime for OrtRuntime {
        fn name(&self) -> &'static str {
            "onnxruntime"
        }

        fn load(&mut self, model: &Path, threads: usize) -> Res<()> {
            self.session = Some(Session::builder(&self.env)?.with_intra_threads(threads)?.commit_from_file(model)?);
            Ok(())
        }

        fn vad_step(&mut self, row: &[f32], state: &[f32]) -> Res<(f32, Vec<f32>)> {
            let input = Tensor::from_array(([1usize, VAD_ROW], row.to_vec()))?;
            let st = Tensor::from_array(([2usize, 1, 128], state.to_vec()))?;
            let out = self.session()?.run(ort::inputs!["input" => input, "state" => st])?;
            let (_, p) = out["prob"].try_extract_tensor::<f32>()?;
            let (_, s) = out["state_out"].try_extract_tensor::<f32>()?;
            Ok((p[0], s.to_vec()))
        }

        fn beat(&mut self, spect: &[f32]) -> Res<Vec<f32>> {
            let input = Tensor::from_array(([1usize, BEAT_FRAMES, BEAT_BINS], spect.to_vec()))?;
            let out = self.session()?.run(ort::inputs!["spect" => input])?;
            let (_, b) = out["beat"].try_extract_tensor::<f32>()?;
            let (_, d) = out["downbeat"].try_extract_tensor::<f32>()?;
            Ok(b.iter().chain(d.iter()).copied().collect())
        }
    }
}

#[cfg(feature = "tract")]
mod tract_rt {
    use super::*;
    use tract_onnx::prelude::*;

    type Plan = std::sync::Arc<TypedSimplePlan>;

    #[derive(Default)]
    pub struct TractRuntime {
        plan: Option<Plan>,
    }

    impl TractRuntime {
        fn plan(&self) -> Res<&Plan> {
            self.plan.as_ref().ok_or_else(|| "no model loaded".into())
        }
    }

    impl super::Runtime for TractRuntime {
        fn name(&self) -> &'static str {
            "tract"
        }

        fn load(&mut self, model: &Path, _threads: usize) -> Res<()> {
            self.plan = Some(tract_onnx::onnx().model_for_path(model)?.into_optimized()?.into_runnable()?);
            Ok(())
        }

        fn vad_step(&mut self, row: &[f32], state: &[f32]) -> Res<(f32, Vec<f32>)> {
            let input = Tensor::from_shape(&[1, VAD_ROW], row)?;
            let st = Tensor::from_shape(&[2, 1, 128], state)?;
            let out = self.plan()?.run(tvec!(input.into(), st.into()))?;
            let p = out[0].try_as_plain_ram()?.as_slice::<f32>()?[0];
            Ok((p, out[1].try_as_plain_ram()?.as_slice::<f32>()?.to_vec()))
        }

        fn beat(&mut self, spect: &[f32]) -> Res<Vec<f32>> {
            let input = Tensor::from_shape(&[1, BEAT_FRAMES, BEAT_BINS], spect)?;
            let out = self.plan()?.run(tvec!(input.into()))?;
            let mut v = out[0].try_as_plain_ram()?.as_slice::<f32>()?.to_vec();
            v.extend_from_slice(out[1].try_as_plain_ram()?.as_slice::<f32>()?);
            Ok(v)
        }
    }
}

#[cfg(windows)]
fn peak_rss_mb() -> f64 {
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: plain FFI call with a correctly sized, zeroed out-structure for the current process.
    unsafe {
        let mut c: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) == 0 {
            return -1.0;
        }
        c.PeakWorkingSetSize as f64 / 1_048_576.0
    }
}

#[cfg(unix)]
fn peak_rss_mb() -> f64 {
    // SAFETY: getrusage fills the zeroed structure for this process.
    unsafe {
        let mut u: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut u);
        // macOS reports bytes; Linux reports kilobytes.
        if cfg!(target_os = "macos") { u.ru_maxrss as f64 / 1_048_576.0 } else { u.ru_maxrss as f64 / 1024.0 }
    }
}
