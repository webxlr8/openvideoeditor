//! Which FFmpeg encoder and muxer write a validated [`Plan`] on this computer (design D2).
//!
//! Each codec has an ordered candidate list. The list is filtered by what this FFmpeg build contains (the
//! [`Inventory`]); the first candidate that opens is used, and the ones that did not open are reported as fallbacks.
//! HEVC and AV1 come only from the operating system's or the graphics vendor's encoders: there is no software
//! fallback, so a computer without one is refused before rendering with a suggestion. The licence rule keeps GPL
//! encoders out of shipped builds (the build script and `check-license.sh` refuse them); `libx264` stays in the H.264
//! list only for development builds that have it.

use std::collections::BTreeSet;
use std::ffi::CStr;
use std::ptr;
use std::sync::OnceLock;

use ffmpeg_the_third::ffi;

use super::settings::{AudioCodec, AudioPlan, Chroma, Container, Plan, SettingsError, VideoCodec, VideoPlan};

/// The encoders and muxers an FFmpeg build contains.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inventory {
    pub encoders: BTreeSet<String>,
    pub muxers: BTreeSet<String>,
}

impl Inventory {
    /// An inventory from names (tests).
    pub fn from_names<'a>(
        encoders: impl IntoIterator<Item = &'a str>,
        muxers: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        Inventory {
            encoders: encoders.into_iter().map(str::to_string).collect(),
            muxers: muxers.into_iter().map(str::to_string).collect(),
        }
    }

    /// What the FFmpeg libraries this process loaded contain (read once).
    pub fn current() -> &'static Inventory {
        static INV: OnceLock<Inventory> = OnceLock::new();
        INV.get_or_init(|| {
            ove_media::init();
            let mut inv = Inventory::default();
            // SAFETY: the iterators walk FFmpeg's static codec and muxer tables; `opaque` is the iteration state they
            // own, starting null. Every returned pointer is to a static entry whose `name` is a NUL-terminated
            // static string.
            unsafe {
                let mut opaque: *mut libc::c_void = ptr::null_mut();
                loop {
                    let c = ffi::av_codec_iterate(&mut opaque);
                    if c.is_null() {
                        break;
                    }
                    if ffi::av_codec_is_encoder(c) != 0 && !(*c).name.is_null() {
                        inv.encoders.insert(CStr::from_ptr((*c).name).to_string_lossy().into_owned());
                    }
                }
                let mut opaque: *mut libc::c_void = ptr::null_mut();
                loop {
                    let m = ffi::av_muxer_iterate(&mut opaque);
                    if m.is_null() {
                        break;
                    }
                    if !(*m).name.is_null() {
                        // Some muxers list several names ("mov,mp4"): keep each.
                        for n in CStr::from_ptr((*m).name).to_string_lossy().split(',') {
                            inv.muxers.insert(n.to_string());
                        }
                    }
                }
            }
            inv
        })
    }

    pub fn has_encoder(&self, name: &str) -> bool {
        self.encoders.contains(name)
    }

    pub fn has_muxer(&self, name: &str) -> bool {
        self.muxers.contains(name)
    }
}

/// H.264 encoder names to try, best first (the export of change 6).
pub fn h264_candidates() -> Vec<&'static str> {
    let mut v = Vec::new();
    if cfg!(target_os = "macos") {
        v.push("h264_videotoolbox");
    }
    if cfg!(windows) {
        v.extend(["h264_nvenc", "h264_qsv", "h264_amf", "h264_mf"]);
    }
    v.extend(["libopenh264", "libx264", "mpeg4"]);
    v
}

