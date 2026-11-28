//! The export settings model (change extend-export-formats-and-queue, design D1): one value that says what an export
//! writes — container, video codec, profile, bit depth, quality, alpha, audio codec, sample rate and bit depth, a time
//! range, the output size and frame rate — plus named presets that expand to such a value. The app, `ove export`
//! and the MCP `export_video` tool all go through [`ExportSettings::validate`], so an impossible combination is refused
//! the same way everywhere, before anything renders, with a message that names the conflicting fields.
//!
//! This module is plain data: no FFmpeg. Which encoder writes a codec on this computer is decided in
//! [`super::encoders`].

use std::fmt;
use std::sync::OnceLock;

use ove_core::time::{Micros, secs_to_micros};
use serde::{Deserialize, Serialize};

/// The file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Container {
    #[default]
    Mp4,
    Mov,
    Mkv,
    Webm,
    M4a,
    Wav,
    Flac,
}

impl Container {
    pub const ALL: [Container; 7] = [
        Container::Mp4,
        Container::Mov,
        Container::Mkv,
        Container::Webm,
        Container::M4a,
        Container::Wav,
        Container::Flac,
    ];

    /// The name used in settings (`mp4`, `mov`, …); also the usual file extension.
    pub fn name(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Mov => "mov",
            Container::Mkv => "mkv",
            Container::Webm => "webm",
            Container::M4a => "m4a",
            Container::Wav => "wav",
            Container::Flac => "flac",
        }
    }

    pub fn extension(self) -> &'static str {
        self.name()
    }

    pub fn label(self) -> &'static str {
        match self {
            Container::Mp4 => "MP4",
            Container::Mov => "MOV",
            Container::Mkv => "MKV",
            Container::Webm => "WebM",
            Container::M4a => "M4A",
            Container::Wav => "WAV",
            Container::Flac => "FLAC",
        }
    }

    /// Containers that hold sound only.
    pub fn audio_only(self) -> bool {
        matches!(self, Container::M4a | Container::Wav | Container::Flac)
    }

    /// A container by name or file extension (`matroska` and `.qt` included), ignoring case.
    pub fn parse(s: &str) -> Option<Container> {
        let s = s.trim().trim_start_matches('.').to_ascii_lowercase();
        Some(match s.as_str() {
            "mp4" | "m4v" => Container::Mp4,
            "mov" | "qt" | "quicktime" => Container::Mov,
            "mkv" | "matroska" => Container::Mkv,
            "webm" => Container::Webm,
            "m4a" | "aac" => Container::M4a,
            "wav" | "wave" => Container::Wav,
            "flac" => Container::Flac,
            _ => return None,
        })
    }

    /// The container a file path's extension names.
    pub fn from_path(p: &std::path::Path) -> Option<Container> {
        p.extension().and_then(|e| e.to_str()).and_then(Container::parse)
    }

    /// The video codecs this container can hold (empty for audio-only containers).
    pub fn video_codecs(self) -> &'static [VideoCodec] {
        use VideoCodec::*;
        match self {
            Container::Mp4 => &[H264, Hevc, Av1],
            Container::Mov => &[H264, Hevc, Prores, Dnxhr],
            Container::Mkv => &[H264, Hevc, Prores, Vp9, Av1],
            Container::Webm => &[Vp9, Av1],
            Container::M4a | Container::Wav | Container::Flac => &[],
        }
    }

    /// The audio codecs this container can hold.
    pub fn audio_codecs(self) -> &'static [AudioCodec] {
        use AudioCodec::*;
        match self {
            Container::Mp4 | Container::M4a => &[Aac],
            Container::Mov => &[Aac, Pcm],
            Container::Mkv => &[Aac, Pcm, Flac, Opus],
            Container::Webm => &[Opus],
            Container::Wav => &[Pcm],
            Container::Flac => &[Flac],
        }
    }

    /// What `{"container": "<this>"}` alone means: the usual video and audio for the container.
    pub fn defaults(self) -> (Option<VideoSettings>, Option<AudioSettings>) {
        let video = |codec| Some(VideoSettings { codec, ..VideoSettings::default() });
        let audio = |codec, bit_depth| Some(AudioSettings { codec, bit_depth, ..AudioSettings::default() });
        match self {
            Container::Mp4 | Container::Mov | Container::Mkv => (video(VideoCodec::H264), audio(AudioCodec::Aac, None)),
            Container::Webm => (video(VideoCodec::Vp9), audio(AudioCodec::Opus, None)),
            Container::M4a => (None, audio(AudioCodec::Aac, None)),
            Container::Wav => (None, audio(AudioCodec::Pcm, Some(24))),
            Container::Flac => (None, audio(AudioCodec::Flac, Some(24))),
        }
    }
}

impl fmt::Display for Container {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    #[default]
    H264,
    Hevc,
    Prores,
    Dnxhr,
    Vp9,
    Av1,
}

impl VideoCodec {
    pub const ALL: [VideoCodec; 6] =
        [VideoCodec::H264, VideoCodec::Hevc, VideoCodec::Prores, VideoCodec::Dnxhr, VideoCodec::Vp9, VideoCodec::Av1];

