//! `ove edit <verb> <project> …`: the shared edit planners (split, trim, ripple trim, ripple delete, delete, insert,
//! overwrite, lift, extract, close gap, slip, slide, roll, move) from the command line and MCP (`edit_timeline`).
//! One plan is one revision; `--dry-run` prints the plan and changes nothing; a refusal exits 1 with its code, the
//! culprit item ids and a message. `--rev` names the revision the request was made against: times in it are moved
//! through the ripples made since, so a stale request lands where it was meant.

use clap::{Args, ValueEnum};
use ove_core::time::Micros;
use ove_edit::{Edge, Range, Request};
use serde_json::{Value, json};

use crate::common::{Ctx, Fail, Out, secs};

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Verb {
    Split,
    Trim,
    RippleTrim,
    RippleDelete,
    Delete,
    Insert,
    Overwrite,
    Lift,
    Extract,
    CloseGap,
    Slip,
    Slide,
    Roll,
    Move,
    /// The whole request as JSON in `--request` (what MCP's `edit_timeline` sends).
    Request,
}

#[derive(Args)]
pub struct EditArgs {
    pub verb: Verb,
    pub project: String,
    /// The item to edit.
    #[arg(long)]
    pub item: Option<String>,
    /// Items, comma-separated.
    #[arg(long, value_delimiter = ',')]
    pub items: Vec<String>,
    /// Tracks, comma-separated (also `--track`).
    #[arg(long = "tracks", alias = "track", value_delimiter = ',')]
    pub tracks: Vec<String>,
    /// A time in seconds (split, insert, overwrite, close-gap).
    #[arg(long)]
    pub at: Option<f64>,
    /// The new edge time in seconds (trim, ripple-trim, roll).
    #[arg(long)]
    pub to: Option<f64>,
    /// `start` or `end` (trim, ripple-trim).
    #[arg(long)]
    pub edge: Option<String>,
    /// Seconds of timeline time (slip, slide); may be negative.
    #[arg(long, allow_hyphen_values = true)]
    pub delta: Option<f64>,
    /// The new start in seconds (move).
    #[arg(long)]
    pub start: Option<f64>,
    /// Left and right item of an edit point (roll).
    #[arg(long)]
    pub left: Option<String>,
    #[arg(long)]
    pub right: Option<String>,
    /// One range `A-B` in seconds (lift, extract).
    #[arg(long)]
    pub range: Option<String>,
    /// Ranges `A-B,C-D` in seconds (ripple-delete).
    #[arg(long)]
    pub ranges: Option<String>,
    /// New items as a JSON array, timing relative to the first (insert, overwrite).
    #[arg(long = "items-json")]
    pub items_json: Option<String>,
    /// The request as JSON (`request` verb), e.g. {"op":"rippleDelete","tracks":["a1"],"ranges":[{"start":1,"end":2}]}.
    #[arg(long)]
    pub request: Option<String>,
    /// Edit only the given item, not its link group (J and L cuts).
    #[arg(long)]
    pub unlinked: bool,
    /// Allow a trim or move to overlap a neighbour (plays as a crossfade).
    #[arg(long)]
    pub crossfade: bool,
    /// The revision the request was made against (default: the current one).
    #[arg(long)]
    pub rev: Option<i64>,
    /// Print the plan without applying it.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

fn need<T: Clone>(v: &Option<T>, flag: &str, verb: &str) -> Result<T, Fail> {
    v.clone().ok_or_else(|| Fail::new("argument.missing", format!("`{verb}` needs --{flag}")))
}

fn time(v: Option<f64>, flag: &str, verb: &str) -> Result<Micros, Fail> {
    secs(need(&v, flag, verb)?, &format!("--{flag}"))
}

fn signed(v: Option<f64>, flag: &str, verb: &str) -> Result<Micros, Fail> {
    ove_core::time::secs_to_micros(need(&v, flag, verb)?)
        .ok_or_else(|| Fail::new("argument.invalid", format!("--{flag} must be a number of seconds")))
}

fn edge(v: &Option<String>, verb: &str) -> Result<Edge, Fail> {
    match need(v, "edge", verb)?.as_str() {
        "start" => Ok(Edge::Start),
        "end" => Ok(Edge::End),
        other => Err(Fail::new("argument.invalid", format!("--edge must be start or end, got {other}"))),
    }
}

/// `A-B` in seconds.
pub fn parse_range(s: &str) -> Result<Range, Fail> {
    let bad = || Fail::new("argument.invalid", format!("a range is START-END in seconds, got `{s}`"));
    let (a, b) = s.trim().split_once('-').ok_or_else(bad)?;
    let a: f64 = a.trim().parse().map_err(|_| bad())?;
    let b: f64 = b.trim().parse().map_err(|_| bad())?;
    Ok(Range { start: secs(a, "range start")?, end: secs(b, "range end")? })
}

fn items(a: &EditArgs) -> Vec<String> {
    let mut v = a.items.clone();
    if let Some(i) = &a.item {
        v.insert(0, i.clone());
    }
    v
}

/// The planner request the arguments describe.
pub fn request(a: &EditArgs) -> Result<Request, Fail> {
    let linked = !a.unlinked;
    let verb = a.verb.to_possible_value().map(|v| v.get_name().to_string()).unwrap_or_default();
    let v = verb.as_str();
    let new_items = || -> Result<Vec<Value>, Fail> {
        let text = need(&a.items_json, "items-json", v)?;
        serde_json::from_str::<Vec<Value>>(&text)
            .map_err(|e| Fail::new("argument.invalid", format!("--items-json must be a JSON array of items: {e}")))
    };
    Ok(match a.verb {
        Verb::Split => Request::Split { items: items(a), tracks: a.tracks.clone(), at: time(a.at, "at", v)?, linked },
        Verb::Trim => Request::Trim {
            item: need(&a.item, "item", v)?,
            edge: edge(&a.edge, v)?,
            to: time(a.to, "to", v)?,
            linked,
            crossfade: a.crossfade,
        },
        Verb::RippleTrim => Request::RippleTrim {
            item: need(&a.item, "item", v)?,
            edge: edge(&a.edge, v)?,
            to: time(a.to, "to", v)?,
            linked,
        },
        Verb::RippleDelete => {
            let ranges = match &a.ranges {
                Some(r) => r.split(',').map(parse_range).collect::<Result<Vec<_>, _>>()?,
                None => vec![],
            };
            if ranges.is_empty() && items(a).is_empty() {
                return Err(Fail::new("argument.missing", "`ripple-delete` needs --items or --ranges"));
            }
            Request::RippleDelete { items: items(a), tracks: a.tracks.clone(), ranges, linked }
        }
        Verb::Delete => Request::Delete { items: items(a), linked },
        Verb::Insert => Request::Insert {
            track: need(&a.tracks.first().cloned(), "track", v)?,
            at: time(a.at, "at", v)?,
            items: new_items()?,
        },
        Verb::Overwrite => Request::Overwrite {
            track: need(&a.tracks.first().cloned(), "track", v)?,
            at: time(a.at, "at", v)?,
            items: new_items()?,
        },
        Verb::Lift => Request::Lift { tracks: a.tracks.clone(), range: parse_range(&need(&a.range, "range", v)?)? },
        Verb::Extract => {
            Request::Extract { tracks: a.tracks.clone(), range: parse_range(&need(&a.range, "range", v)?)? }
        }
        Verb::CloseGap => {
            Request::CloseGap { track: need(&a.tracks.first().cloned(), "track", v)?, at: time(a.at, "at", v)? }
        }
        Verb::Slip => Request::Slip { item: need(&a.item, "item", v)?, delta: signed(a.delta, "delta", v)?, linked },
        Verb::Slide => Request::Slide { item: need(&a.item, "item", v)?, delta: signed(a.delta, "delta", v)?, linked },
        Verb::Roll => Request::Roll {
            left: need(&a.left, "left", v)?,
            right: need(&a.right, "right", v)?,
            to: time(a.to, "to", v)?,
            linked,
        },
        Verb::Move => Request::Move {
            item: need(&a.item, "item", v)?,
            start: time(a.start, "start", v)?,
            track: a.tracks.first().cloned(),
            linked,
            crossfade: a.crossfade,
        },
        Verb::Request => {
            let text = need(&a.request, "request", v)?;
            serde_json::from_str(&text)
                .map_err(|e| Fail::new("argument.invalid", format!("--request is not a valid edit request: {e}")))?
        }
    })
}

/// Runs the request through the registry's `edit_timeline`, the handler MCP and the in-app assistant use.
pub fn run(ctx: &Ctx, a: &EditArgs) -> Out {
    let req = request(a)?;
    let request = serde_json::to_value(&req).map_err(|e| Fail::new("argument.invalid", e.to_string()))?;
    ctx.tool("edit_timeline", json!({"project": a.project, "request": request, "rev": a.rev, "preview": a.dry_run}))
}
