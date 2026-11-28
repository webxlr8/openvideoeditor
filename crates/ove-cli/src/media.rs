//! `ove media …`: list, add, remove, relink and probe the files a project uses.

use std::path::PathBuf;

use clap::Subcommand;
use serde_json::json;

use crate::common::{Ctx, Out};
use ove_tools::Fail;

#[derive(Subcommand)]
pub enum MediaCmd {
    /// List the project's media: id, kind, duration, status (online / offline / changed) and how often it is used.
    #[command(alias = "ls")]
    List {
        project: String,
        /// Also media no clip uses.
        #[arg(long)]
        all: bool,
    },
    /// Add files: probes each, registers it and places a clip on the timeline (one revision).
    Add {
        project: String,
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Track (name or id). Default: Main for video and images, Audio for audio.
        #[arg(long)]
        track: Option<String>,
        /// Start of the first clip in seconds (default: the end of the track); the others follow it.
        #[arg(long)]
        at: Option<f64>,
        /// Only register the files; no clips.
        #[arg(long)]
        no_timeline: bool,
        /// Clip length for still images, in seconds.
        #[arg(long, default_value_t = 5.0)]
        image_duration: f64,
    },
    /// Remove media and every clip that uses it.
    #[command(alias = "rm")]
    Remove {
        project: String,
        #[arg(required = true)]
        media: Vec<String>,
    },
    /// Point media at a new file, or find every offline file by name under --folder.
    Relink {
        project: String,
        media: Option<String>,
        path: Option<PathBuf>,
        #[arg(long)]
        folder: Option<PathBuf>,
    },
    /// Container, streams, codecs, duration and frame rate of a media item or any file.
    Probe { project: String, media: String },
    /// Decode one frame to a PNG.
    Grab {
        project: String,
        media: String,
        #[arg(long, default_value_t = 0.0)]
        at: f64,
        #[arg(long)]
        width: Option<u32>,
        #[arg(long)]
        out: PathBuf,
    },
    /// A contact sheet of frames across the file (or a range).
    #[command(alias = "sheet")]
    Filmstrip {
        project: String,
        media: String,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 8)]
        count: u32,
        /// One frame every N seconds (instead of --count).
        #[arg(long)]
        every: Option<f64>,
        #[arg(long, default_value_t = 0.0)]
        from: f64,
        #[arg(long)]
        to: Option<f64>,
        /// Columns (default: up to 5).
        #[arg(long, default_value_t = 0)]
        columns: u32,
        #[arg(long, default_value_t = 240)]
        tile_width: u32,
    },
    /// Hard cuts in a video: times and scores.
    Scenes {
        project: String,
        media: String,
        #[arg(long, default_value_t = 0.3)]
        threshold: f64,
    },
    /// Extract a media file's sound into the library as a new WAV or FLAC media entry.
    ExtractAudio {
        project: String,
        media: String,
        /// wav (24-bit) or flac.
        #[arg(long, default_value = "wav")]
        format: String,
    },
    /// Search footage by what it shows and what is said: moments, spoken, timelines and files. Each hit carries a
    /// clip line to paste into timeline.tsx. Works on a closed project.
    Search {
        project: String,
        /// Words, or a quoted phrase for exact spoken matches.
        query: Option<String>,
        /// all, moments, spoken, timelines or files.
        #[arg(long, default_value = "all")]
        scope: String,
        /// Only this media item.
        #[arg(long)]
        media: Option<String>,
        /// Results per section (at most 50).
        #[arg(long)]
        limit: Option<i64>,
        /// More like this frame: MEDIA@SECONDS.
        #[arg(long)]
        like: Option<String>,
    },
    /// The transcript of one media file, in source seconds.
    Transcript {
        project: String,
        media: String,
        #[arg(long)]
        from: Option<f64>,
        #[arg(long)]
        to: Option<f64>,
    },
    /// One sheet of up to 36 tiles, one per shot, with time labels.
    Overview {
        project: String,
        media: String,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value_t = 240)]
        tile_width: u32,
    },
    /// Organise media in one revision: OPS is a JSON array of steps (see organize_media), or @FILE.
    #[command(alias = "organise")]
    Organize { project: String, ops: String },
}

pub fn run(ctx: &Ctx, cmd: &MediaCmd) -> Out {
    match cmd {
        MediaCmd::List { project, all } => ctx.tool("list_media", json!({"project": project, "all": all})),
        MediaCmd::Add { project, paths, track, at, no_timeline, image_duration } => ctx.tool(
            "add_media",
            json!({"project": project, "paths": paths, "track": track, "at": at, "noTimeline": no_timeline,
                   "imageDuration": image_duration}),
        ),
        MediaCmd::Remove { project, media } => ctx.tool("remove_media", json!({"project": project, "media": media})),
        MediaCmd::Relink { project, media, path, folder } => {
            ctx.tool("relink_media", json!({"project": project, "media": media, "path": path, "folder": folder}))
        }
        MediaCmd::Probe { project, media } => ctx.tool("probe_media", json!({"project": project, "media": media})),
        MediaCmd::Grab { project, media, at, width, out } => {
            ctx.tool("grab_frame", json!({"project": project, "media": media, "at": at, "width": width, "out": out}))
        }
        MediaCmd::Filmstrip { project, media, out, count, every, from, to, columns, tile_width } => ctx.tool(
            "filmstrip",
            json!({"project": project, "media": media, "out": out, "count": count, "every": every, "from": from,
                   "to": to, "columns": columns, "tileWidth": tile_width}),
        ),
        MediaCmd::ExtractAudio { project, media, format } => {
            ctx.tool("extract_audio", json!({"project": project, "media": media, "format": format}))
        }
        MediaCmd::Search { project, query, scope, media, limit, like } => {
            let similar = match like {
                Some(l) => {
                    let (m, at) = l.rsplit_once('@').ok_or_else(|| {
                        Fail::new("argument.invalid", "--like takes MEDIA@SECONDS, for example interview@12.5")
                    })?;
                    let at: f64 = at.parse().map_err(|_| {
                        Fail::new("argument.invalid", format!("--like: {at} is not a number of seconds"))
                    })?;
                    Some(json!({"media": m, "at": at}))
                }
                None => None,
            };
            ctx.tool(
                "search_media",
                json!({"project": project, "query": query, "scope": scope, "media": media, "limit": limit,
                       "similarTo": similar}),
            )
        }
        MediaCmd::Transcript { project, media, from, to } => {
            ctx.tool("media_transcript", json!({"project": project, "media": media, "from": from, "to": to}))
        }
        MediaCmd::Overview { project, media, out, tile_width } => ctx.tool(
            "inspect_media",
            json!({"project": project, "media": media, "overview": true, "out": out, "tileWidth": tile_width}),
        ),
        MediaCmd::Organize { project, ops } => {
            let text = match ops.strip_prefix('@') {
                Some(path) => {
                    std::fs::read_to_string(path).map_err(|e| Fail::new("file.notFound", format!("{path}: {e}")))?
                }
                None => ops.clone(),
            };
            let ops: serde_json::Value = serde_json::from_str(&text)
                .map_err(|e| Fail::new("argument.invalid", format!("OPS is not JSON: {e}")))?;
            ctx.tool("organize_media", json!({"project": project, "ops": ops}))
        }
        MediaCmd::Scenes { project, media, threshold } => {
            ctx.tool("detect_scenes", json!({"project": project, "media": media, "threshold": threshold}))
        }
    }
}
