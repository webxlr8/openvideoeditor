//! `ove mcp`: a Model Context Protocol server on stdio (JSON-RPC 2.0, one message per line). Every tool runs the
//! matching `ove` command in-process, so the tools and the command line can never disagree; edits are recorded with
//! source `mcp`.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use serde_json::{Map, Value, json};

use crate::common::Fail;

/// How a tool argument becomes command-line arguments.
#[derive(Clone, Copy)]
enum Kind {
    Str,
    Num,
    Int,
    Bool,
    /// Several positional values.
    List,
    /// Text written to a temporary file whose path is passed (timeline source).
    TextFile,
}

struct Arg {
    name: &'static str,
    kind: Kind,
    required: bool,
    /// `None`: positional.
    flag: Option<&'static str>,
    about: &'static str,
}

const fn pos(name: &'static str, kind: Kind, about: &'static str) -> Arg {
    Arg { name, kind, required: true, flag: None, about }
}
const fn opt_pos(name: &'static str, kind: Kind, about: &'static str) -> Arg {
    Arg { name, kind, required: false, flag: None, about }
}
const fn flag(name: &'static str, kind: Kind, f: &'static str, about: &'static str) -> Arg {
    Arg { name, kind, required: false, flag: Some(f), about }
}

pub struct Tool {
    name: &'static str,
    about: &'static str,
    read_only: bool,
    cmd: &'static [&'static str],
    args: &'static [Arg],
    /// The command writes PNG images to this flag's path (a file, or a folder for several; an empty flag is the
    /// last positional argument); the tool returns them.
    images: Option<(&'static str, bool)>,
}

const PROJECT: Arg = pos("project", Kind::Str, "Project name, id or bundle path");

