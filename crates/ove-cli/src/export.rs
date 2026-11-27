//! `ove export`: render the timeline (or a range of it) with the export settings model — a preset, or a format with
//! codec options. Calls the `export_video` registry tool, so the command line and MCP take the same settings.

use std::path::PathBuf;

use clap::{Args, Subcommand};
use serde_json::{Map, Value, json};

use crate::common::{Ctx, Fail, Out};

/// `ove export`: export a project, or look after the export queue.
#[derive(Args, Debug)]
#[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
pub struct ExportCmd {
    #[command(subcommand)]
    pub sub: Option<ExportSub>,
    #[command(flatten)]
    pub args: ExportArgs,
}

#[derive(Subcommand, Debug)]
pub enum ExportSub {
    /// The export jobs: queued, running and finished in the last day (--all: every one).
    Queue {
        #[arg(long)]
        all: bool,
    },
    /// One job's state and progress; --wait returns when it has finished.
    Status {
        id: String,
        #[arg(long)]
        wait: bool,
    },
    /// Cancel a queued or running job (a finished job's file is kept).
    Cancel { id: String },
    /// The export presets.
    Presets,
}

#[derive(Args, Debug)]
pub struct ExportArgs {
    /// Project name, id or bundle path.
    #[arg(required_unless_present = "worker")]
    pub project: Option<String>,
    /// Output file; its extension must match the format (.mp4, .mov, .mkv, .webm, .m4a, .wav, .flac).
    #[arg(required_unless_present_any = ["worker", "as_media"])]
    pub out: Option<PathBuf>,
    /// Queue the export and return its job id at once (`ove export status <id> --wait` follows it).
    #[arg(long)]
    pub detach: bool,
    /// Run queued exports until none is left (started by the app and `ove`).
    #[arg(long, hide = true)]
    pub worker: bool,
    /// Render --range into the project's media library instead of a file (DNxHR HQ or ProRes 422 HQ at the project's
    /// size; --small: H.264).
    #[arg(long, requires = "range")]
    pub as_media: bool,
    /// With --as-media: also replace what the range covers on --tracks with one clip of the new media (one undo step).
    #[arg(long, requires = "as_media")]
    pub replace: bool,
    /// With --as-media: the tracks to render and replace, comma-separated ids or names (default: all but audio).
    #[arg(long, requires = "as_media", value_delimiter = ',')]
    pub tracks: Vec<String>,
    /// With --as-media: the new media's name.
    #[arg(long, requires = "as_media")]
    pub name: Option<String>,
    /// With --as-media: H.264 in MP4 instead of an intermediate codec.
    #[arg(long, requires = "as_media")]
    pub small: bool,
    /// A named preset ("YouTube 1080p", "ProRes 422 HQ master", "Audio WAV 24-bit", …; `ove docs export`).
    #[arg(long)]
    pub preset: Option<String>,
    /// Container: mp4, mov, mkv, webm, m4a, wav, flac (default: the extension of OUT).
    #[arg(long)]
    pub format: Option<String>,
    /// h264, hevc, prores, dnxhr, vp9 or av1.
    #[arg(long)]
    pub video_codec: Option<String>,
    /// ProRes: proxy, lt, standard, hq, 4444; DNxHR: lb, sq, hq, hqx, 444; HEVC: main, main10.
    #[arg(long)]
    pub profile: Option<String>,
    /// Video bit depth: 8, 10 or 12.
    #[arg(long)]
    pub bit_depth: Option<u8>,
    /// very-low, low, medium, high, very-high.
    #[arg(long)]
    pub quality: Option<String>,
    /// Keep transparency (ProRes 4444 in MOV, VP9 in WebM).
    #[arg(long)]
    pub alpha: bool,
    /// aac, pcm, flac, opus or none.
    #[arg(long)]
    pub audio_codec: Option<String>,
    /// PCM and FLAC: 16 or 24.
    #[arg(long)]
    pub audio_bit_depth: Option<u8>,
    /// Only this part of the timeline, in seconds: START-END (e.g. 10-20 or 1.5-4).
    #[arg(long)]
    pub range: Option<String>,
    /// Output size: a height (720, 1080, 2160, 4k) or `match` (the timeline's own size).
    #[arg(long)]
    pub size: Option<String>,
    /// Output height in pixels (same as --size N).
    #[arg(long)]
    pub height: Option<u32>,
    #[arg(long)]
    pub fps: Option<u32>,
}

