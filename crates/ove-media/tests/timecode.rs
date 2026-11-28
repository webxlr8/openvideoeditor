//! Task 1.1: source timecode from QuickTime timecode tracks, Broadcast WAV and iXML. The video fixtures are made by
//! the fixture ffmpeg (`OVE_FIXTURE_FFMPEG` or `ffmpeg` on PATH); the WAV fixtures are written here, chunk by chunk.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use ove_media::{SourceTimecode, TimecodeSource, probe};

fn ffmpeg() -> Option<String> {
    let bin = std::env::var("OVE_FIXTURE_FFMPEG").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "ffmpeg".into());
    Command::new(&bin).arg("-version").output().ok().filter(|o| o.status.success()).map(|_| bin)
}

/// A short video with a timecode track (or none when `tc` is `None`).
fn video(dir: &Path, name: &str, rate: &str, tc: Option<&str>) -> Option<PathBuf> {
    let ff = ffmpeg()?;
    let out = dir.join(name);
    let mut cmd = Command::new(ff);
    cmd.args(["-v", "error", "-y", "-f", "lavfi", "-i"]).arg(format!("testsrc2=s=160x90:r={rate}:d=1")).args([
        "-f",
        "lavfi",
        "-i",
        "sine=f=440:r=48000:d=1",
        "-c:v",
        "mpeg4",
        "-c:a",
        "pcm_s16le",
    ]);
    if name.ends_with(".mp4") {
        cmd.args(["-c:a", "aac"]);
    }
    if let Some(tc) = tc {
        cmd.args(["-timecode", tc]);
    }
    let st = cmd.arg(&out).status().unwrap();
    assert!(st.success(), "ffmpeg failed for {name}");
    Some(out)
}

fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = id.to_vec();
    v.extend_from_slice(&(body.len() as u32).to_le_bytes());
    v.extend_from_slice(body);
    if body.len() % 2 == 1 {
        v.push(0);
    }
    v
}

/// A 48 kHz mono 16-bit WAV of 0.5 s of silence with optional `bext` time reference and iXML text.
fn wav(dir: &Path, name: &str, time_reference: Option<u64>, ixml: Option<&str>) -> PathBuf {
    let rate = 48_000u32;
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&1u16.to_le_bytes()); // PCM
    fmt.extend_from_slice(&1u16.to_le_bytes()); // mono
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * 2).to_le_bytes());
    fmt.extend_from_slice(&2u16.to_le_bytes());
    fmt.extend_from_slice(&16u16.to_le_bytes());
    let mut body = b"WAVE".to_vec();
    if let Some(t) = time_reference {
        let mut bext = vec![0u8; 602];
        bext[0..10].copy_from_slice(b"test scene");
        bext[338..346].copy_from_slice(&t.to_le_bytes());
        bext[346..348].copy_from_slice(&1u16.to_le_bytes()); // version
        body.extend(chunk(b"bext", &bext));
    }
    body.extend(chunk(b"fmt ", &fmt));
    if let Some(x) = ixml {
        body.extend(chunk(b"iXML", x.as_bytes()));
    }
    body.extend(chunk(b"data", &vec![0u8; rate as usize]));
    let mut file = b"RIFF".to_vec();
    file.extend_from_slice(&(body.len() as u32).to_le_bytes());
    file.extend(body);
    let out = dir.join(name);
    std::fs::write(&out, file).unwrap();
    out
}

#[test]
fn video_timecode_tracks_keep_their_exact_rate() {
    let dir = tempfile::tempdir().unwrap();
    let Some(v2398) = video(dir.path(), "cam_2398.mov", "24000/1001", Some("10:00:00:00")) else {
        eprintln!("skipped: no ffmpeg CLI to make fixtures (set OVE_FIXTURE_FFMPEG)");
        return;
    };
    let tc = probe(&v2398).unwrap().source_timecode.expect("23.976 timecode");
    assert_eq!((tc.rate_num, tc.rate_den, tc.drop_frame), (24000, 1001, false));
    assert_eq!(tc.frames, 10 * 3600 * 24);
    assert_eq!(tc.label(), "10:00:00:00");
    assert_eq!(tc.source, TimecodeSource::Container);
    // 864,000 frames at 24000/1001 is 36,036 s of real time.
    assert_eq!(tc.start(), (864_000 * 1001, 24_000));

    // Drop-frame is defined for multiples of 30000/1001 (FFmpeg refuses it at 23.976), so it is checked at 29.97.
    let v2997 = video(dir.path(), "cam_2997df.mov", "30000/1001", Some("10:00:00;00")).unwrap();
    let tc = probe(&v2997).unwrap().source_timecode.expect("29.97 DF timecode");
    assert_eq!((tc.rate_num, tc.rate_den, tc.drop_frame), (30000, 1001, true));
    assert_eq!(tc.label(), "10:00:00;00");
    assert_eq!(tc.frames, 1_078_920);

    let v25 = video(dir.path(), "cam_25.mp4", "25", Some("01:02:03:04")).unwrap();
    let tc = probe(&v25).unwrap().source_timecode.expect("25 fps timecode");
    assert_eq!((tc.rate_num, tc.rate_den, tc.drop_frame), (25, 1, false));
    assert_eq!(tc.label(), "01:02:03:04");
    assert_eq!(tc.seconds(), 3723.0 + 4.0 / 25.0);

    let none = video(dir.path(), "phone.mp4", "30", None).unwrap();
    let info = probe(&none).unwrap();
    assert_eq!(info.source_timecode, None);
    let json = serde_json::to_value(&info).unwrap();
    assert!(json.get("sourceTimecode").is_none(), "no timecode is not printed");
}

