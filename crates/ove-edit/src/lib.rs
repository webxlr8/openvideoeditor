//! Edit planners shared by every writer (GUI, assistant, CLI, MCP): split, trim, ripple trim, ripple delete, insert,
//! overwrite, lift, extract, close gap, slip, slide, roll, move and delete, plus snapping.
//!
//! Each planner is a pure function of a document and a request. It returns a [`Plan`] (the ops of one batch, a
//! summary, the touched item ids and the time map of a ripple) or a [`Refusal`] (a code, the item ids that caused
//! it and a message). The caller applies a plan as one batch, so it becomes one revision and one undo step.
//!
//! Rules every planner follows:
//! - Requested times snap to the timeline's frame grid (`fps`).
//! - Items on locked tracks are never changed; a ripple that would move or cut one is refused (`ripple.blocked`).
//! - A ripple shifts the target tracks, every sync-locked track, and every track holding a link partner of a
//!   shifted item.
//! - Linked items (same `link` value) edit together unless the request says `linked: false`.
//! - A plan never creates an overlap that was not there before, unless the request asks for a crossfade.
#![forbid(unsafe_code)]

pub mod retime;
pub mod snap;
mod verbs;
mod work;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use ove_core::model::Document;
use ove_core::ops::Op;
use ove_core::time::Micros;
use ove_core::timemap::TimeMap;

pub use verbs::plan_with;

/// A planned edit: the ops of one batch and what they do.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub ops: Vec<Op>,
    /// One line for the history and undo menu.
    pub summary: String,
    /// Ids of items the plan adds, changes or removes.
    pub touched: Vec<String>,
    /// How a ripple moved time per track (empty when nothing rippled).
    #[serde(skip_serializing_if = "TimeMap::is_empty")]
    pub time_map: TimeMap,
    /// Items whose edit stopped at a limit (source start or end, a neighbour, time 0).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub limited: Vec<String>,
    /// The document after the plan.
    #[serde(skip)]
    pub result: Document,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

/// Why a request was refused. Nothing changes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Refusal {
    /// Stable code, for example `ripple.blocked`.
    pub code: String,
    /// Ids of the items that caused the refusal.
    pub culprits: Vec<String>,
    pub message: String,
}

impl Refusal {
    pub fn new(code: &str, culprits: Vec<String>, message: impl Into<String>) -> Self {
        Refusal { code: code.into(), culprits, message: message.into() }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Refusal {}

/// Which edge of an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Start,
    End,
}

/// A span of timeline time `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    #[serde(with = "secs")]
    pub start: Micros,
    #[serde(with = "secs")]
    pub end: Micros,
}

fn yes() -> bool {
    true
}

/// An edit request. Times are seconds on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum Request {
    /// Cut items in two at `at`: the given items (and their link partners), or every item under `at` on `tracks`
    /// (all unlocked tracks when empty).
    Split {
        #[serde(default)]
        items: Vec<String>,
        #[serde(default)]
        tracks: Vec<String>,
        #[serde(with = "secs")]
        at: Micros,
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Move one edge of an item; later items stay where they are.
    Trim {
        item: String,
        edge: Edge,
        #[serde(with = "secs")]
        to: Micros,
        #[serde(default = "yes")]
        linked: bool,
        /// Allow the trimmed item to overlap a neighbour (an overlap plays as a crossfade).
        #[serde(default)]
        crossfade: bool,
    },
    /// Move one edge of an item and ripple: later items move by the length added or removed.
    RippleTrim {
        item: String,
        edge: Edge,
        #[serde(with = "secs")]
        to: Micros,
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Remove items and close the space they used, or remove time ranges from `tracks`.
    RippleDelete {
        #[serde(default)]
        items: Vec<String>,
        #[serde(default)]
        tracks: Vec<String>,
        #[serde(default)]
        ranges: Vec<Range>,
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Remove items, leaving their space empty.
    Delete {
        items: Vec<String>,
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Place new items at `at` and push later items on the affected tracks later. Item JSON gives the type, the
    /// fields and the timing relative to the first item (`startTime`/`endTime`); `trackId` defaults to `track`.
    Insert {
        track: String,
        #[serde(with = "secs")]
        at: Micros,
        items: Vec<Value>,
    },
    /// Place new items at `at`, replacing whatever was there on their tracks.
    Overwrite {
        track: String,
        #[serde(with = "secs")]
        at: Micros,
        items: Vec<Value>,
    },
    /// Remove a range from tracks, leaving a gap.
    Lift { tracks: Vec<String>, range: Range },
    /// Remove a range from tracks and close it (later items move earlier).
    Extract { tracks: Vec<String>, range: Range },
    /// Close the empty space at `at` on a track.
    CloseGap {
        track: String,
        #[serde(with = "secs")]
        at: Micros,
    },
    /// Show other source frames in the same timeline span: `delta` seconds of timeline time later in the source.
    Slip {
        item: String,
        #[serde(with = "secs")]
        delta: Micros,
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Move an item between its neighbours, which grow and shrink to keep the track's length.
    Slide {
        item: String,
        #[serde(with = "secs")]
        delta: Micros,
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Move the edit point between two adjacent items.
    Roll {
        left: String,
        right: String,
        #[serde(with = "secs")]
        to: Micros,
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Move an item to a new start (and optionally another track), keeping its length.
    Move {
        item: String,
        #[serde(with = "secs")]
        start: Micros,
        #[serde(default)]
        track: Option<String>,
        #[serde(default = "yes")]
        linked: bool,
        #[serde(default)]
        crossfade: bool,
    },
}

impl Request {
    /// The verb name, as on the command line (`ripple-delete`).
    pub fn verb(&self) -> &'static str {
        match self {
            Request::Split { .. } => "split",
            Request::Trim { .. } => "trim",
            Request::RippleTrim { .. } => "ripple-trim",
            Request::RippleDelete { .. } => "ripple-delete",
            Request::Delete { .. } => "delete",
            Request::Insert { .. } => "insert",
            Request::Overwrite { .. } => "overwrite",
            Request::Lift { .. } => "lift",
            Request::Extract { .. } => "extract",
            Request::CloseGap { .. } => "close-gap",
            Request::Slip { .. } => "slip",
            Request::Slide { .. } => "slide",
            Request::Roll { .. } => "roll",
            Request::Move { .. } => "move",
        }
    }
}

/// Plans a request with item ids from a deterministic generator seeded by the document revision.
pub fn plan(doc: &Document, req: &Request) -> Result<Plan, Refusal> {
    let mut seed = (doc.project.rev as u64) ^ 0x2545_f491_4f6c_dd1d;
    let mut ids = move || {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        format!("i_{:08x}", (seed >> 32) as u32)
    };
    plan_with(doc, req, &mut ids)
}

/// Seconds on the wire, microseconds inside.
pub mod secs {
    use ove_core::time::{Micros, micros_to_json, secs_to_micros};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(us: &Micros, s: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(&micros_to_json(*us), s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Micros, D::Error> {
        let secs = f64::deserialize(d)?;
        secs_to_micros(secs).ok_or_else(|| serde::de::Error::custom("a time must be a finite number of seconds"))
    }
}
