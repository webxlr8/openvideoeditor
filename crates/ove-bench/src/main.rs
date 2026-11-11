//! Headless playback benchmark (task 3.3): plays a file through the real decode worker + player at 1× on the
//! system clock, then issues random seeks, and writes machine-readable metrics (see `schema.json`).
//!
//! ```text
//! ove-bench <file> [--seconds N] [--seeks N] [--seed N] [--hwaccel auto|off|<device>] [--queue N] [--out FILE]
//! ```
//! Exit codes: 0 success, 1 the file could not be opened or decoded, 2 bad arguments.

mod memory;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use ove_media::{DecodeWorker, DecoderOptions, HwAccel, MediaInfo, open_video, probe};
use ove_playback::{Command, FrameSource, Player, PlayerConfig, SourcePoll, SystemTime, TimeSource, WorkerSource};
use serde::Serialize;

const USAGE: &str = "usage: ove-bench <file> [--seconds N] [--seeks N] [--seed N] [--hwaccel auto|off|<device>] \
                     [--queue N] [--out FILE]";

#[derive(Debug)]
struct Args {
    file: PathBuf,
    seconds: f64,
    seeks: usize,
    seed: u64,
    hwaccel: HwAccel,
    queue: usize,
    out: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let mut a = Args {
        file: PathBuf::new(),
        seconds: 60.0,
        seeks: 100,
        seed: 1,
        hwaccel: HwAccel::from_env(),
        queue: ove_media::DEFAULT_QUEUE_FRAMES,
        out: None,
    };
    let mut file = None;
    while let Some(arg) = it.next() {
        let mut val = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--seconds" => a.seconds = val("--seconds")?.parse().map_err(|e| format!("--seconds: {e}"))?,
            "--seeks" => a.seeks = val("--seeks")?.parse().map_err(|e| format!("--seeks: {e}"))?,
            "--seed" => a.seed = val("--seed")?.parse().map_err(|e| format!("--seed: {e}"))?,
            "--queue" => a.queue = val("--queue")?.parse().map_err(|e| format!("--queue: {e}"))?,
            "--out" => a.out = Some(PathBuf::from(val("--out")?)),
            "--hwaccel" => {
                a.hwaccel = match val("--hwaccel")?.as_str() {
                    "auto" => HwAccel::Auto,
                    "off" => HwAccel::Off,
                    other => HwAccel::Named(other.to_string()),
                }
            }
            "-h" | "--help" => return Err(USAGE.into()),
            s if s.starts_with('-') => return Err(format!("unknown option {s}\n{USAGE}")),
            s => file = Some(PathBuf::from(s)),
        }
    }
    a.file = file.ok_or_else(|| USAGE.to_string())?;
    Ok(a)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaSummary {
    container: String,
    codec: String,
    profile: Option<String>,
    width: u32,
    height: u32,
    pixel_format: Option<String>,
    frame_rate: f64,
    variable_frame_rate: bool,
    duration_us: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OffsetStats {
    mean: i64,
    p95: i64,
    max: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SeekStats {
    count: usize,
    p50: i64,
    p95: i64,
    max: i64,
    /// Every seek showed the frame whose interval contains the target.
    exact: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    schema_version: u32,
    file: String,
    media: MediaSummary,
    decode_path: String,
    queue_frames: usize,
    play_seconds: f64,
    presented_frames: u64,
    dropped_frames: u64,
    skipped_frames: u64,
    /// |master clock − pts| at presentation, µs.
    av_offset_us: OffsetStats,
    /// Seek request → first frame of the new position decoded, µs.
    seek_us: SeekStats,
    peak_memory_bytes: u64,
    machine: Machine,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Machine {
    os: &'static str,
    arch: &'static str,
    cpus: usize,
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
}

fn pct(v: &mut [i64], p: f64) -> i64 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn summary(info: &MediaInfo) -> Result<MediaSummary, String> {
    let v = info.video.as_ref().ok_or("no video stream")?;
    Ok(MediaSummary {
        container: info.container.clone(),
        codec: v.codec.clone(),
        profile: v.profile.clone(),
        width: v.width,
        height: v.height,
        pixel_format: v.pixel_format.as_ref().map(|p| p.name.clone()),
        frame_rate: v.frame_rate.as_f64().or(v.base_frame_rate.as_f64()).unwrap_or(0.0),
        variable_frame_rate: v.variable_frame_rate,
        duration_us: info.duration_us.or(v.duration_us),
    })
}

fn run(a: &Args) -> Result<Report, String> {
    let info = probe(&a.file).map_err(|e| e.to_string())?;
    let media = summary(&info)?;
    let fps = if media.frame_rate > 0.0 { media.frame_rate } else { 30.0 };
    let interval = (1e6 / fps).round() as i64;
    let duration = media.duration_us.unwrap_or(0).max(interval);
    let last_frame = (duration - interval).max(0);
    let opts = DecoderOptions { hwaccel: a.hwaccel.clone(), ..Default::default() };

    // --- 1× playback on the system clock -------------------------------------------------------------------------
    let decoder = open_video(&a.file, opts.clone()).map_err(|e| e.to_string())?;
    let source = WorkerSource::new(DecodeWorker::spawn(Box::new(decoder), a.queue));
    let time = SystemTime::new();
    let cfg = PlayerConfig { frame_interval_us: interval, last_frame_us: last_frame };
    let mut player = Player::new(source, cfg, None, time.now_us());
    // Wait for the first frame (and let the queue fill) before starting, like the app does after opening a file.
    let ready = Instant::now();
    while player.tick(time.now_us()).present.is_none() {
        if ready.elapsed() > Duration::from_secs(10) {
            return Err("first frame did not arrive within 10 s".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    std::thread::sleep(Duration::from_millis(100));
    let play_for = (a.seconds * 1e6) as i64;
    let play_for = play_for.min(duration);
    let t0 = time.now_us();
    player.command(Command::Play, t0);
    let mut error = None;
    while time.now_us() - t0 < play_for && player.transport().is_playing() {
        let out = player.tick(time.now_us());
        if let Some(e) = out.error {
            error = Some(e);
            break;
        }
        // ~1 kHz scheduler tick (a display link drives this in the app).
        std::thread::sleep(Duration::from_micros(500));
    }
    if let Some(e) = error {
        return Err(e);
    }
    let played = (time.now_us() - t0) as f64 / 1e6;
    let diag = player.diagnostics();
    let decode_path = player.source_mut().decode_path();

    // --- random seeks ---------------------------------------------------------------------------------------------
    let mut rng = Rng(a.seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    let mut lat = Vec::with_capacity(a.seeks);
    let mut exact = true;
    let source = player.source_mut();
    for _ in 0..a.seeks {
        let t = (rng.next() % (last_frame.max(1) as u64)) as i64;
        let start = Instant::now();
        source.seek(t);
        let frame = loop {
            match source.poll() {
                SourcePoll::Frame(f) => break f,
                SourcePoll::Pending => std::thread::sleep(Duration::from_micros(100)),
                SourcePoll::Eof => return Err(format!("no frame after seeking to {t} µs")),
                SourcePoll::Error(e) => return Err(e),
            }
            if start.elapsed() > Duration::from_secs(10) {
                return Err(format!("seek to {t} µs timed out"));
            }
        };
        lat.push(start.elapsed().as_micros() as i64);
        // Exact: the shown frame starts at or before the target (or is the first frame) and the next frame starts
        // after it. Container durations are not trusted for this (Matroska rounds them to milliseconds).
        let starts_ok = frame.pts_us <= t || frame.pts_us <= interval;
        let next_after = loop {
            match source.poll() {
                SourcePoll::Frame(n) => break n.pts_us > t,
                SourcePoll::Pending => std::thread::sleep(Duration::from_micros(100)),
                SourcePoll::Eof => break true,
                SourcePoll::Error(e) => return Err(e),
            }
        };
        if !(starts_ok && next_after) {
            exact = false;
        }
    }
    let max = lat.iter().copied().max().unwrap_or(0);

    Ok(Report {
        schema_version: 1,
        file: a.file.display().to_string(),
        media,
        decode_path: decode_path.to_string(),
        queue_frames: a.queue,
        play_seconds: played,
        presented_frames: diag.presented_frames,
        dropped_frames: diag.dropped_frames,
        skipped_frames: diag.skipped_frames,
        av_offset_us: OffsetStats {
            mean: diag.av_offset.mean_us,
            p95: diag.av_offset.p95_us,
            max: diag.av_offset.max_us,
        },
        seek_us: SeekStats { count: lat.len(), p50: pct(&mut lat, 0.5), p95: pct(&mut lat, 0.95), max, exact },
        peak_memory_bytes: memory::peak_bytes(),
        machine: Machine {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            cpus: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        },
    })
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    match run(&args) {
        Ok(report) => {
            let json = serde_json::to_string_pretty(&report).expect("report serializes");
            match &args.out {
                Some(p) => {
                    if let Err(e) = std::fs::write(p, json + "\n") {
                        eprintln!("{}: {e}", p.display());
                        std::process::exit(1);
                    }
                }
                None => println!("{json}"),
            }
        }
        Err(e) => {
            eprintln!("{}: {e}", args.file.display());
            std::process::exit(1);
        }
    }
}