#[test]
fn broadcast_wav_and_ixml_timecode_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let ixml = "<?xml version=\"1.0\"?><BWFXML><SPEED><TIMECODE_RATE>24000/1001</TIMECODE_RATE>\
                <TIMECODE_FLAG>NDF</TIMECODE_FLAG></SPEED></BWFXML>";
    let start = 864_000u64 * 1001 * 2 + 4 * 48_000; // 4 real seconds after 10:00:00:00 at 23.976, in samples
    let bwf = wav(dir.path(), "recorder.wav", Some(start), Some(ixml));
    let info = probe(&bwf).unwrap();
    let tc = info.source_timecode.expect("BWF timecode");
    assert_eq!(tc.source, TimecodeSource::Bwf);
    assert_eq!((tc.samples, tc.sample_rate), (Some(start), Some(48_000)));
    assert_eq!((tc.rate_num, tc.rate_den), (24000, 1001));
    assert_eq!(tc.label(), "10:00:03:23");

    // bext only: labelled at the default rate, start still exact.
    let bare = wav(dir.path(), "bext_only.wav", Some(48_000 * 3600), None);
    let tc = probe(&bare).unwrap().source_timecode.unwrap();
    assert_eq!(tc.seconds(), 3600.0);
    // iXML only.
    let x = "<BWFXML><BWF_TIME_REFERENCE_LOW>96000</BWF_TIME_REFERENCE_LOW><SPEED><TIMECODE_RATE>25</TIMECODE_RATE>\
             </SPEED></BWFXML>";
    let ix = wav(dir.path(), "ixml_only.wav", None, Some(x));
    let tc = probe(&ix).unwrap().source_timecode.unwrap();
    assert_eq!((tc.source, tc.label().as_str()), (TimecodeSource::Ixml, "00:00:02:00"));
    // A zero reference with no iXML is "not set"; a WAV without chunks has no timecode.
    assert_eq!(probe(&wav(dir.path(), "zero.wav", Some(0), None)).unwrap().source_timecode, None);
    assert_eq!(probe(&wav(dir.path(), "plain.wav", None, None)).unwrap().source_timecode, None);
}

#[test]
fn a_video_and_a_wav_with_shared_timecode_align_to_the_exact_sample() {
    let dir = tempfile::tempdir().unwrap();
    let Some(cam) = video(dir.path(), "cam.mov", "24000/1001", Some("10:00:00:00")) else {
        eprintln!("skipped: no ffmpeg CLI to make fixtures (set OVE_FIXTURE_FFMPEG)");
        return;
    };
    let cam_tc: SourceTimecode = probe(&cam).unwrap().source_timecode.unwrap();
    let start = 864_000u64 * 1001 * 2 + 4 * 48_000;
    let rec = wav(dir.path(), "rec.wav", Some(start), None);
    let rec_tc = probe(&rec).unwrap().source_timecode.unwrap();
    assert_eq!(cam_tc.offset_samples(&rec_tc, 48_000), 4 * 48_000);
    assert_eq!(cam_tc.offset_us(&rec_tc), 4_000_000);
    // One sample later is one sample further.
    let rec2 = wav(dir.path(), "rec2.wav", Some(start + 1), None);
    let rec2_tc = probe(&rec2).unwrap().source_timecode.unwrap();
    assert_eq!(cam_tc.offset_samples(&rec2_tc, 48_000), 4 * 48_000 + 1);
}