/// The tool table. Names are snake_case verbs; descriptions say what the tool does and returns.
pub static TOOLS: &[Tool] = &[
    Tool {
        name: "list_projects",
        about: "List the projects in the library (name, id, aspect ratio, duration, bundle path).",
        read_only: true,
        cmd: &["project", "list"],
        args: &[flag("search", Kind::Str, "--search", "Only names containing this text")],
        images: None,
    },
    Tool {
        name: "create_project",
        about: "Create a project with the starter tracks (Text, Captions, Overlay, Main, Audio).",
        read_only: false,
        cmd: &["project", "create"],
        args: &[
            pos("name", Kind::Str, "Project name"),
            flag("aspect", Kind::Str, "--aspect", "16:9, 9:16, 1:1, 4:5, 4:3 or 3:4"),
            flag("open", Kind::Bool, "--open", "Also open it in the running app"),
        ],
        images: None,
    },
    Tool {
        name: "show_project",
        about: "Project summary: aspect ratio, duration, tracks with item counts, media count, revision.",
        read_only: true,
        cmd: &["project", "show"],
        args: &[PROJECT],
        images: None,
    },
    Tool {
        name: "rename_project",
        about: "Rename a project.",
        read_only: false,
        cmd: &["project", "rename"],
        args: &[PROJECT, pos("name", Kind::Str, "New name")],
        images: None,
    },
    Tool {
        name: "print_timeline",
        about: "The timeline as TSX text (timeline.tsx): tracks and clips with every property. Edit this text and pass it to apply_timeline. Lines can be limited with from/lines.",
        read_only: true,
        cmd: &["source", "print"],
        args: &[
            PROJECT,
            flag("from", Kind::Int, "--from", "First line (1-based)"),
            flag("lines", Kind::Int, "--lines", "Number of lines"),
        ],
        images: None,
    },
    Tool {
        name: "check_timeline",
        about: "Check an edited timeline text without applying it: parse errors with positions, what would change, conflicts with edits made meanwhile.",
        read_only: true,
        cmd: &["source", "check"],
        args: &[
            PROJECT,
            pos("source", Kind::TextFile, "The full edited timeline.tsx text"),
            flag("prefer", Kind::Str, "--prefer", "On conflicting edits keep the `file` (default) or `editor` value"),
        ],
        images: None,
    },
    Tool {
        name: "apply_timeline",
        about: "Apply an edited timeline text as one revision (one undo step). The text must keep the header comment with its base revision; edits made meanwhile are merged.",
        read_only: false,
        cmd: &["source", "apply"],
        args: &[
            PROJECT,
            pos("source", Kind::TextFile, "The full edited timeline.tsx text"),
            flag("prefer", Kind::Str, "--prefer", "On conflicting edits keep the `file` (default) or `editor` value"),
        ],
        images: None,
    },
    Tool {
        name: "verify_timeline",
        about: "Report problems: overlaps, gaps, clips past the end of their source, missing media, pending placeholders.",
        read_only: true,
        cmd: &["verify"],
        args: &[PROJECT],
        images: None,
    },
    Tool {
        name: "timeline_history",
        about: "List revisions (oldest first) with their source and summary.",
        read_only: true,
        cmd: &["history"],
        args: &[PROJECT],
        images: None,
    },
    Tool {
        name: "undo_timeline",
        about: "Undo the latest revision, or return to the state of revision `rev`, as a new revision.",
        read_only: false,
        cmd: &["undo"],
        args: &[PROJECT, opt_pos("rev", Kind::Int, "Revision to return to")],
        images: None,
    },
    Tool {
        name: "list_media",
        about: "The project's media: id, kind, duration, online/offline status and use count.",
        read_only: true,
        cmd: &["media", "list"],
        args: &[PROJECT, flag("all", Kind::Bool, "--all", "Include unused media")],
        images: None,
    },
    Tool {
        name: "add_media",
        about: "Add files to a project: each is probed, registered and placed as a clip (video/images on Main, audio on Audio, after the last clip unless `at` is given).",
        read_only: false,
        cmd: &["media", "add"],
        args: &[
            PROJECT,
            pos("paths", Kind::List, "Absolute file paths"),
            flag("track", Kind::Str, "--track", "Track name or id"),
            flag("at", Kind::Num, "--at", "Start of the first clip in seconds"),
            flag("noTimeline", Kind::Bool, "--no-timeline", "Only register the files"),
        ],
        images: None,
    },
    Tool {
        name: "remove_media",
        about: "Remove media items and every clip that uses them.",
        read_only: false,
        cmd: &["media", "remove"],
        args: &[PROJECT, pos("media", Kind::List, "Media ids or file names")],
        images: None,
    },
    Tool {
        name: "relink_media",
        about: "Point a media item at a new file, or find offline files by name under a folder.",
        read_only: false,
        cmd: &["media", "relink"],
        args: &[
            PROJECT,
            opt_pos("media", Kind::Str, "Media id or file name"),
            opt_pos("path", Kind::Str, "New file path"),
            flag("folder", Kind::Str, "--folder", "Folder to search"),
        ],
        images: None,
    },
    Tool {
        name: "probe_media",
        about: "Container, streams, codecs, resolution, frame rate and duration of a media item or file.",
        read_only: true,
        cmd: &["media", "probe"],
        args: &[PROJECT, pos("media", Kind::Str, "Media id, file name or path")],
        images: None,
    },
    Tool {
        name: "grab_frame",
        about: "Decode one frame of a media item or file; returns it as an image.",
        read_only: true,
        cmd: &["media", "grab"],
        args: &[
            PROJECT,
            pos("media", Kind::Str, "Media id, file name or path"),
            flag("at", Kind::Num, "--at", "Time in seconds (default 0)"),
            flag("width", Kind::Int, "--width", "Width in pixels (default: source width)"),
        ],
        images: Some(("--out", false)),
    },
    Tool {
        name: "filmstrip",
        about: "Contact sheet of frames across a media file (or a range); returns the image and the frame times.",
        read_only: true,
        cmd: &["media", "filmstrip"],
        args: &[
            PROJECT,
            pos("media", Kind::Str, "Media id, file name or path"),
            flag("count", Kind::Int, "--count", "Frames (default 8)"),
            flag("every", Kind::Num, "--every", "One frame every N seconds"),
            flag("from", Kind::Num, "--from", "Start (s)"),
            flag("to", Kind::Num, "--to", "End (s)"),
            flag("columns", Kind::Int, "--columns", "Columns"),
            flag("tileWidth", Kind::Int, "--tile-width", "Tile width in pixels (default 240)"),
        ],
        images: Some(("--out", false)),
    },
    Tool {
        name: "detect_scenes",
        about: "Hard cuts in a video: times (s) and scores.",
        read_only: true,
        cmd: &["media", "scenes"],
        args: &[
            PROJECT,
            pos("media", Kind::Str, "Media id, file name or path"),
            flag("threshold", Kind::Num, "--threshold", "0–1, default 0.3"),
        ],
        images: None,
    },
    Tool {
        name: "audio_levels",
        about: "Peak and RMS level (dBFS) per bucket across an audio or video file.",
        read_only: true,
        cmd: &["audio", "levels"],
        args: &[
            PROJECT,
            pos("media", Kind::Str, "Media id, file name or path"),
            flag("points", Kind::Int, "--points", "Buckets (default 100)"),
        ],
        images: None,
    },
    Tool {
        name: "waveform_image",
        about: "The waveform of an audio or video file as an image.",
        read_only: true,
        cmd: &["audio", "waveform"],
        args: &[
            PROJECT,
            pos("media", Kind::Str, "Media id, file name or path"),
            flag("width", Kind::Int, "--width", "Pixels (default 1200)"),
            flag("height", Kind::Int, "--height", "Pixels (default 200)"),
        ],
        images: Some(("--out", false)),
    },
    Tool {
        name: "detect_silence",
        about: "Ranges quieter than a threshold for at least a minimum length — candidates for cuts.",
        read_only: true,
        cmd: &["audio", "silence"],
        args: &[
            PROJECT,
            pos("media", Kind::Str, "Media id, file name or path"),
            flag("thresholdDb", Kind::Num, "--threshold", "dBFS, default -40"),
            flag("minDuration", Kind::Num, "--min", "Seconds, default 0.5"),
        ],
        images: None,
    },
    Tool {
        name: "measure_loudness",
        about: "Integrated loudness (LUFS, EBU R128), loudness range, sample peak and true peak.",
        read_only: true,
        cmd: &["audio", "loudness"],
        args: &[PROJECT, pos("media", Kind::Str, "Media id, file name or path")],
        images: None,
    },
    Tool {
        name: "look",
        about: "Render composited timeline frames (what the export will show) and return them as images, optionally with brightness stats.",
        read_only: true,
        cmd: &["look"],
        args: &[
            PROJECT,
            flag("at", Kind::Str, "--at", "Times in seconds, comma-separated"),
            flag("every", Kind::Num, "--every", "One frame every N seconds"),
            flag("count", Kind::Int, "--count", "Frames across the timeline (default 6)"),
            flag("width", Kind::Int, "--width", "Pixels (default 640)"),
            flag("stats", Kind::Bool, "--stats", "Brightness and black/clipped shares"),
        ],
        images: Some(("--out", true)),
    },
    Tool {
        name: "screenshot",
        about: "Render the frame under the running app's playhead; returns the image.",
        read_only: true,
        cmd: &["screenshot"],
        args: &[flag("width", Kind::Int, "--width", "Pixels (default 1280)")],
        images: Some(("", false)),
    },
    Tool {
        name: "export_video",
        about: "Render the timeline to an MP4 (H.264 + AAC). Refused while placeholders are pending. Takes about real time or less.",
        read_only: false,
        cmd: &["export"],
        args: &[
            PROJECT,
            pos("out", Kind::Str, "Output .mp4 path"),
            flag("height", Kind::Int, "--height", "720, 1080 (default) or 2160"),
            flag("fps", Kind::Int, "--fps", "Frame rate (default 30)"),
        ],
        images: None,
    },
    Tool {
        name: "export_timeline_file",
        about: "Write the timeline for another editor: FCPXML (Final Cut, Resolve), XML (Premiere), EDL or OTIO — chosen by the file extension.",
        read_only: true,
        cmd: &["timeline", "export"],
        args: &[
            PROJECT,
            pos("file", Kind::Str, "Output path (.fcpxml, .xml, .edl or .otio)"),
            flag("fps", Kind::Num, "--fps", "Frame rate (default 30; 29.97 and 23.976 allowed)"),
        ],
        images: None,
    },
    Tool {
        name: "import_timeline_file",
        about: "Add the tracks and clips of an FCPXML / XML / EDL / OTIO file to a project as one revision.",
        read_only: false,
        cmd: &["timeline", "import"],
        args: &[
            PROJECT,
            pos("file", Kind::Str, "Timeline file path"),
            flag("fps", Kind::Num, "--fps", "Frame rate of an EDL (default 30)"),
        ],
        images: None,
    },
    Tool {
        name: "app_context",
        about: "What the running app shows: open project (name, id, bundle, revision), playhead, play state, selected clip ids, viewer mode.",
        read_only: true,
        cmd: &["app", "context"],
        args: &[],
        images: None,
    },
    Tool {
        name: "open_in_app",
        about: "Open a project in the running app.",
        read_only: false,
        cmd: &["app", "open"],
        args: &[PROJECT],
        images: None,
    },
    Tool {
        name: "seek",
        about: "Move the app's playhead to a time in seconds.",
        read_only: false,
        cmd: &["app", "seek"],
        args: &[pos("seconds", Kind::Num, "Time in seconds")],
        images: None,
    },
    Tool {
        name: "play",
        about: "Start playback in the app.",
        read_only: false,
        cmd: &["app", "play"],
        args: &[],
        images: None,
    },
    Tool {
        name: "pause",
        about: "Pause playback in the app.",
        read_only: false,
        cmd: &["app", "pause"],
        args: &[],
        images: None,
    },
    Tool {
        name: "docs",
        about: "Documentation: without a topic, the list of topics; `agents` is the editing guide (start there).",
        read_only: true,
        cmd: &["docs"],
        args: &[opt_pos("topic", Kind::Str, "Topic name")],
        images: None,
    },
];

