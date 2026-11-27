//! `ove audio …`: levels, waveform, silence, loudness and duck (registry tools; they need the `media` feature).

use std::path::PathBuf;

use clap::Subcommand;
use serde_json::json;

use crate::common::{Ctx, Out};

#[derive(Subcommand)]
pub enum AudioCmd {
    /// Peak and RMS level (dBFS) per bucket across the file.
    Levels {
        project: String,
        media: String,
        #[arg(long, default_value_t = 100)]
        points: usize,
    },
    /// The waveform as a PNG.
    Waveform {
        project: String,
        media: String,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 1200)]
        width: u32,
        #[arg(long, default_value_t = 200)]
        height: u32,
    },
    /// Ranges quieter than --threshold for at least --min seconds.
    Silence {
        project: String,
        media: String,
        #[arg(long, default_value_t = -40.0, allow_negative_numbers = true)]
        threshold: f64,
        #[arg(long, default_value_t = 0.5)]
        min: f64,
    },
    /// Integrated loudness (LUFS), loudness range, sample and true peak of a media file, or of the timeline mix.
    Loudness {
        project: String,
        /// Media id, file name or path (omit with --timeline).
        media: Option<String>,
        /// Measure the timeline mix as it would export.
        #[arg(long, conflicts_with = "media")]
        timeline: bool,
    },
    /// Lower music under dialogue: volume keyframes on the music items, as one revision.
    Duck {
        project: String,
        /// Music item ids or track ids/names, comma-separated.
        #[arg(long)]
        music: String,
        /// Dialogue item ids or track ids/names, comma-separated.
        #[arg(long)]
        under: String,
        /// dB during speech (-60 to 0).
        #[arg(long, default_value_t = -12.0, allow_negative_numbers = true)]
        depth: f64,
        /// Seconds of ramp down before speech.
        #[arg(long, default_value_t = 0.2)]
        attack: f64,
        /// Seconds of ramp up after speech.
        #[arg(long, default_value_t = 0.4)]
        release: f64,
        /// Lines of speech closer than this many seconds merge.
        #[arg(long, default_value_t = 0.5)]
        hold: f64,
        /// Where speech comes from: transcript, level, or auto (transcript when cached).
        #[arg(long, default_value = "auto")]
        source: String,
        /// Multiply with existing volume keyframes instead of replacing them in the affected range.
        #[arg(long, conflicts_with = "replace")]
        merge: bool,
        /// Replace volume keyframes in the affected range (the default).
        #[arg(long)]
        replace: bool,
        /// Show the planned keyframes without changing anything.
        #[arg(long)]
        dry_run: bool,
    },
}

pub fn audio(ctx: &Ctx, cmd: &AudioCmd) -> Out {
    match cmd {
        AudioCmd::Levels { project, media, points } => {
            ctx.tool("audio_levels", json!({"project": project, "media": media, "points": points}))
        }
        AudioCmd::Waveform { project, media, out, width, height } => ctx.tool(
            "waveform_image",
            json!({"project": project, "media": media, "out": out, "width": width, "height": height}),
        ),
        AudioCmd::Silence { project, media, threshold, min } => ctx.tool(
            "detect_silence",
            json!({"project": project, "media": media, "thresholdDb": threshold, "minDuration": min}),
        ),
        AudioCmd::Loudness { project, media, timeline } => {
            ctx.tool("measure_loudness", json!({"project": project, "media": media, "timeline": timeline}))
        }
        AudioCmd::Duck { project, music, under, depth, attack, release, hold, source, merge, replace: _, dry_run } => {
            ctx.tool(
                "duck_audio",
                json!({"project": project, "music": music, "under": under, "depth": depth, "attack": attack,
                    "release": release, "hold": hold, "source": source,
                    "mode": if *merge { "merge" } else { "replace" }, "dryRun": dry_run}),
            )
        }
    }
}
