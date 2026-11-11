//! Decode worker: bounded queue, seek flush via generations, panic containment, offline detection.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use common::{frame_md5, media, reference_framemd5};
use ove_media::{
    DecodePath, DecodeWorker, DecoderOptions, FfmpegVideoDecoder, HwAccel, MediaError, OutputFormat, VideoDecoder,
    VideoFrame, WorkerEvent,
};

fn native() -> DecoderOptions {
    DecoderOptions { hwaccel: HwAccel::Off, output: OutputFormat::Native, inject_hw_failure_after: None }
}

struct Counting {
    inner: FfmpegVideoDecoder,
    calls: Arc<AtomicU64>,
}

impl VideoDecoder for Counting {
    fn next_frame(&mut self) -> ove_media::Result<Option<VideoFrame>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.next_frame()
    }
    fn seek(&mut self, t: i64) -> ove_media::Result<()> {
        self.inner.seek(t)
    }
    fn decode_path(&self) -> DecodePath {
        self.inner.decode_path()
    }
}

#[test]
fn queue_is_bounded() {
    let calls = Arc::new(AtomicU64::new(0));
    let dec = Counting {
        inner: FfmpegVideoDecoder::open(&media("h264_longgop.mp4"), native()).unwrap(),
        calls: calls.clone(),
    };
    let mut w = DecodeWorker::spawn(Box::new(dec), 4);
    std::thread::sleep(Duration::from_millis(300));
    // 4 queued + 1 held by the worker while it waits for space.
    let n = calls.load(Ordering::SeqCst);
    assert!(n <= 5, "decoder ran ahead by {n} frames");
    // Consuming frees space and decoding continues in order.
    let mut last = -1;
    for _ in 0..20 {
        match w.recv_timeout(Duration::from_secs(2)).unwrap() {
            WorkerEvent::Frame(f) => {
                assert!(f.pts_us > last);
                last = f.pts_us;
            }
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn seek_flushes_queue_and_delivers_target_frame() {
    let path = media("h264_longgop.mp4");
    let reference = reference_framemd5(&path);
    let dec = FfmpegVideoDecoder::open(&path, native()).unwrap();
    let mut w = DecodeWorker::spawn(Box::new(dec), 8);
    std::thread::sleep(Duration::from_millis(100)); // let the queue fill with frames from the start
    for (t, k) in [(3_000_000, 90usize), (500_000, 15), (1_016_667, 30), (3_966_667, 119)] {
        w.seek(t);
        match w.recv_timeout(Duration::from_secs(5)).unwrap() {
            WorkerEvent::Frame(f) => assert_eq!(frame_md5(&f), reference[k], "seek to {t}"),
            other => panic!("{other:?}"),
        }
    }
    // After the last frame comes Eof, then a seek restarts decoding (loop playback).
    assert!(matches!(w.recv_timeout(Duration::from_secs(5)), Some(WorkerEvent::Eof)));
    w.seek(0);
    match w.recv_timeout(Duration::from_secs(5)).unwrap() {
        WorkerEvent::Frame(f) => assert_eq!(f.pts_us, 0),
        other => panic!("{other:?}"),
    }
}

struct Panicky;
impl VideoDecoder for Panicky {
    fn next_frame(&mut self) -> ove_media::Result<Option<VideoFrame>> {
        panic!("decoder bug")
    }
    fn seek(&mut self, _: i64) -> ove_media::Result<()> {
        Ok(())
    }
    fn decode_path(&self) -> DecodePath {
        DecodePath::Software
    }
}

#[test]
fn worker_panic_becomes_an_error() {
    let mut w = DecodeWorker::spawn(Box::new(Panicky), 4);
    match w.recv_timeout(Duration::from_secs(2)).unwrap() {
        WorkerEvent::Error(MediaError::WorkerPanicked(msg)) => assert!(msg.contains("decoder bug")),
        other => panic!("{other:?}"),
    }
    // The handle stays usable: further calls report the stopped worker instead of hanging or panicking.
    w.seek(0);
    assert!(matches!(w.recv_timeout(Duration::from_millis(200)), Some(WorkerEvent::Error(_)) | None));
}

#[test]
fn worker_reports_offline_media() {
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("clip.mov");
    std::fs::copy(media("prores_hq.mov"), &copy).unwrap();
    let dec = FfmpegVideoDecoder::open(&copy, native()).unwrap();
    let mut w = DecodeWorker::spawn(Box::new(dec), 2);
    assert!(matches!(w.recv_timeout(Duration::from_secs(2)), Some(WorkerEvent::Frame(_))));
    std::fs::remove_file(&copy).unwrap();
    let err = loop {
        match w.recv_timeout(Duration::from_secs(5)).expect("worker went silent") {
            WorkerEvent::Frame(_) => continue,
            WorkerEvent::Eof => panic!("reached the end without noticing the file was removed"),
            WorkerEvent::Error(e) => break e,
        }
    };
    assert_eq!(err.code(), "media.offline");
}
