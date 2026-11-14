//! System audio output (cpal) feeding from an [`AudioRing`], following the default output device.
//!
//! The stream lives on its own thread (streams are not `Send` on every platform). That thread watches the default
//! output device every [`DEVICE_POLL`] and rebuilds the stream when the default changes or the stream reports an
//! error, so playback continues on the new device well within 1 s.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::audio::AudioRing;
use crate::clock::TimeSource;

/// How often the default output device is checked.
pub const DEVICE_POLL: Duration = Duration::from_millis(250);

/// Why audio output is unavailable (the player then runs on the system clock).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AudioUnavailable {
    #[error("no audio output device")]
    NoDevice,
    #[error("audio output could not start: {0}")]
    Failed(String),
}

/// Current output state, readable from the UI (diagnostics overlay, "audio unavailable" notice).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutputStatus {
    pub device: Option<String>,
    pub sample_rate: u32,
    pub channels: u16,
    /// Number of times the stream moved to another device.
    pub device_changes: u32,
    pub last_error: Option<String>,
}

pub struct AudioOutput {
    ring: AudioRing,
    status: Arc<Mutex<OutputStatus>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// Decide whether the stream must be rebuilt.
pub(crate) fn needs_rebuild<I: PartialEq>(current: Option<&I>, default_now: Option<&I>, stream_failed: bool) -> bool {
    stream_failed || current != default_now
}

impl AudioOutput {
    /// Open the default output device. Fails with [`AudioUnavailable`] when there is none.
    pub fn open_default(time: Arc<dyn TimeSource>) -> Result<AudioOutput, AudioUnavailable> {
        let ring = AudioRing::new(48_000, 2);
        let status = Arc::new(Mutex::new(OutputStatus::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel();
        let (r2, s2, st2) = (ring.clone(), status.clone(), stop.clone());
        let thread = std::thread::Builder::new()
            .name("ove-audio".into())
            .spawn(move || run(r2, s2, st2, time, ready_tx))
            .map_err(|e| AudioUnavailable::Failed(e.to_string()))?;
        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(AudioOutput { ring, status, stop, thread: Some(thread) }),
            Ok(Err(e)) => {
                stop.store(true, Ordering::SeqCst);
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                stop.store(true, Ordering::SeqCst);
                Err(AudioUnavailable::Failed("audio device did not respond".into()))
            }
        }
    }

    pub fn ring(&self) -> &AudioRing {
        &self.ring
    }

    pub fn status(&self) -> OutputStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

impl Drop for AudioOutput {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Active {
    _stream: cpal::Stream,
    id: Option<cpal::DeviceId>,
    failed: Arc<AtomicBool>,
}

fn build(
    host: &cpal::Host,
    ring: &AudioRing,
    time: &Arc<dyn TimeSource>,
    status: &Mutex<OutputStatus>,
) -> Result<Active, AudioUnavailable> {
    let device = host.default_output_device().ok_or(AudioUnavailable::NoDevice)?;
    let supported = device.default_output_config().map_err(|e| AudioUnavailable::Failed(e.to_string()))?;
    let config = supported.config();
    let channels = config.channels;
    let rate = config.sample_rate;
    if ring.format() != (rate, channels) {
        ring.set_format(rate, channels);
    }
    let failed = Arc::new(AtomicBool::new(false));
    let f2 = failed.clone();
    let err_cb = move |_e: cpal::Error| f2.store(true, Ordering::SeqCst);
    let mut scratch: Vec<f32> = Vec::new();
    let (ring2, time2) = (ring.clone(), time.clone());
    let mut fill = move |n: usize, info: &cpal::OutputCallbackInfo| -> Vec<f32> {
        let ts = info.timestamp();
        let latency = ts.playback.checked_duration_since(ts.callback).map(|d| d.as_micros() as i64).unwrap_or(0);
        scratch.resize(n, 0.0);
        ring2.fill(&mut scratch, time2.now_us(), latency);
        std::mem::take(&mut scratch)
    };
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => device.build_output_stream::<f32, _, _>(
            config,
            move |data: &mut [f32], info| {
                let buf = fill(data.len(), info);
                data.copy_from_slice(&buf);
            },
            err_cb,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_output_stream::<i16, _, _>(
            config,
            move |data: &mut [i16], info| {
                let buf = fill(data.len(), info);
                for (o, s) in data.iter_mut().zip(buf) {
                    *o = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                }
            },
            err_cb,
            None,
        ),
        cpal::SampleFormat::I32 => device.build_output_stream::<i32, _, _>(
            config,
            move |data: &mut [i32], info| {
                let buf = fill(data.len(), info);
                for (o, s) in data.iter_mut().zip(buf) {
                    *o = (s.clamp(-1.0, 1.0) as f64 * i32::MAX as f64) as i32;
                }
            },
            err_cb,
            None,
        ),
        other => {
            return Err(AudioUnavailable::Failed(format!("unsupported device sample format {other:?}")));
        }
    }
    .map_err(|e| AudioUnavailable::Failed(e.to_string()))?;
    stream.play().map_err(|e| AudioUnavailable::Failed(e.to_string()))?;
    let name = device.description().map(|d| d.name().to_string()).ok();
    if let Ok(mut s) = status.lock() {
        s.device = name;
        s.sample_rate = rate;
        s.channels = channels;
    }
    Ok(Active { _stream: stream, id: device.id().ok(), failed })
}

fn run(
    ring: AudioRing,
    status: Arc<Mutex<OutputStatus>>,
    stop: Arc<AtomicBool>,
    time: Arc<dyn TimeSource>,
    ready: mpsc::Sender<Result<(), AudioUnavailable>>,
) {
    let host = cpal::default_host();
    let mut active = match build(&host, &ring, &time, &status) {
        Ok(a) => {
            let _ = ready.send(Ok(()));
            Some(a)
        }
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(DEVICE_POLL);
        let default_id = host.default_output_device().and_then(|d| d.id().ok());
        let (cur, failed) = match &active {
            Some(a) => (a.id.clone(), a.failed.load(Ordering::SeqCst)),
            None => (None, true),
        };
        if needs_rebuild(cur.as_ref(), default_id.as_ref(), failed) {
            active = None; // release the old stream first
            match build(&host, &ring, &time, &status) {
                Ok(a) => {
                    active = Some(a);
                    if let Ok(mut s) = status.lock() {
                        s.device_changes += 1;
                        s.last_error = None;
                    }
                }
                Err(e) => {
                    if let Ok(mut s) = status.lock() {
                        s.device = None;
                        s.last_error = Some(e.to_string());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuild_decision() {
        assert!(!needs_rebuild(Some(&"a"), Some(&"a"), false));
        assert!(needs_rebuild(Some(&"a"), Some(&"b"), false), "default device changed (headphones)");
        assert!(needs_rebuild(Some(&"a"), None, false), "device removed");
        assert!(needs_rebuild(None, Some(&"b"), false), "device appeared");
        assert!(needs_rebuild(Some(&"a"), Some(&"a"), true), "stream error");
    }

    /// Opening the default device either works or reports a plain `AudioUnavailable` (CI and containers usually have
    /// no sound card); it never panics or hangs.
    #[test]
    fn open_default_succeeds_or_reports_unavailable() {
        let t: Arc<dyn TimeSource> = Arc::new(crate::clock::SystemTime::new());
        match AudioOutput::open_default(t) {
            Ok(out) => {
                let st = out.status();
                assert!(st.sample_rate > 0 && st.channels > 0);
                assert_eq!(out.ring().format(), (st.sample_rate, st.channels));
            }
            Err(e) => eprintln!("audio unavailable here: {e}"),
        }
    }
}