/// Encoder names for a codec on this operating system, best first, before filtering by the build.
pub fn video_candidates(codec: VideoCodec, alpha: bool) -> Vec<&'static str> {
    let mac = cfg!(target_os = "macos");
    let win = cfg!(windows);
    match codec {
        VideoCodec::H264 => h264_candidates(),
        VideoCodec::Hevc if mac => vec!["hevc_videotoolbox"],
        VideoCodec::Hevc if win => vec!["hevc_nvenc", "hevc_qsv", "hevc_amf", "hevc_mf"],
        VideoCodec::Hevc => vec![],
        // VideoToolbox first on macOS for Apple's own encoder, except with alpha, which goes through the encoder whose
        // alpha format is verified in tests.
        VideoCodec::Prores if mac && !alpha => vec!["prores_videotoolbox", "prores_ks"],
        VideoCodec::Prores => vec!["prores_ks"],
        VideoCodec::Dnxhr => vec!["dnxhd"],
        VideoCodec::Vp9 => vec!["libvpx-vp9"],
        VideoCodec::Av1 if win => vec!["av1_nvenc", "av1_qsv", "av1_amf"],
        VideoCodec::Av1 => vec![],
    }
}

/// True for the platform hardware (or operating-system) encoders.
pub fn is_hardware_encoder(name: &str) -> bool {
    ["videotoolbox", "nvenc", "qsv", "amf", "_mf", "vaapi"].iter().any(|k| name.contains(k))
}

/// How an encoder is named to people: "ProRes (FFmpeg encoder)" for `prores_ks`, which is not Apple-certified.
pub fn encoder_label(name: &str) -> String {
    match name {
        "prores_ks" => "ProRes (FFmpeg encoder)".into(),
        "prores_videotoolbox" => "ProRes (Apple VideoToolbox)".into(),
        "dnxhd" => "DNxHR (FFmpeg encoder)".into(),
        "libvpx-vp9" => "VP9 (libvpx)".into(),
        n if n.ends_with("_videotoolbox") => format!("{} (Apple VideoToolbox)", codec_word(n)),
        n if n.ends_with("_nvenc") => format!("{} (NVIDIA NVENC)", codec_word(n)),
        n if n.ends_with("_qsv") => format!("{} (Intel Quick Sync)", codec_word(n)),
        n if n.ends_with("_amf") => format!("{} (AMD AMF)", codec_word(n)),
        n if n.ends_with("_mf") => format!("{} (Windows Media Foundation)", codec_word(n)),
        n => n.to_string(),
    }
}

fn codec_word(name: &str) -> &'static str {
    match name.split('_').next() {
        Some("h264") => "H.264",
        Some("hevc") => "HEVC",
        Some("av1") => "AV1",
        Some("prores") => "ProRes",
        _ => "video",
    }
}

/// The encoders of `candidates` this build has, in order; refused with a suggestion when there are none.
pub fn video_encoders(v: &VideoPlan, inv: &Inventory) -> Result<Vec<&'static str>, SettingsError> {
    let found: Vec<&'static str> =
        video_candidates(v.codec, v.alpha).into_iter().filter(|n| inv.has_encoder(n)).collect();
    if !found.is_empty() {
        return Ok(found);
    }
    let available = |codecs: &[VideoCodec]| -> Vec<&'static str> {
        codecs
            .iter()
            .filter(|c| video_candidates(**c, false).iter().any(|n| inv.has_encoder(n)))
            .map(|c| c.label())
            .collect()
    };
    let message = match v.codec {
        VideoCodec::Hevc => format!(
            "no HEVC encoder on this computer (HEVC is written only by the operating system's or the graphics card's \
             encoder); export {} instead",
            or_list(&available(&[VideoCodec::Prores, VideoCodec::H264]), "ProRes or H.264")
        ),
        VideoCodec::Av1 => format!(
            "no AV1 hardware encoder on this computer; export {} instead",
            or_list(&available(&[VideoCodec::Vp9, VideoCodec::H264, VideoCodec::Prores]), "VP9 or H.264")
        ),
        c => format!(
            "this build of the app has no {} encoder ({}); export {} instead",
            c.label(),
            video_candidates(c, v.alpha).join(", "),
            or_list(&available(&[VideoCodec::H264, VideoCodec::Prores]), "H.264")
        ),
    };
    Err(SettingsError::new(&["video.codec"], message))
}