    pub fn name(self) -> &'static str {
        match self {
            VideoCodec::H264 => "h264",
            VideoCodec::Hevc => "hevc",
            VideoCodec::Prores => "prores",
            VideoCodec::Dnxhr => "dnxhr",
            VideoCodec::Vp9 => "vp9",
            VideoCodec::Av1 => "av1",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            VideoCodec::H264 => "H.264",
            VideoCodec::Hevc => "HEVC",
            VideoCodec::Prores => "ProRes",
            VideoCodec::Dnxhr => "DNxHR",
            VideoCodec::Vp9 => "VP9",
            VideoCodec::Av1 => "AV1",
        }
    }

    pub fn parse(s: &str) -> Option<VideoCodec> {
        let s = s.trim().to_ascii_lowercase().replace(['.', '-', ' '], "");
        Some(match s.as_str() {
            "h264" | "avc" => VideoCodec::H264,
            "hevc" | "h265" => VideoCodec::Hevc,
            "prores" => VideoCodec::Prores,
            "dnxhr" | "dnxhd" => VideoCodec::Dnxhr,
            "vp9" => VideoCodec::Vp9,
            "av1" => VideoCodec::Av1,
            _ => return None,
        })
    }

    /// Profile names accepted for this codec, default first.
    pub fn profiles(self) -> &'static [&'static str] {
        match self {
            VideoCodec::H264 => &["high"],
            VideoCodec::Hevc => &["main", "main10"],
            VideoCodec::Prores => &["hq", "proxy", "lt", "standard", "4444"],
            VideoCodec::Dnxhr => &["hq", "lb", "sq", "hqx", "444"],
            VideoCodec::Vp9 | VideoCodec::Av1 => &[],
        }
    }
}

impl fmt::Display for VideoCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Picture quality for codecs whose size is not fixed by the profile (H.264, HEVC, VP9, AV1). ProRes and DNxHR
/// ignore it: their profile sets the data rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Quality {
    #[serde(rename = "very low", alias = "very-low", alias = "veryLow", alias = "lowest")]
    VeryLow,
    #[serde(rename = "low")]
    Low,
    #[default]
    #[serde(rename = "medium")]
    Medium,
    #[serde(rename = "high")]
    High,
    #[serde(rename = "very high", alias = "very-high", alias = "veryHigh", alias = "highest")]
    VeryHigh,
}

impl Quality {
    pub fn parse(s: &str) -> Option<Quality> {
        Some(match s.trim().to_ascii_lowercase().replace(['-', '_'], " ").as_str() {
            "very low" | "verylow" | "lowest" => Quality::VeryLow,
            "low" => Quality::Low,
            "medium" | "normal" | "default" => Quality::Medium,
            "high" => Quality::High,
            "very high" | "veryhigh" | "highest" => Quality::VeryHigh,
            _ => return None,
        })
    }

    /// Bits per pixel per frame for an H.264-class encoder (HEVC and AV1 use 60 % of it).
    pub fn bits_per_pixel(self) -> f64 {
        match self {
            Quality::VeryLow => 0.03,
            Quality::Low => 0.06,
            Quality::Medium => 0.1,
            Quality::High => 0.15,
            Quality::VeryHigh => 0.25,
        }
    }

    /// The constant-quality level for VP9 (lower is better).
    pub fn vp9_crf(self) -> u32 {
        match self {
            Quality::VeryLow => 45,
            Quality::Low => 38,
            Quality::Medium => 33,
            Quality::High => 28,
            Quality::VeryHigh => 22,
        }
    }

    /// AAC/Opus bit rate in bits per second.
    pub fn audio_bitrate(self, codec: AudioCodec) -> u32 {
        let aac = match self {
            Quality::VeryLow => 96_000,
            Quality::Low => 128_000,
            Quality::Medium => 192_000,
            Quality::High => 256_000,
            Quality::VeryHigh => 320_000,
        };
        if codec == AudioCodec::Opus { aac * 2 / 3 } else { aac }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioCodec {
    #[default]
    Aac,
    /// Uncompressed PCM, 16-bit or 24-bit.
    Pcm,
    Flac,
    Opus,
}

impl AudioCodec {
    pub fn name(self) -> &'static str {
        match self {
            AudioCodec::Aac => "aac",
            AudioCodec::Pcm => "pcm",
            AudioCodec::Flac => "flac",
            AudioCodec::Opus => "opus",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AudioCodec::Aac => "AAC",
            AudioCodec::Pcm => "PCM",
            AudioCodec::Flac => "FLAC",
            AudioCodec::Opus => "Opus",
        }
    }

    pub fn parse(s: &str) -> Option<AudioCodec> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "aac" => AudioCodec::Aac,
            "pcm" | "lpcm" | "wav" => AudioCodec::Pcm,
            "flac" => AudioCodec::Flac,
            "opus" => AudioCodec::Opus,
            _ => return None,
        })
    }

    /// Lossless codecs take a bit depth; lossy ones a bit rate.
    pub fn lossless(self) -> bool {
        matches!(self, AudioCodec::Pcm | AudioCodec::Flac)
    }
}

impl fmt::Display for AudioCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// HDR transfer function to tag the output with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HdrTransfer {
    /// SMPTE ST 2084.
    Pq,
    /// ARIB STD-B67 (hybrid log-gamma).
    Hlg,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VideoSettings {
    pub codec: VideoCodec,
    /// See [`VideoCodec::profiles`]; `None` = the codec's default (ProRes and DNxHR: HQ).
    pub profile: Option<String>,
    /// 8, 10 or 12; `None` = the codec's (or profile's) default.
    pub bit_depth: Option<u8>,
    pub quality: Quality,
    /// Keep the composite's transparency (ProRes 4444 and VP9 in WebM only).
    pub alpha: bool,
    /// Video bit rate in bits per second, overriding `quality` (H.264, HEVC, AV1).
    pub bitrate: Option<u64>,
}