pub fn tool(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.name == name)
}

fn schema(t: &Tool) -> Value {
    let mut props = Map::new();
    let mut req = Vec::new();
    for a in t.args {
        let ty = match a.kind {
            Kind::Str | Kind::TextFile => json!({"type": "string"}),
            Kind::Num => json!({"type": "number"}),
            Kind::Int => json!({"type": "integer"}),
            Kind::Bool => json!({"type": "boolean"}),
            Kind::List => json!({"type": "array", "items": {"type": "string"}, "minItems": 1}),
        };
        let mut ty = ty;
        ty["description"] = a.about.into();
        props.insert(a.name.into(), ty);
        if a.required {
            req.push(a.name);
        }
    }
    json!({"type": "object", "properties": props, "required": req, "additionalProperties": false})
}

pub fn list() -> Value {
    let tools: Vec<Value> = TOOLS
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.about,
                "inputSchema": schema(t),
                "annotations": {"readOnlyHint": t.read_only, "destructiveHint": !t.read_only && matches!(t.name, "remove_media" | "undo_timeline")},
            })
        })
        .collect();
    json!({"tools": tools})
}

fn scalar(v: &Value, name: &str, kind: Kind) -> Result<String, Fail> {
    let bad = || Fail::new("argument.invalid", format!("`{name}` has the wrong type"));
    Ok(match kind {
        Kind::Str | Kind::TextFile => v.as_str().ok_or_else(bad)?.to_string(),
        Kind::Num => v.as_f64().ok_or_else(bad)?.to_string(),
        Kind::Int => v.as_i64().ok_or_else(bad)?.to_string(),
        Kind::Bool | Kind::List => return Err(bad()),
    })
}

