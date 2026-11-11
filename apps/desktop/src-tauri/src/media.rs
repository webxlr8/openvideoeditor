//! Opening media for preview.
//!
//! Until the decoder (ove-media) is wired into the shell, an accepted file is played back as a
//! generated test pattern with the file's name; `MediaInfo::placeholder` says so. Path checks and
//! error reporting are final.

use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::{AppError, AppResult, ErrorCode};
use ove_gpu::VideoFrame;

/// Containers accepted for preview (specs/media-playback: Open media for preview).
pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "mov", "mxf", "mkv", "webm"];
pub const AUDIO_EXTENSIONS: &[&str] = &["m4a", "wav", "caf", "opus", "ogg", "aac"];

/// Properties shown next to the viewer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    pub path: String,
    pub name: String,
    pub duration_seconds: f64,
    pub width: u32,
    pub height: u32,
    pub frame_rate: f64,
    pub total_frames: u32,
    pub video_codec: String,
    pub audio_layout: Option<String>,
    /// True while the file is shown as a generated test pattern (decoder not wired yet).
    pub placeholder: bool,
}

fn display_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

/// Validates `path` and returns what the viewer will show. Errors name the file and the reason.
pub fn open(path: &Path) -> AppResult<MediaInfo> {
    let name = display_name(path);
    let meta = std::fs::metadata(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => AppError::new(
            ErrorCode::NotFound,
            format!("“{name}” could not be found. It may have been moved or deleted."),
        ),
        std::io::ErrorKind::PermissionDenied => {
            AppError::new(ErrorCode::Io, format!("“{name}” could not be opened: permission denied."))
        }
        _ => AppError::new(ErrorCode::Io, format!("“{name}” could not be opened: {e}.")),
    })?;
    if meta.is_dir() {
        return Err(AppError::new(ErrorCode::Unsupported, format!("“{name}” is a folder, not a media file.")));
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let is_video = VIDEO_EXTENSIONS.contains(&ext.as_str());
    let is_audio = AUDIO_EXTENSIONS.contains(&ext.as_str());
    if !is_video && !is_audio {
        let what = if ext.is_empty() { "files without an extension".to_string() } else { format!(".{ext} files") };
        return Err(AppError::new(
            ErrorCode::Unsupported,
            format!(
                "“{name}” can't be opened: {what} are not supported. Supported: MP4, MOV, MXF, MKV, WebM video and M4A, WAV, CAF, Opus audio."
            ),
        ));
    }
    if meta.len() == 0 {
        return Err(AppError::new(ErrorCode::Corrupt, format!("“{name}” is empty (0 bytes) and can't be played.")));
    }
    Ok(test_pattern_info(path.display().to_string(), name, is_audio))
}

pub fn test_pattern_info(path: String, name: String, audio_only: bool) -> MediaInfo {
    MediaInfo {
        path,
        name,
        duration_seconds: 60.0,
        width: if audio_only { 0 } else { 1920 },
        height: if audio_only { 0 } else { 1080 },
        frame_rate: 30.0,
        total_frames: 1800,
        video_codec: if audio_only { "none".into() } else { "test pattern".into() },
        audio_layout: None,
        placeholder: true,
    }
}

const PATTERN_W: u32 = 640;
const PATTERN_H: u32 = 360;

/// A generated NV12 frame for `frame` (colour bars, a moving marker, and a frame-number bar
/// code so frame accuracy is visible). Limited-range BT.709.
pub fn test_pattern_frame(frame: u32, total: u32, fps: f64) -> VideoFrame {
    const BARS: [(u8, u8, u8); 8] = [
        (180, 128, 128),
        (168, 44, 136),
        (145, 147, 44),
        (133, 63, 52),
        (63, 193, 204),
        (51, 109, 212),
        (28, 212, 120),
        (16, 128, 128),
    ];
    let (w, h) = (PATTERN_W, PATTERN_H);
    let mut y = vec![0u8; (w * h) as usize];
    let mut uv = vec![128u8; (w * h / 2) as usize];
    let marker = (u64::from(frame) * u64::from(w) / u64::from(total.max(1))) as u32;
    for row in 0..h {
        for col in 0..w {
            let (yy, u, v) = if row < h * 3 / 4 {
                BARS[(col * 8 / w) as usize]
            } else if row < h * 7 / 8 {
                // 16-bit frame number as black/white blocks.
                let bit = col * 16 / w;
                if frame >> (15 - bit) & 1 == 1 { (235, 128, 128) } else { (16, 128, 128) }
            } else {
                let ramp = 16 + (col * 219 / (w - 1)) as u8;
                (ramp, 128, 128)
            };
            let on_marker = col.abs_diff(marker) < 2;
            y[(row * w + col) as usize] = if on_marker { 235 } else { yy };
            if row % 2 == 0 && col % 2 == 0 {
                let i = ((row / 2) * w + col) as usize;
                uv[i] = if on_marker { 128 } else { u };
                uv[i + 1] = if on_marker { 128 } else { v };
            }
        }
    }
    let pts = (f64::from(frame) / fps * 1_000_000.0).round() as i64;
    VideoFrame::nv12(w, h, y, uv, pts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ove-media-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn missing_file_names_it() {
        let err = open(Path::new("/definitely/not/here/clip.mov")).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert!(err.message.contains("clip.mov"));
    }

    #[test]
    fn unsupported_extension() {
        let err = open(&temp("notes.txt", b"hi")).unwrap_err();
        assert_eq!(err.code, ErrorCode::Unsupported);
        assert!(err.message.contains("notes.txt") && err.message.contains(".txt"));
    }

    #[test]
    fn empty_file_is_corrupt() {
        let err = open(&temp("empty.mov", b"")).unwrap_err();
        assert_eq!(err.code, ErrorCode::Corrupt);
    }

    #[test]
    fn folder_is_rejected() {
        let err = open(&std::env::temp_dir()).unwrap_err();
        assert_eq!(err.code, ErrorCode::Unsupported);
    }

    #[test]
    fn accepted_file_is_placeholder_until_decoder_lands() {
        let info = open(&temp("A.MOV", b"data")).unwrap();
        assert_eq!(info.name, "A.MOV");
        assert!(info.placeholder);
    }

    #[test]
    fn pattern_frames_are_valid_and_distinct() {
        let a = test_pattern_frame(0, 1800, 30.0);
        let b = test_pattern_frame(1, 1800, 30.0);
        a.validate().unwrap();
        assert_ne!(a.planes[0], b.planes[0]);
        assert_eq!(b.pts, 33_333);
    }
}