impl Default for VideoSettings {
    fn default() -> Self {
        VideoSettings {
            codec: VideoCodec::H264,
            profile: None,
            bit_depth: None,
            quality: Quality::Medium,
            alpha: false,
            bitrate: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioSettings {
    pub codec: AudioCodec,
    /// Samples per second (48 000; the mixer's rate).
    pub sample_rate: u32,
    /// PCM: 16 or 24; FLAC: 16 or 24. `None` = 24 for PCM and FLAC.
    pub bit_depth: Option<u8>,
    /// AAC and Opus bit rate in bits per second; `None` follows the video quality (192 kb/s AAC at medium).
    pub bitrate: Option<u32>,
}

impl Default for AudioSettings {
    fn default() -> Self {
        AudioSettings { codec: AudioCodec::Aac, sample_rate: crate::audio::RATE, bit_depth: None, bitrate: None }
    }
}

/// The output frame size: a height (the width follows the aspect ratio) or the timeline's own size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "NumberOrMatch", into = "NumberOrMatch")]
pub enum Size {
    Height(u32),
    MatchTimeline,
}

/// Frames per second, or the timeline's rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "NumberOrMatch", into = "NumberOrMatch")]
pub enum Fps {
    Rate(u32),
    MatchTimeline,
}

/// Wire form of [`Size`] and [`Fps`]: a number, or the word `"match"`.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum NumberOrMatch {
    Number(u32),
    Word(MatchWord),
}

#[derive(Serialize, Deserialize)]
enum MatchWord {
    #[serde(rename = "match", alias = "timeline", alias = "match timeline")]
    Match,
}

impl From<NumberOrMatch> for Size {
    fn from(v: NumberOrMatch) -> Self {
        match v {
            NumberOrMatch::Number(h) => Size::Height(h),
            NumberOrMatch::Word(_) => Size::MatchTimeline,
        }
    }
}

impl From<Size> for NumberOrMatch {
    fn from(v: Size) -> Self {
        match v {
            Size::Height(h) => NumberOrMatch::Number(h),
            Size::MatchTimeline => NumberOrMatch::Word(MatchWord::Match),
        }
    }
}

impl From<NumberOrMatch> for Fps {
    fn from(v: NumberOrMatch) -> Self {
        match v {
            NumberOrMatch::Number(n) => Fps::Rate(n),
            NumberOrMatch::Word(_) => Fps::MatchTimeline,
        }
    }
}

impl From<Fps> for NumberOrMatch {
    fn from(v: Fps) -> Self {
        match v {
            Fps::Rate(n) => NumberOrMatch::Number(n),
            Fps::MatchTimeline => NumberOrMatch::Word(MatchWord::Match),
        }
    }
}

impl Size {
    /// Parses `1080`, `1080p`, `720p`, `4k`, `2160` or `match`.
    pub fn parse(s: &str) -> Option<Size> {
        let s = s.trim().to_ascii_lowercase();
        match s.as_str() {
            "match" | "timeline" | "match timeline" => Some(Size::MatchTimeline),
            "4k" | "uhd" => Some(Size::Height(2160)),
            _ => s.trim_end_matches('p').parse().ok().map(Size::Height),
        }
    }
}

/// What to export. See the module documentation; [`ExportSettings::validate`] says whether it can be written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExportSettings {
    pub container: Container,
    /// `None`: audio only. When both `video` and `audio` are missing, the container's usual pair is used.
    #[serde(default)]
    pub video: Option<VideoSettings>,
    /// `None`: no sound.
    #[serde(default)]
    pub audio: Option<AudioSettings>,
    /// Start and end in seconds; `None` = the whole timeline.
    pub range: Option<[f64; 2]>,
    pub size: Size,
    pub fps: Fps,
    /// Tag the output as HDR (needs an HDR timeline).
    pub hdr: Option<HdrTransfer>,
    /// Loudness target of the mix, integrated LUFS (-14, -16, -23 or -24); `None` = off. Export measures the mix
    /// first and applies one static gain before the true-peak limiter (add-audio-mixing-and-monitoring, D8).
    #[serde(default)]
    pub loudness: Option<f64>,
    /// Video encoder to force (testing); `None` picks automatically.
    pub encoder: Option<String>,
    /// Compositing backend: `auto`, `cpu` or `gpu`; `None` is the program's default (design D9).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
}

impl Default for ExportSettings {
    /// H.264 + AAC in MP4, 1080 pixels high, 30 frames per second, medium quality (the export of change 6).
    fn default() -> Self {
        let (video, audio) = Container::Mp4.defaults();
        ExportSettings {
            container: Container::Mp4,
            video,
            audio,
            range: None,
            size: Size::Height(1080),
            fps: Fps::Rate(30),
            hdr: None,
            loudness: None,
            encoder: None,
            backend: None,
        }
    }
}

/// Settings that cannot be exported: the message names the conflicting fields, which are also listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingsError {
    pub fields: Vec<String>,
    pub message: String,
}

impl SettingsError {
    pub fn new(fields: &[&str], message: impl Into<String>) -> Self {
        SettingsError { fields: fields.iter().map(|f| f.to_string()).collect(), message: message.into() }
    }
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SettingsError {}

/// ProRes profiles in FFmpeg's numbering (`prores_ks` and `prores_videotoolbox` `profile` option).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProresProfile {
    Proxy,
    Lt,
    Standard,
    Hq,
    P4444,
}

impl ProresProfile {
    pub fn number(self) -> i32 {
        match self {
            ProresProfile::Proxy => 0,
            ProresProfile::Lt => 1,
            ProresProfile::Standard => 2,
            ProresProfile::Hq => 3,
            ProresProfile::P4444 => 4,
        }
    }

    /// The FFmpeg `profile` option value.
    pub fn option(self) -> &'static str {
        match self {
            ProresProfile::Proxy => "proxy",
            ProresProfile::Lt => "lt",
            ProresProfile::Standard => "standard",
            ProresProfile::Hq => "hq",
            ProresProfile::P4444 => "4444",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ProresProfile::Proxy => "ProRes 422 Proxy",
            ProresProfile::Lt => "ProRes 422 LT",
            ProresProfile::Standard => "ProRes 422",
            ProresProfile::Hq => "ProRes 422 HQ",
            ProresProfile::P4444 => "ProRes 4444",
        }
    }
}

