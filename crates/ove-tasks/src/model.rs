//! What a task is: its priority, resource class, declared memory, state and the reasons work pauses.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Task identifier, unique within one scheduler (one process).
pub type TaskId = u64;

/// Scheduling priority, most urgent first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    /// P0: export. Never paused; makes derivative and analysis work yield.
    Export = 0,
    /// P1: something the person is waiting for (a probe, the thumbnail being looked at).
    Interactive = 1,
    /// P2: media derivatives (thumbnails, skim sheets, proxies).
    Derivative = 2,
    /// P3: analysis and indexing (captions, search indexes).
    Analysis = 3,
}

impl Priority {
    pub const ALL: [Priority; 4] = [Priority::Export, Priority::Interactive, Priority::Derivative, Priority::Analysis];

    pub fn level(self) -> u8 {
        self as u8
    }

    pub fn from_level(level: u8) -> Priority {
        match level {
            0 => Priority::Export,
            1 => Priority::Interactive,
            2 => Priority::Derivative,
            _ => Priority::Analysis,
        }
    }

    /// Derivative and analysis work pauses for playback, export, battery and heat; the others never do.
    pub fn pausable(self) -> bool {
        self >= Priority::Derivative
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Priority::Export => "export",
            Priority::Interactive => "interactive",
            Priority::Derivative => "derivative",
            Priority::Analysis => "analysis",
        }
    }
}

/// The scarce resource a task mostly uses. Each class has its own concurrency limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Class {
    /// Video decode or encode sessions (hardware sessions are few).
    Decode,
    /// Model inference (one model in memory at a time).
    Inference,
    /// CPU-heavy work.
    Cpu,
    /// Mostly file reads and writes.
    Io,
}

impl Class {
    pub const ALL: [Class; 4] = [Class::Decode, Class::Inference, Class::Cpu, Class::Io];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Class::Decode => "decode",
            Class::Inference => "inference",
            Class::Cpu => "cpu",
            Class::Io => "io",
        }
    }
}

/// Why derivative and analysis work is paused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PauseReason {
    /// The person paused all background work.
    User,
    /// An export is queued or running.
    Export,
    /// The timeline is playing.
    Playback,
    /// The computer runs on battery.
    Battery,
    /// The operating system's low-power (battery saver) mode is on.
    LowPower,
    /// The operating system reports thermal pressure.
    Thermal,
}

impl PauseReason {
    pub const ALL: [PauseReason; 6] = [
        PauseReason::User,
        PauseReason::Export,
        PauseReason::Playback,
        PauseReason::Battery,
        PauseReason::LowPower,
        PauseReason::Thermal,
    ];

    /// The text the Background Tasks popover shows after "Paused: ".
    pub fn label(self) -> &'static str {
        match self {
            PauseReason::User => "paused by you",
            PauseReason::Export => "export running",
            PauseReason::Playback => "playing",
            PauseReason::Battery => "on battery",
            PauseReason::LowPower => "low-power mode",
            PauseReason::Thermal => "computer is hot",
        }
    }

    /// Whether a pause for this reason applies to a task of this priority. Export is never paused; a person's
    /// pause also holds interactive work.
    pub fn applies_to(self, priority: Priority) -> bool {
        match self {
            PauseReason::User => priority != Priority::Export,
            _ => priority.pausable(),
        }
    }
}

/// What a task declares when it is submitted.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskSpec {
    /// Kind, such as `thumbnail`, `probe`, `captions`, `export`, `skim`, `proxy`.
    pub kind: String,
    /// Short human name ("Thumbnail: interview.mov").
    pub label: String,
    /// Project id or bundle path the task belongs to, if any.
    pub project: Option<String>,
    pub priority: Priority,
    pub class: Class,
    /// Estimated peak memory in bytes.
    pub est_mem: u64,
    /// Inputs key: a second submit with the same key while the first is unfinished returns the first task.
    pub key: Option<String>,
    /// Start the task outside the queue if the scheduler has not started it this long after submission.
    pub watchdog: Option<Duration>,
}

impl TaskSpec {
    pub fn new(kind: impl Into<String>, label: impl Into<String>, priority: Priority, class: Class) -> Self {
        TaskSpec {
            kind: kind.into(),
            label: label.into(),
            project: None,
            priority,
            class,
            est_mem: 0,
            key: None,
            watchdog: None,
        }
    }

    pub fn project(mut self, project: impl Into<String>) -> Self {
        self.project = Some(project.into());
        self
    }

    pub fn memory(mut self, bytes: u64) -> Self {
        self.est_mem = bytes;
        self
    }

    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn watchdog(mut self, after: Duration) -> Self {
        self.watchdog = Some(after);
        self
    }
}

/// A task's state as surfaces show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    Queued,
    Running,
    Paused,
    Done,
    Failed,
    Cancelled,
}

impl TaskState {
    pub fn finished(self) -> bool {
        matches!(self, TaskState::Done | TaskState::Failed | TaskState::Cancelled)
    }
}

/// A snapshot of one task, for the popover, the CLI and MCP.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskInfo {
    pub id: TaskId,
    pub kind: String,
    pub label: String,
    pub project: Option<String>,
    pub priority: Priority,
    pub class: Class,
    pub state: TaskState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_reason: Option<PauseReason>,
    /// 0..=1 when the task reports progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    /// The task's memory estimate exceeds the whole budget, so it runs with nothing else beside it.
    #[serde(default)]
    pub running_alone: bool,
    /// Started outside the queue by its watchdog.
    #[serde(default)]
    pub bypassed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub est_mem: u64,
}

/// How a task ended badly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskError {
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for TaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskError::Cancelled => f.write_str("cancelled"),
            TaskError::Failed(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for TaskError {}

/// Returned by a checkpoint when the task has been cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

impl From<Cancelled> for TaskError {
    fn from(_: Cancelled) -> Self {
        TaskError::Cancelled
    }
}

/// Selects tasks for listing, waiting and cancelling.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskFilter {
    pub project: Option<String>,
    pub kind: Option<String>,
}

impl TaskFilter {
    pub fn matches(&self, project: Option<&str>, kind: &str) -> bool {
        self.project.as_deref().is_none_or(|p| project == Some(p)) && self.kind.as_deref().is_none_or(|k| k == kind)
    }
}

/// The result of waiting for tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitOutcome {
    /// Every matching task finished.
    pub done: bool,
    /// Matching tasks still queued, running or paused (0 when `done`).
    pub remaining: usize,
}

/// Something surfaces should redraw for.
#[derive(Debug, Clone, PartialEq)]
pub enum TaskEvent {
    /// A task was added, started, paused, resumed or reported progress.
    Updated(TaskInfo),
    /// A task finished (done, failed or cancelled).
    Finished(TaskInfo),
    /// The set of active pause reasons changed (after overrides).
    PauseChanged(Vec<PauseReason>),
}