/// Temporary files a call created, inside a private directory (random name, owner-only on Unix) made on first use
/// and removed with everything in it when dropped. Paths in the shared temp folder were predictable and written
/// through symlinks; nothing else can create entries in this directory.
#[derive(Default)]
pub struct Scratch {
    dir: Option<tempfile::TempDir>,
    made: usize,
}

impl Scratch {
    /// A fresh path in the private directory (not created).
    pub fn file(&mut self, ext: &str) -> Result<PathBuf, Fail> {
        let dir = match &mut self.dir {
            Some(d) => d,
            empty => {
                let mut b = tempfile::Builder::new();
                b.prefix("ove-mcp-");
                // Owner-only (the default follows the umask, usually 0755). On Windows the per-user temp folder's
                // ACL already keeps other users out.
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    b.permissions(std::fs::Permissions::from_mode(0o700));
                }
                empty.insert(b.tempdir().map_err(|e| Fail::new("file.unwritable", format!("temporary folder: {e}")))?)
            }
        };
        self.made += 1;
        Ok(dir.path().join(format!("{}.{ext}", self.made)))
    }
}

/// Writes a new file (never an existing one, never through a link).
fn write_new(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    f.write_all(text.as_bytes())
}

/// Command-line arguments for a tool call (without the program name and global flags), plus the image output path.
pub fn argv(
    t: &Tool,
    args: &Map<String, Value>,
    scratch: &mut Scratch,
) -> Result<(Vec<String>, Option<PathBuf>), Fail> {
    for k in args.keys() {
        if !t.args.iter().any(|a| a.name == k) {
            return Err(Fail::new("argument.unknown", format!("{} has no argument `{k}`", t.name)));
        }
    }
    let mut out: Vec<String> = t.cmd.iter().map(|s| s.to_string()).collect();
    let mut flags = Vec::new();
    for a in t.args {
        let Some(v) = args.get(a.name).filter(|v| !v.is_null()) else {
            if a.required {
                return Err(Fail::new("argument.missing", format!("{} needs `{}`", t.name, a.name)));
            }
            continue;
        };
        let values: Vec<String> = match a.kind {
            Kind::Bool => {
                if v.as_bool()
                    .ok_or_else(|| Fail::new("argument.invalid", format!("`{}` must be true or false", a.name)))?
                {
                    flags.push(a.flag.unwrap_or_default().to_string());
                }
                continue;
            }
            Kind::List => v
                .as_array()
                .ok_or_else(|| Fail::new("argument.invalid", format!("`{}` must be a list", a.name)))?
                .iter()
                .map(|x| scalar(x, a.name, Kind::Str))
                .collect::<Result<_, _>>()?,
            Kind::TextFile => {
                let p = scratch.file("tsx")?;
                write_new(&p, &scalar(v, a.name, Kind::TextFile)?)
                    .map_err(|e| Fail::new("file.unwritable", format!("temporary file: {e}")))?;
                vec![p.display().to_string()]
            }
            k => vec![scalar(v, a.name, k)?],
        };
        match a.flag {
            Some(f) => {
                for x in values {
                    flags.push(f.to_string());
                    flags.push(x);
                }
            }
            None => out.extend(values),
        }
    }
    // Positional values may start with `-` (negative numbers): end option parsing before them is not possible
    // with flags after, so flags go first.
    let cmd_len = t.cmd.len();
    let mut positional: Vec<String> = out.split_off(cmd_len);
    out.extend(flags);
    let mut image = None;
    if let Some((f, many)) = t.images {
        let p = if many {
            let d = scratch.file("d")?;
            std::fs::create_dir_all(&d).map_err(|e| Fail::new("file.unwritable", e.to_string()))?;
            d
        } else {
            scratch.file("png")?
        };
        if f.is_empty() {
            positional.push(p.display().to_string());
        } else {
            out.push(f.to_string());
            out.push(p.display().to_string());
        }
        image = Some(p);
    }
    if !positional.is_empty() {
        out.push("--".into());
        out.extend(positional);
    }
    Ok((out, image))
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}