/// DNxHR profiles (`dnxhd` encoder `profile` option).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnxhrProfile {
    Lb,
    Sq,
    Hq,
    Hqx,
    P444,
}

impl DnxhrProfile {
    pub fn option(self) -> &'static str {
        match self {
            DnxhrProfile::Lb => "dnxhr_lb",
            DnxhrProfile::Sq => "dnxhr_sq",
            DnxhrProfile::Hq => "dnxhr_hq",
            DnxhrProfile::Hqx => "dnxhr_hqx",
            DnxhrProfile::P444 => "dnxhr_444",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DnxhrProfile::Lb => "DNxHR LB",
            DnxhrProfile::Sq => "DNxHR SQ",
            DnxhrProfile::Hq => "DNxHR HQ",
            DnxhrProfile::Hqx => "DNxHR HQX",
            DnxhrProfile::P444 => "DNxHR 444",
        }
    }
}

/// A codec profile after validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// The codec's only (or bit-depth-derived) profile: H.264 High, HEVC Main/Main 10, VP9, AV1.
    Default,
    Prores(ProresProfile),
    Dnxhr(DnxhrProfile),
}

/// Chroma subsampling of the encoded picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chroma {
    C420,
    C422,
    C444,
}

impl Chroma {
    pub fn label(self) -> &'static str {
        match self {
            Chroma::C420 => "4:2:0",
            Chroma::C422 => "4:2:2",
            Chroma::C444 => "4:4:4",
        }
    }
}

/// Validated video settings.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoPlan {
    pub codec: VideoCodec,
    pub profile: Profile,
    pub bit_depth: u8,
    pub chroma: Chroma,
    pub quality: Quality,
    pub alpha: bool,
    pub bitrate: Option<u64>,
}

impl VideoPlan {
    /// "ProRes 422 HQ", "HEVC 10-bit", "VP9", …
    pub fn label(&self) -> String {
        match self.profile {
            Profile::Prores(p) => p.label().to_string(),
            Profile::Dnxhr(p) => p.label().to_string(),
            Profile::Default if self.bit_depth > 8 => format!("{} {}-bit", self.codec.label(), self.bit_depth),
            Profile::Default => self.codec.label().to_string(),
        }
    }
}

/// Validated audio settings.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioPlan {
    pub codec: AudioCodec,
    pub sample_rate: u32,
    /// 16 or 24 for PCM and FLAC; 0 for lossy codecs.
    pub bit_depth: u8,
    /// Bits per second for lossy codecs; 0 for lossless ones.
    pub bitrate: u32,
}

/// Settings that passed [`ExportSettings::validate`]. Size and frame rate are resolved against the document later.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub container: Container,
    pub video: Option<VideoPlan>,
    pub audio: Option<AudioPlan>,
    /// Start and end in microseconds.
    pub range: Option<(Micros, Micros)>,
    pub size: Size,
    pub fps: Fps,
    pub hdr: Option<HdrTransfer>,
    /// Loudness target, integrated LUFS.
    pub loudness: Option<f64>,
}

/// The loudness targets export offers (integrated LUFS): streaming, podcasts, EBU R128 broadcast, ATSC A/85.
pub const LOUDNESS_TARGETS: [f64; 4] = [-14.0, -16.0, -23.0, -24.0];

/// The codecs that can carry alpha, for messages.
const ALPHA_CODECS: &str = "ProRes 4444 (MOV) and VP9 (WebM)";

fn secs(us: Micros) -> String {
    let s = us as f64 / 1e6;
    if (s - s.round()).abs() < 1e-9 { format!("{s:.0} s") } else { format!("{s:.3} s") }
}

impl ExportSettings {
    /// Settings for a container with its usual codecs (see [`Container::defaults`]).
    pub fn for_container(container: Container) -> Self {
        let (video, audio) = container.defaults();
        ExportSettings { container, video, audio, ..ExportSettings::default() }
    }

    /// H.264 + AAC in MP4 at a height and frame rate (the settings of the first export version).
    pub fn h264(height: u32, fps: u32) -> Self {
        ExportSettings { size: Size::Height(height), fps: Fps::Rate(fps), ..ExportSettings::default() }
    }

    /// Fills in the container's usual video and audio when the settings name neither.
    pub fn with_defaults(mut self) -> Self {
        if self.video.is_none() && self.audio.is_none() {
            let (v, a) = self.container.defaults();
            self.video = v;
            self.audio = a;
        }
        self
    }

    /// Checks the settings without rendering. `duration` is the timeline's length when known (the range must lie
    /// inside it).
    pub fn validate(&self, duration: Option<Micros>) -> Result<Plan, SettingsError> {
        let s = self.clone().with_defaults();
        let c = s.container;
        if s.video.is_none() && s.audio.is_none() {
            return Err(SettingsError::new(&["video", "audio"], "the export has neither video nor audio"));
        }
        let video = match &s.video {
            Some(v) => Some(validate_video(c, v, s.hdr)?),
            None => None,
        };
        let quality = s.video.as_ref().map(|v| v.quality).unwrap_or_default();
        let audio = match &s.audio {
            Some(a) => Some(validate_audio(c, a, quality)?),
            None => None,
        };
        if c.audio_only() && audio.is_none() {
            return Err(SettingsError::new(
                &["container", "audio"],
                format!("container {c} holds sound only, but the settings have no audio"),
            ));
        }
        if let Size::Height(h) = s.size
            && !(16..=4320).contains(&h)
        {
            return Err(SettingsError::new(&["size"], format!("size must be 16–4320 pixels high, not {h}")));
        }
        if let Fps::Rate(f) = s.fps
            && !(1..=120).contains(&f)
        {
            return Err(SettingsError::new(&["fps"], format!("frame rate must be 1–120, not {f}")));
        }
        if s.hdr.is_some() && video.is_none() {
            return Err(SettingsError::new(&["hdr", "video"], "hdr needs video"));
        }
        if let Some(l) = s.loudness {
            if !LOUDNESS_TARGETS.contains(&l) {
                return Err(SettingsError::new(
                    &["loudness"],
                    format!("loudness must be off or one of -14, -16, -23, -24 LUFS, not {l}"),
                ));
            }
            if audio.is_none() {
                return Err(SettingsError::new(&["loudness", "audio"], "a loudness target needs audio"));
            }
        }
        let range = match s.range {
            Some(r) => Some(validate_range(r, duration)?),
            None => None,
        };
        Ok(Plan { container: c, video, audio, range, size: s.size, fps: s.fps, hdr: s.hdr, loudness: s.loudness })
    }
}

