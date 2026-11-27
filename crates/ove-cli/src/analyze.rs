//! `ove audio …`: levels, waveform, silence, loudness (the registry's analysis tools; they need the `media` feature).

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
    /// Integrated loudness (LUFS), loudness range, sample and true peak.
    Loudness { project: String, media: String },
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
        AudioCmd::Loudness { project, media } => {
            ctx.tool("measure_loudness", json!({"project": project, "media": media}))
        }
    }
}