fn images_at(p: &std::path::Path) -> Vec<PathBuf> {
    if p.is_dir() {
        let mut v: Vec<PathBuf> = std::fs::read_dir(p)
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|x| x.extension().is_some_and(|e| e == "png")).collect())
            .unwrap_or_default();
        v.sort();
        v
    } else if p.is_file() {
        vec![p.to_path_buf()]
    } else {
        vec![]
    }
}

/// Runs one tool call: the `tools/call` result object.
pub fn call(name: &str, args: &Map<String, Value>, run: &dyn Fn(Vec<String>) -> crate::common::Out) -> Value {
    let Some(t) = tool(name) else {
        return json!({"content": [{"type": "text", "text": format!("unknown tool {name}")}], "isError": true});
    };
    let mut scratch = Scratch::default();
    let res = argv(t, args, &mut scratch).and_then(|(a, img)| run(a).map(|r| (r, img)));
    match res {
        Ok(((v, _), img)) => {
            let mut content =
                vec![json!({"type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default()})];
            for p in img.as_deref().map(images_at).unwrap_or_default().iter().take(12) {
                if let Ok(b) = std::fs::read(p) {
                    content.push(json!({"type": "image", "data": base64(&b), "mimeType": "image/png"}));
                }
            }
            json!({"content": content, "structuredContent": if v.is_object() { v } else { json!({"result": v}) }, "isError": false})
        }
        Err(f) => {
            let e = json!({"error": {"code": f.code, "message": f.message, "details": f.details}});
            json!({"content": [{"type": "text", "text": e.to_string()}], "structuredContent": e, "isError": true})
        }
    }
}

const INSTRUCTIONS: &str = "Edit OpenVideoEditor projects. Start with `docs` topic `agents` for the editing guide. \
Typical loop: app_context (what the user has open) → print_timeline → edit the TSX text → check_timeline → \
apply_timeline (one undo step) → verify_timeline. Times are seconds. When the app has the project open, edits \
appear in it at once.";

const PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// Serves MCP on stdin/stdout until stdin closes.
pub fn serve(run: &dyn Fn(Vec<String>) -> crate::common::Out) -> std::io::Result<()> {
    crate::common::set_progress(false);
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Err(e) => Some(
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}}),
            ),
            Ok(msg) => handle_contained(&msg, run),
        };
        if let Some(r) = reply {
            writeln!(stdout, "{r}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

pub fn handle(msg: &Value, run: &dyn Fn(Vec<String>) -> crate::common::Out) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    // Notifications (no id) get no reply.
    let id = id?;
    let result = match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOLS[0]);
            let version = if PROTOCOLS.contains(&asked) { asked } else { PROTOCOLS[0] };
            Ok(json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "openvideoeditor", "title": "OpenVideoEditor", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(list()),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").and_then(Value::as_object).cloned().unwrap_or_default();
            if tool(name).is_none() {
                Err((-32602, format!("unknown tool: {name}")))
            } else {
                Ok(call(name, &args, run))
            }
        }
        "resources/list" => Ok(json!({"resources": []})),
        "prompts/list" => Ok(json!({"prompts": []})),
        other => Err((-32601, format!("method not found: {other}"))),
    };
    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err((code, message)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
    })
}