fn validate_range(r: [f64; 2], duration: Option<Micros>) -> Result<(Micros, Micros), SettingsError> {
    let bad = || SettingsError::new(&["range"], format!("range {} – {} is not a pair of times in seconds", r[0], r[1]));
    let (a, b) = (secs_to_micros(r[0]).ok_or_else(bad)?, secs_to_micros(r[1]).ok_or_else(bad)?);
    if a < 0 {
        return Err(SettingsError::new(&["range"], format!("range start ({}) is before the timeline starts", secs(a))));
    }
    if b <= a {
        return Err(SettingsError::new(
            &["range"],
            format!("range end ({}) must be after its start ({})", secs(b), secs(a)),
        ));
    }
    if let Some(d) = duration
        && b > d
    {
        return Err(SettingsError::new(
            &["range"],
            format!("range {} – {} is outside the timeline, which ends at {}", secs(a), secs(b), secs(d)),
        ));
    }
    Ok((a, b))
}

fn validate_video(c: Container, v: &VideoSettings, hdr: Option<HdrTransfer>) -> Result<VideoPlan, SettingsError> {
    let codec = v.codec;
    if c.audio_only() {
        return Err(SettingsError::new(
            &["container", "video"],
            format!("container {c} holds sound only; remove the video settings or pick mp4, mov, mkv or webm"),
        ));
    }
    if !c.video_codecs().contains(&codec) {
        let fits: Vec<&str> =
            Container::ALL.iter().filter(|k| k.video_codecs().contains(&codec)).map(|k| k.name()).collect();
        return Err(SettingsError::new(
            &["video.codec", "container"],
            format!("video codec {} cannot be written to container {c}; use {}", codec.label(), fits.join(" or ")),
        ));
    }
    let names = codec.profiles();
    let profile_name = match v.profile.as_deref().map(|p| p.trim().to_ascii_lowercase()) {
        None => names.first().map(|n| n.to_string()),
        Some(p) => {
            let p = match (codec, p.as_str()) {
                (VideoCodec::Prores, "422") => "standard".to_string(),
                (VideoCodec::Prores, "422hq" | "422 hq") => "hq".to_string(),
                (VideoCodec::Prores, "422lt" | "422 lt") => "lt".to_string(),
                (VideoCodec::Prores, "422proxy" | "422 proxy") => "proxy".to_string(),
                (VideoCodec::Dnxhr, other) => other.trim_start_matches("dnxhr").trim_start_matches(['_', ' ']).into(),
                (_, other) => other.to_string(),
            };
            if !names.contains(&p.as_str()) {
                let what = if names.is_empty() {
                    format!("{} has no profiles", codec.label())
                } else {
                    format!("{} profiles are {}", codec.label(), names.join(", "))
                };
                return Err(SettingsError::new(
                    &["video.profile", "video.codec"],
                    format!("video profile {p:?} does not exist; {what}"),
                ));
            }
            Some(p)
        }
    };
    let profile = match (codec, profile_name.as_deref()) {
        (VideoCodec::Prores, Some(p)) => Profile::Prores(match p {
            "proxy" => ProresProfile::Proxy,
            "lt" => ProresProfile::Lt,
            "standard" => ProresProfile::Standard,
            "4444" => ProresProfile::P4444,
            _ => ProresProfile::Hq,
        }),
        (VideoCodec::Dnxhr, Some(p)) => Profile::Dnxhr(match p {
            "lb" => DnxhrProfile::Lb,
            "sq" => DnxhrProfile::Sq,
            "hqx" => DnxhrProfile::Hqx,
            "444" => DnxhrProfile::P444,
            _ => DnxhrProfile::Hq,
        }),
        _ => Profile::Default,
    };
    // Bit depths each codec (and profile) can be written in; the first is the default.
    let depths: &[u8] = match (codec, profile, profile_name.as_deref()) {
        (VideoCodec::H264, ..) => &[8],
        (VideoCodec::Hevc, _, Some("main10")) => &[10],
        (VideoCodec::Hevc, _, Some("main")) if v.profile.is_some() => &[8],
        (VideoCodec::Hevc, ..) => &[8, 10],
        (VideoCodec::Prores, ..) => &[10],
        (VideoCodec::Dnxhr, Profile::Dnxhr(DnxhrProfile::Hqx | DnxhrProfile::P444), _) => &[10],
        (VideoCodec::Dnxhr, ..) => &[8],
        (VideoCodec::Vp9, ..) => &[8, 10],
        (VideoCodec::Av1, ..) => &[8, 10],
    };
    let bit_depth = v.bit_depth.unwrap_or(if hdr.is_some() { *depths.last().unwrap_or(&8) } else { depths[0] });
    if !depths.contains(&bit_depth) {
        let allowed: Vec<String> = depths.iter().map(|d| format!("{d}-bit")).collect();
        let what = match profile_name.as_deref() {
            Some(p) if names.len() > 1 => format!("{} {p}", codec.label()),
            _ => codec.label().to_string(),
        };
        return Err(SettingsError::new(
            &["video.bitDepth", "video.codec"],
            format!("{what} is written in {} only, not {bit_depth}-bit", allowed.join(" or ")),
        ));
    }
    if v.alpha {
        let ok = match codec {
            VideoCodec::Prores => profile == Profile::Prores(ProresProfile::P4444),
            VideoCodec::Vp9 => c == Container::Webm && bit_depth == 8,
            _ => false,
        };
        if !ok {
            let fields: &[&str] = if codec == VideoCodec::Prores {
                &["video.alpha", "video.profile"]
            } else if codec == VideoCodec::Vp9 {
                &["video.alpha", "container", "video.bitDepth"]
            } else {
                &["video.alpha", "video.codec"]
            };
            return Err(SettingsError::new(
                fields,
                format!(
                    "{} {}cannot carry alpha; alpha is supported by {ALPHA_CODECS} only",
                    codec.label(),
                    match codec {
                        VideoCodec::Prores => "422 ",
                        VideoCodec::Vp9 if c != Container::Webm => "outside WebM ",
                        VideoCodec::Vp9 => "at 10-bit ",
                        _ => "",
                    }
                ),
            ));
        }
    }
    if let Some(t) = hdr {
        let ok = (codec == VideoCodec::Hevc && bit_depth == 10) || codec == VideoCodec::Prores;
        if !ok {
            return Err(SettingsError::new(
                &["hdr", "video.codec", "video.bitDepth"],
                format!(
                    "{} HDR needs HEVC 10-bit or ProRes, not {} {bit_depth}-bit",
                    match t {
                        HdrTransfer::Pq => "PQ",
                        HdrTransfer::Hlg => "HLG",
                    },
                    codec.label()
                ),
            ));
        }
    }
    let chroma = match profile {
        Profile::Prores(ProresProfile::P4444) | Profile::Dnxhr(DnxhrProfile::P444) => Chroma::C444,
        Profile::Prores(_) | Profile::Dnxhr(_) => Chroma::C422,
        Profile::Default => Chroma::C420,
    };
    if let Some(b) = v.bitrate
        && !(100_000..=2_000_000_000).contains(&b)
    {
        return Err(SettingsError::new(&["video.bitrate"], format!("video bit rate {b} is out of range")));
    }
    Ok(VideoPlan { codec, profile, bit_depth, chroma, quality: v.quality, alpha: v.alpha, bitrate: v.bitrate })
}