fn or_list(items: &[&str], fallback: &str) -> String {
    match items {
        [] => fallback.to_string(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

/// How the sound is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioWriter {
    /// Through an FFmpeg encoder.
    Encoder(&'static str),
    /// PCM packets packed here (no encoder needed).
    Pcm,
}

impl AudioWriter {
    pub fn name(self, bit_depth: u8) -> &'static str {
        match self {
            AudioWriter::Encoder(n) => n,
            AudioWriter::Pcm if bit_depth == 16 => "pcm_s16le",
            AudioWriter::Pcm => "pcm_s24le",
        }
    }
}

pub fn audio_writer(a: &AudioPlan, inv: &Inventory) -> Result<AudioWriter, SettingsError> {
    let name = match a.codec {
        AudioCodec::Pcm => return Ok(AudioWriter::Pcm),
        AudioCodec::Aac => "aac",
        AudioCodec::Flac => "flac",
        AudioCodec::Opus => "libopus",
    };
    if inv.has_encoder(name) {
        Ok(AudioWriter::Encoder(name))
    } else {
        Err(SettingsError::new(
            &["audio.codec"],
            format!("this build of the app has no {} encoder ({name})", a.codec.label()),
        ))
    }
}

/// How the file is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileWriter {
    /// Through an FFmpeg muxer.
    Muxer(&'static str),
    /// WAV, written here.
    Wav,
}

pub fn file_writer(c: Container, inv: &Inventory) -> Result<FileWriter, SettingsError> {
    let names: &[&'static str] = match c {
        Container::Wav => return Ok(FileWriter::Wav),
        Container::Mp4 => &["mp4"],
        Container::Mov => &["mov"],
        Container::Mkv => &["matroska"],
        Container::Webm => &["webm"],
        // `ipod` writes the M4A brand; plain MP4 is the same file structure.
        Container::M4a => &["ipod", "mp4"],
        Container::Flac => &["flac"],
    };
    names.iter().find(|n| inv.has_muxer(n)).map(|n| FileWriter::Muxer(n)).ok_or_else(|| {
        SettingsError::new(
            &["container"],
            format!("this build of the app cannot write {} files ({} muxer missing)", c.label(), names.join("/")),
        )
    })
}

/// Everything an export needs from the build, chosen before rendering.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub file: FileWriter,
    /// Video encoders to try in order (empty for audio-only).
    pub video: Vec<String>,
    pub audio: Option<AudioWriter>,
}

/// Picks the muxer and encoders for `plan`, or says what this computer lacks.
pub fn choose(plan: &Plan, inv: &Inventory, forced: Option<&str>) -> Result<Choice, SettingsError> {
    let file = file_writer(plan.container, inv)?;
    let video = match (&plan.video, forced) {
        (None, _) => Vec::new(),
        // A forced encoder (tests) is tried even when the inventory does not list it: it then fails to open.
        (Some(_), Some(f)) => vec![f.to_string()],
        (Some(v), None) => video_encoders(v, inv)?.into_iter().map(str::to_string).collect(),
    };
    let audio = match &plan.audio {
        Some(a) => Some(audio_writer(a, inv)?),
        None => None,
    };
    Ok(Choice { file, video, audio })
}

/// Pixel formats (FFmpeg names) to feed the encoder, best first; the first one the encoder supports is used.
pub fn pixel_formats(v: &VideoPlan) -> &'static [&'static str] {
    if v.alpha {
        return match v.codec {
            VideoCodec::Prores => &["yuva444p10le"],
            _ => &["yuva420p"],
        };
    }
    match (v.chroma, v.bit_depth) {
        (Chroma::C420, 8) => &["yuv420p", "nv12"],
        (Chroma::C420, _) => &["p010le", "yuv420p10le"],
        (Chroma::C422, 8) => &["yuv422p", "nv16"],
        (Chroma::C422, _) => &["yuv422p10le", "p210le"],
        (Chroma::C444, 8) => &["yuv444p", "gbrp"],
        (Chroma::C444, _) => &["yuv444p10le", "gbrp10le"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::settings::{ExportSettings, VideoSettings};

    fn plan(container: Container, codec: VideoCodec) -> Plan {
        ExportSettings {
            container,
            video: Some(VideoSettings { codec, ..VideoSettings::default() }),
            audio: None,
            ..ExportSettings::default()
        }
        .validate(None)
        .unwrap()
    }

    const ALL_MUXERS: [&str; 7] = ["mp4", "mov", "matroska", "webm", "ipod", "flac", "wav"];

    #[test]
    fn mocked_inventory_picks_the_first_available_candidate() {
        let inv = Inventory::from_names(["hevc_qsv", "hevc_mf", "prores_ks", "aac"], ALL_MUXERS);
        let c = choose(&plan(Container::Mp4, VideoCodec::Hevc), &inv, None).unwrap();
        if cfg!(windows) {
            assert_eq!(c.video, ["hevc_qsv", "hevc_mf"]);
        }
        let c = choose(&plan(Container::Mov, VideoCodec::Prores), &inv, None).unwrap();
        assert_eq!(c.video.last().map(String::as_str), Some("prores_ks"));
        assert_eq!(c.file, FileWriter::Muxer("mov"));
    }

    #[test]
    fn hevc_is_refused_without_a_permitted_encoder_and_suggests_prores_or_h264() {
        // A build with only software encoders: libx265 is never a candidate, even when present.
        let inv = Inventory::from_names(["libx265", "prores_ks", "h264_mf", "aac"], ALL_MUXERS);
        let e = choose(&plan(Container::Mp4, VideoCodec::Hevc), &inv, None).unwrap_err();
        assert_eq!(e.fields, ["video.codec"]);
        assert!(e.message.contains("no HEVC encoder"), "{e}");
        assert!(e.message.contains("ProRes"), "{e}");
        let e = choose(&plan(Container::Mp4, VideoCodec::Av1), &inv, None).unwrap_err();
        assert!(e.message.contains("no AV1 hardware encoder"), "{e}");
        assert!(!video_candidates(VideoCodec::Av1, false).iter().any(|n| n.contains("svt") || n.contains("aom")));
        assert!(!video_candidates(VideoCodec::Hevc, false).contains(&"libx265"));
    }

    #[test]
    fn missing_muxers_and_audio_encoders_are_named() {
        let inv = Inventory::from_names(["aac", "prores_ks"], ["mp4", "mov"]);
        let e = file_writer(Container::Webm, &inv).unwrap_err();
        assert!(e.message.contains("WebM"), "{e}");
        assert_eq!(file_writer(Container::M4a, &inv).unwrap(), FileWriter::Muxer("mp4"), "M4A falls back to mp4");
        assert_eq!(file_writer(Container::Wav, &inv).unwrap(), FileWriter::Wav, "WAV needs no muxer");
        let flac = AudioPlan { codec: AudioCodec::Flac, sample_rate: 48_000, bit_depth: 24, bitrate: 0 };
        assert!(audio_writer(&flac, &inv).unwrap_err().message.contains("FLAC"));
        let pcm = AudioPlan { codec: AudioCodec::Pcm, ..flac };
        assert_eq!(audio_writer(&pcm, &inv).unwrap(), AudioWriter::Pcm);
    }

    #[test]
    fn labels_and_pixel_formats() {
        assert_eq!(encoder_label("prores_ks"), "ProRes (FFmpeg encoder)");
        assert_eq!(encoder_label("hevc_nvenc"), "HEVC (NVIDIA NVENC)");
        let p = plan(Container::Mov, VideoCodec::Prores);
        assert_eq!(pixel_formats(p.video.as_ref().unwrap())[0], "yuv422p10le");
        let h = plan(Container::Mp4, VideoCodec::H264);
        assert_eq!(pixel_formats(h.video.as_ref().unwrap())[0], "yuv420p");
    }

    #[test]
    fn the_current_inventory_has_the_baseline() {
        let inv = Inventory::current();
        assert!(inv.has_encoder("aac") && inv.has_muxer("mp4") && inv.has_muxer("mov"), "{inv:?}");
        eprintln!("encoders: {:?}", inv.encoders);
    }
}