/// [`handle`], with a panic anywhere in the message's handling (a tool, a command it runs) answered as a JSON-RPC
/// internal error, so one bad call does not end the session.
pub fn handle_contained(msg: &Value, run: &dyn Fn(Vec<String>) -> crate::common::Out) -> Option<Value> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(msg, run))) {
        Ok(reply) => reply,
        Err(payload) => {
            let what = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".into());
            let id = msg.get("id").cloned()?;
            Some(
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": format!("internal error: {what}")}}),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call_msg(name: &str, arguments: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": name, "arguments": arguments}})
    }

    #[test]
    fn a_panicking_tool_is_an_error_reply_not_the_end() {
        let msg = call_msg("list_projects", json!({}));
        let reply = handle_contained(&msg, &|_| panic!("tool blew up")).unwrap();
        assert_eq!(reply["id"], 7);
        assert_eq!(reply["error"]["code"], -32603);
        assert!(reply["error"]["message"].as_str().unwrap().contains("tool blew up"));
        // The next message is served normally.
        let ok = handle_contained(&msg, &|_| Ok((json!({"projects": []}), String::new()))).unwrap();
        assert_eq!(ok["result"]["isError"], false);
        // A notification that panics gets no reply.
        let note = json!({"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "list_projects"}});
        assert!(handle_contained(&note, &|_| panic!("quiet")).is_none());
    }

    #[test]
    fn timeline_text_goes_to_a_private_folder_that_is_removed() {
        let seen = std::cell::RefCell::new(None::<PathBuf>);
        let msg = call_msg("check_timeline", json!({"project": "p", "source": "<timeline />"}));
        let reply = handle_contained(&msg, &|args| {
            let path = PathBuf::from(args.iter().find(|a| a.ends_with(".tsx")).unwrap());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "<timeline />");
            let dir = path.parent().unwrap().to_path_buf();
            assert!(dir.file_name().unwrap().to_string_lossy().starts_with("ove-mcp-"));
            assert_ne!(dir, std::env::temp_dir(), "not directly in the shared temp folder");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
            }
            *seen.borrow_mut() = Some(dir);
            Ok((json!({}), String::new()))
        })
        .unwrap();
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        assert!(!seen.borrow().as_ref().unwrap().exists(), "removed after the call");
    }
}