fn validate_audio(c: Container, a: &AudioSettings, quality: Quality) -> Result<AudioPlan, SettingsError> {
    let codec = a.codec;
    if !c.audio_codecs().contains(&codec) {
        let allowed: Vec<&str> = c.audio_codecs().iter().map(|k| k.name()).collect();
        return Err(SettingsError::new(
            &["audio.codec", "container"],
            format!(
                "audio codec {} cannot be written to container {c}; {c} takes {}",
                codec.label(),
                allowed.join(", ")
            ),
        ));
    }
    if a.sample_rate != crate::audio::RATE {
        return Err(SettingsError::new(
            &["audio.sampleRate"],
            format!("audio is exported at {} Hz only, not {} Hz", crate::audio::RATE, a.sample_rate),
        ));
    }
    let bit_depth = if codec.lossless() {
        let d = a.bit_depth.unwrap_or(24);
        if d != 16 && d != 24 {
            return Err(SettingsError::new(
                &["audio.bitDepth"],
                format!("{} audio is 16-bit or 24-bit, not {d}-bit", codec.label()),
            ));
        }
        d
    } else {
        if let Some(d) = a.bit_depth {
            return Err(SettingsError::new(
                &["audio.bitDepth", "audio.codec"],
                format!("{} has no bit depth (got {d}); set audio.bitrate instead", codec.label()),
            ));
        }
        0
    };
    let bitrate = if codec.lossless() {
        if a.bitrate.is_some() {
            return Err(SettingsError::new(
                &["audio.bitrate", "audio.codec"],
                format!("{} is lossless and has no bit rate; set audio.bitDepth instead", codec.label()),
            ));
        }
        0
    } else {
        let b = a.bitrate.unwrap_or_else(|| quality.audio_bitrate(codec));
        if !(32_000..=512_000).contains(&b) {
            return Err(SettingsError::new(&["audio.bitrate"], format!("audio bit rate must be 32–512 kb/s, not {b}")));
        }
        b
    };
    Ok(AudioPlan { codec, sample_rate: a.sample_rate, bit_depth, bitrate })
}

/// A named settings value shared by the app, `ove export --preset` and MCP `export_video`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub name: String,
    /// One line for menus.
    pub about: String,
    pub settings: ExportSettings,
}

/// The preset table (`presets.json`), in menu order.
pub fn presets() -> &'static [Preset] {
    static P: OnceLock<Vec<Preset>> = OnceLock::new();
    P.get_or_init(|| match serde_json::from_str(include_str!("presets.json")) {
        Ok(p) => p,
        Err(e) => {
            // The table is part of the source and checked by a test; this cannot happen in a build that passed it.
            tracing::error!("export presets are invalid: {e}");
            Vec::new()
        }
    })
}