/// `START-END` (also `START,END` or `START:END`) in seconds.
pub fn parse_range(s: &str) -> Result<[f64; 2], Fail> {
    let bad = || Fail::new("argument.invalid", format!("--range {s:?} is not START-END in seconds (e.g. 10-20)"));
    let (a, b) = s.split_once(['-', ',', ':']).ok_or_else(bad)?;
    let a: f64 = a.trim().parse().map_err(|_| bad())?;
    let b: f64 = b.trim().parse().map_err(|_| bad())?;
    Ok([a, b])
}

/// The `export_video` arguments for the flags given.
pub fn tool_args(a: &ExportArgs) -> Result<Map<String, Value>, Fail> {
    let mut m = Map::new();
    let project = a.project.as_ref().ok_or_else(|| Fail::new("argument.missing", "name the project to export"))?;
    let out = a.out.as_ref().ok_or_else(|| Fail::new("argument.missing", "name the file to export to"))?;
    m.insert("project".into(), json!(project));
    m.insert("out".into(), json!(out));
    let mut put = |k: &str, v: Value| {
        if !v.is_null() {
            m.insert(k.into(), v);
        }
    };
    put("preset", json!(a.preset));
    put("format", json!(a.format));
    put("videoCodec", json!(a.video_codec));
    put("profile", json!(a.profile));
    put("bitDepth", json!(a.bit_depth));
    put("quality", json!(a.quality.as_ref().map(|q| q.replace('-', " "))));
    if a.alpha {
        put("alpha", json!(true));
    }
    put("audioCodec", json!(a.audio_codec));
    put("audioBitDepth", json!(a.audio_bit_depth));
    put("resolution", json!(a.size));
    put("height", json!(a.height));
    put("fps", json!(a.fps));
    if let Some(r) = &a.range {
        put("range", json!(parse_range(r)?));
    }
    Ok(m)
}

pub fn run(ctx: &Ctx, c: &ExportCmd) -> Out {
    match &c.sub {
        Some(ExportSub::Queue { all }) => ctx.tool("list_export_jobs", json!({"all": all})),
        Some(ExportSub::Status { id, wait }) => ctx.tool("job_status", json!({"job_id": id, "wait": wait})),
        Some(ExportSub::Cancel { id }) => ctx.tool("cancel_job", json!({"job_id": id})),
        Some(ExportSub::Presets) => presets(),
        None if c.args.as_media => ctx.tool("save_range_as_media", as_media_args(&c.args)?),
        None => {
            let mut args = tool_args(&c.args)?;
            args.insert("wait".into(), json!(!c.args.detach));
            ctx.tool("export_video", Value::Object(args))
        }
    }
}

/// The `save_range_as_media` arguments for `--as-media`.
pub fn as_media_args(a: &ExportArgs) -> Result<Value, Fail> {
    let project = a.project.as_ref().ok_or_else(|| Fail::new("argument.missing", "name the project"))?;
    let [start, end] = parse_range(a.range.as_deref().unwrap_or_default())?;
    let mut v = json!({"project": project, "start": start, "end": end, "replace": a.replace, "small": a.small});
    if !a.tracks.is_empty() {
        v["tracks"] = json!(a.tracks);
    }
    if let Some(n) = &a.name {
        v["name"] = json!(n);
    }
    Ok(v)
}

/// `ove export --worker`: runs the queue (no library needed).
#[cfg(feature = "media")]
pub fn worker() -> Out {
    let q = ove_tools::tools::export_queue::queue()?;
    let ran = ove_tools::tools::export_queue::run_worker(&q)
        .map_err(|e| Fail::new("export.worker", format!("the export worker stopped: {e}")))?;
    Ok((json!({"ran": ran}), format!("ran {ran} export job{}", if ran == 1 { "" } else { "s" })))
}

#[cfg(feature = "media")]
fn presets() -> Out {
    let all = ove_render::export::presets();
    let text = all.iter().map(|p| format!("{:28} {}", p.name, p.about)).collect::<Vec<_>>().join("\n");
    Ok((json!({"presets": all}), text))
}

#[cfg(not(feature = "media"))]
pub fn worker() -> Out {
    Err(Fail::new("export.unavailable", "this build of ove has no export; build it with `--features media`"))
}

#[cfg(not(feature = "media"))]
fn presets() -> Out {
    worker()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_parse() {
        assert_eq!(parse_range("10-20").unwrap(), [10.0, 20.0]);
        assert_eq!(parse_range("1.5,4").unwrap(), [1.5, 4.0]);
        assert!(parse_range("10").is_err());
        assert!(parse_range("a-b").is_err());
    }
}