/// A preset by name, ignoring case and spacing differences.
pub fn preset(name: &str) -> Option<&'static Preset> {
    let key = |s: &str| s.to_ascii_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>();
    let k = key(name);
    presets().iter().find(|p| key(&p.name) == k)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(codec: VideoCodec) -> VideoSettings {
        VideoSettings { codec, ..VideoSettings::default() }
    }

    fn with(container: Container, v: Option<VideoSettings>, a: Option<AudioSettings>) -> ExportSettings {
        ExportSettings { container, video: v, audio: a, ..ExportSettings::default() }
    }

    fn refused(s: &ExportSettings) -> SettingsError {
        match s.validate(Some(60_000_000)) {
            Ok(p) => panic!("accepted: {p:?}"),
            Err(e) => e,
        }
    }

    #[test]
    fn every_preset_is_valid_and_named_uniquely() {
        let all = presets();
        assert!(all.len() >= 10, "{} presets", all.len());
        let mut names = std::collections::HashSet::new();
        for p in all {
            assert!(names.insert(p.name.to_lowercase()), "duplicate preset {}", p.name);
            let plan = p.settings.validate(None).unwrap_or_else(|e| panic!("{}: {e}", p.name));
            assert!(!p.about.is_empty());
            eprintln!("{:28} {:5} {:?}", p.name, plan.container.name(), plan.video.as_ref().map(VideoPlan::label));
        }
    }

    #[test]
    fn prores_master_preset_expands_to_mov_prores_hq_and_24_bit_pcm_at_timeline_size() {
        let p = preset("ProRes 422 HQ master").unwrap();
        let plan = p.settings.validate(None).unwrap();
        assert_eq!(plan.container, Container::Mov);
        let v = plan.video.unwrap();
        assert_eq!(
            (v.codec, v.profile, v.bit_depth, v.chroma),
            (VideoCodec::Prores, Profile::Prores(ProresProfile::Hq), 10, Chroma::C422)
        );
        let a = plan.audio.unwrap();
        assert_eq!((a.codec, a.bit_depth), (AudioCodec::Pcm, 24));
        assert_eq!(plan.size, Size::MatchTimeline);
        assert_eq!(plan.fps, Fps::MatchTimeline);
        assert!(preset("prores 422 hq MASTER").is_some(), "lookup ignores case");
    }

    #[test]
    fn audio_presets_have_no_video() {
        for name in ["Audio WAV 24-bit", "Audio WAV 16-bit", "Audio M4A", "Audio FLAC"] {
            let plan = preset(name).unwrap_or_else(|| panic!("{name}")).settings.validate(None).unwrap();
            assert!(plan.video.is_none() && plan.audio.is_some() && plan.container.audio_only(), "{name}");
        }
    }

    #[test]
    fn container_alone_means_its_usual_codecs() {
        let s: ExportSettings = serde_json::from_str(r#"{"container": "wav"}"#).unwrap();
        let plan = s.validate(None).unwrap();
        assert!(plan.video.is_none());
        assert_eq!(
            plan.audio.unwrap(),
            AudioPlan { codec: AudioCodec::Pcm, sample_rate: 48_000, bit_depth: 24, bitrate: 0 }
        );
        let s: ExportSettings = serde_json::from_str(r#"{"container": "webm"}"#).unwrap();
        let plan = s.validate(None).unwrap();
        assert_eq!(plan.video.unwrap().codec, VideoCodec::Vp9);
        assert_eq!(plan.audio.unwrap().codec, AudioCodec::Opus);
    }

    #[test]
    fn json_round_trip_with_match_words() {
        let s: ExportSettings = serde_json::from_str(
            r#"{"container":"mov","video":{"codec":"prores","profile":"4444","alpha":true,"quality":"very high"},
                "audio":{"codec":"pcm","bitDepth":16},"size":"match","fps":25,"range":[1.5,4]}"#,
        )
        .unwrap();
        assert_eq!(s.size, Size::MatchTimeline);
        assert_eq!(s.fps, Fps::Rate(25));
        assert_eq!(s.video.as_ref().unwrap().quality, Quality::VeryHigh);
        let back: ExportSettings = serde_json::from_value(serde_json::to_value(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        assert_eq!(serde_json::to_value(Size::MatchTimeline).unwrap(), serde_json::json!("match"));
        assert_eq!(serde_json::to_value(Size::Height(720)).unwrap(), serde_json::json!(720));
        let plan = s.validate(Some(10_000_000)).unwrap();
        assert_eq!(plan.range, Some((1_500_000, 4_000_000)));
        assert!(plan.video.unwrap().alpha);
    }

    #[test]
    fn prores_in_mp4_is_refused_naming_codec_and_container() {
        let e = refused(&with(Container::Mp4, Some(video(VideoCodec::Prores)), None));
        assert_eq!(e.fields, ["video.codec", "container"]);
        assert!(e.message.contains("ProRes") && e.message.contains("mp4") && e.message.contains("mov"), "{e}");
    }

    #[test]
    fn codec_container_matrix() {
        for c in Container::ALL {
            for codec in VideoCodec::ALL {
                let r = with(c, Some(video(codec)), None).validate(None);
                assert_eq!(r.is_ok(), c.video_codecs().contains(&codec), "{c} {codec}: {r:?}");
            }
        }
        let e = refused(&with(Container::Webm, Some(video(VideoCodec::Vp9)), Some(AudioSettings::default())));
        assert_eq!(e.fields, ["audio.codec", "container"]);
        let e = refused(&with(Container::Wav, Some(video(VideoCodec::H264)), Some(AudioSettings::default())));
        assert_eq!(e.fields, ["container", "video"]);
        let e = refused(&with(Container::Wav, None, Some(AudioSettings::default())));
        assert!(e.message.contains("AAC") && e.message.contains("wav"), "{e}");
    }

    #[test]
    fn alpha_is_refused_outside_prores_4444_and_vp9_webm() {
        let alpha = |codec, profile: Option<&str>| VideoSettings {
            codec,
            profile: profile.map(str::to_string),
            alpha: true,
            ..VideoSettings::default()
        };
        let e = refused(&with(Container::Mp4, Some(alpha(VideoCodec::H264, None)), None));
        assert_eq!(e.fields, ["video.alpha", "video.codec"]);
        assert!(e.message.contains("ProRes 4444") && e.message.contains("VP9"), "{e}");
        let e = refused(&with(Container::Mov, Some(alpha(VideoCodec::Prores, Some("hq"))), None));
        assert_eq!(e.fields, ["video.alpha", "video.profile"]);
        let e = refused(&with(Container::Mkv, Some(alpha(VideoCodec::Vp9, None)), None));
        assert!(e.fields.contains(&"container".to_string()));
        with(Container::Mov, Some(alpha(VideoCodec::Prores, Some("4444"))), None).validate(None).unwrap();
        with(Container::Webm, Some(alpha(VideoCodec::Vp9, None)), None).validate(None).unwrap();
    }

    #[test]
    fn bit_depths_follow_codec_and_profile() {
        let depth = |codec, profile: Option<&str>, d| VideoSettings {
            codec,
            profile: profile.map(str::to_string),
            bit_depth: d,
            ..VideoSettings::default()
        };
        let e = refused(&with(Container::Mp4, Some(depth(VideoCodec::H264, None, Some(10))), None));
        assert_eq!(e.fields, ["video.bitDepth", "video.codec"]);
        let ok = |c, v: VideoSettings| with(c, Some(v), None).validate(None).unwrap().video.unwrap();
        assert_eq!(ok(Container::Mp4, depth(VideoCodec::Hevc, None, Some(10))).bit_depth, 10);
        assert_eq!(ok(Container::Mp4, depth(VideoCodec::Hevc, Some("main10"), None)).bit_depth, 10);
        refused(&with(Container::Mp4, Some(depth(VideoCodec::Hevc, Some("main10"), Some(8))), None));
        assert_eq!(ok(Container::Mov, depth(VideoCodec::Dnxhr, Some("hqx"), None)).bit_depth, 10);
        assert_eq!(ok(Container::Mov, depth(VideoCodec::Dnxhr, Some("dnxhr_sq"), None)).bit_depth, 8);
        refused(&with(Container::Mov, Some(depth(VideoCodec::Dnxhr, Some("lb"), Some(10))), None));
        refused(&with(Container::Mov, Some(depth(VideoCodec::Prores, Some("hq"), Some(12))), None));
        let e = refused(&with(Container::Mov, Some(depth(VideoCodec::Prores, Some("ultra"), None)), None));
        assert_eq!(e.fields, ["video.profile", "video.codec"]);
        assert_eq!(
            ok(Container::Mov, depth(VideoCodec::Prores, Some("422"), None)).profile,
            Profile::Prores(ProresProfile::Standard)
        );
    }

    #[test]
    fn ranges_must_be_ordered_and_inside_the_timeline() {
        let r = |a, b| ExportSettings { range: Some([a, b]), ..ExportSettings::default() };
        let e = refused(&r(20.0, 10.0));
        assert_eq!(e.fields, ["range"]);
        assert!(e.message.contains("after its start"), "{e}");
        refused(&r(5.0, 5.0));
        refused(&r(-1.0, 5.0));
        let e = refused(&r(50.0, 70.0));
        assert!(e.message.contains("outside the timeline"), "{e}");
        assert_eq!(r(10.0, 20.0).validate(Some(60_000_000)).unwrap().range, Some((10_000_000, 20_000_000)));
    }

    #[test]
    fn audio_rules() {
        let a = |codec, d, b| Some(AudioSettings { codec, bit_depth: d, bitrate: b, ..AudioSettings::default() });
        let e = refused(&with(Container::Wav, None, a(AudioCodec::Pcm, Some(32), None)));
        assert_eq!(e.fields, ["audio.bitDepth"]);
        refused(&with(Container::M4a, None, a(AudioCodec::Aac, Some(16), None)));
        refused(&with(Container::Flac, None, a(AudioCodec::Flac, None, Some(128_000))));
        let e = refused(&with(
            Container::Wav,
            None,
            Some(AudioSettings { codec: AudioCodec::Pcm, sample_rate: 44_100, ..AudioSettings::default() }),
        ));
        assert_eq!(e.fields, ["audio.sampleRate"]);
        let plan = with(Container::Wav, None, a(AudioCodec::Pcm, Some(16), None)).validate(None).unwrap();
        assert_eq!(plan.audio.unwrap().bit_depth, 16);
        // Neither video nor audio: the container's usual pair.
        assert_eq!(with(Container::Wav, None, None).validate(None).unwrap().audio.unwrap().bit_depth, 24);
    }

    #[test]
    fn hdr_needs_a_10_bit_hevc_or_prores() {
        let s = |codec, d| ExportSettings {
            container: Container::Mov,
            video: Some(VideoSettings { codec, bit_depth: d, ..VideoSettings::default() }),
            hdr: Some(HdrTransfer::Pq),
            ..ExportSettings::default()
        };
        let e = refused(&s(VideoCodec::H264, None));
        assert!(e.fields.contains(&"hdr".to_string()));
        refused(&s(VideoCodec::Hevc, Some(8)));
        assert_eq!(s(VideoCodec::Hevc, None).validate(None).unwrap().video.unwrap().bit_depth, 10);
        s(VideoCodec::Prores, None).validate(None).unwrap();
    }

    #[test]
    fn size_and_rate_limits_and_parsing() {
        refused(&ExportSettings { size: Size::Height(8), ..ExportSettings::default() });
        refused(&ExportSettings { fps: Fps::Rate(0), ..ExportSettings::default() });
        assert_eq!(Size::parse("1080p"), Some(Size::Height(1080)));
        assert_eq!(Size::parse("4K"), Some(Size::Height(2160)));
        assert_eq!(Size::parse("match"), Some(Size::MatchTimeline));
        assert_eq!(Container::parse(".MOV"), Some(Container::Mov));
        assert_eq!(Quality::parse("very-high"), Some(Quality::VeryHigh));
        assert_eq!(VideoCodec::parse("H.265"), Some(VideoCodec::Hevc));
    }
}
