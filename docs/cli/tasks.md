# Background tasks

Background work (thumbnails, probes, skim sheets, proxies, captions, export, analysis) runs through one scheduler,
the `ove-tasks` crate. The app and the CLI both use it. This page is for people who add a new kind of background
work.

## Priorities

| Priority | Level | Used for | Pauses |
|---|---|---|---|
| `export` | P0 | Export | Never. While an export is queued or running, P2 and P3 work pauses. |
| `interactive` | P1 | Work the person waits for: a probe at import, the thumbnail being looked at, a hovered skim sheet | Only when the person pauses all background work |
| `derivative` | P2 | Thumbnails, skim sheets, proxies | Playback, export, battery, low-power mode, thermal pressure, the person's pause |
| `analysis` | P3 | Captions, transcripts, indexes | As P2 |

A waiting task rises one level for every 5 minutes it waits, up to `interactive`, so nothing starves. Ageing never
lifts a task above `interactive`: export always goes first.

## Resource classes and memory

Each task declares the resource it mostly uses. The scheduler runs at most this many tasks of a class at once:

| Class | Limit | Examples |
|---|---|---|
| `decode` | 2 | Video decode or encode: thumbnails, skim sheets, proxies, export |
| `inference` | 1 | Speech recognition, embedding models |
| `cpu` | logical cores − 2, at least 1 | Waveforms, image analysis |
| `io` | 4 | Probes, hashing, copying |

Each task also declares its estimated peak memory. The running tasks together stay inside the background memory
budget: the smaller of 1.5 GB and 20 % of physical memory. A task larger than the whole budget runs alone when nothing
else runs, and nothing starts beside it.

Task threads run at the operating system's background priority (Windows background mode, macOS "utility" quality of
service). Export threads run at normal priority.

## Pausing

The scheduler pauses P2 and P3 work while the timeline plays, while an export runs, on battery power, in low-power
mode and under thermal pressure. The person can pause all background work (P1 too) and can choose "run anyway" for any
one reason. A task pauses at its next checkpoint. While paused, it gives its class slot and memory back, so an export
never waits behind paused work.

## Cancelling and partial output

Every task gets a cancel token. A task calls `ctx.checkpoint()?` once per frame or chunk: the call returns an error
once the task is cancelled, and blocks while the task is paused. A cancelled task stops within 200 ms.

A task writes its output to `<final name>.part` and renames it when it succeeds (`ove_tasks::PartFile`,
`ove_tasks::write_output`). A part file that is not committed is removed, so features never see half-written output.

## Results are re-derived, not queued

The queue is not saved. Each task's output path is a function of its inputs (for media, the file's content id), so a
finished output is found again and reused. On project open, a scan queues only the outputs that are missing. Give
each task an inputs key: a second submit with the same key while the first is unfinished returns the first task.

## Adding a task kind

1. Choose the kind name, priority and class.
2. Derive the output path from the inputs. Return early when the output already exists.
3. Submit the task. Check in once per unit of work, report progress, and write through a part file.

```rust
use std::path::PathBuf;
use std::time::Duration;
use ove_tasks::{Class, Options, PartFile, Priority, Scheduler, TaskError, TaskSpec};

let scheduler = Scheduler::new(Options::default());
let dir = tempfile::tempdir().unwrap();
let output: PathBuf = dir.path().join("cache/waveform/0123abcd.bin");

let spec = TaskSpec::new("waveform", "Waveform: interview.wav", Priority::Derivative, Class::Cpu)
    .project("my-project")
    .memory(32 * 1024 * 1024)
    .key("waveform:0123abcd");
let out = output.clone();
let id = scheduler.submit(spec, move |ctx| {
    if out.exists() {
        return Ok(()); // Already derived: reuse it.
    }
    let part = PartFile::new(&out).map_err(|e| TaskError::Failed(e.to_string()))?;
    let chunks = 10;
    for i in 0..chunks {
        ctx.checkpoint()?; // Stops here when cancelled; waits here while paused.
        // ... compute one chunk and append it to part.path() ...
        ctx.progress((i + 1) as f64 / chunks as f64);
    }
    std::fs::write(part.path(), b"peaks").map_err(|e| TaskError::Failed(e.to_string()))?;
    part.commit().map_err(|e| TaskError::Failed(e.to_string()))?;
    Ok(())
});

let done = scheduler.wait_task(id, Duration::from_secs(10)).unwrap();
assert_eq!(done.state, ove_tasks::TaskState::Done);
assert!(output.exists());
```

Progress events go out at most 4 times per second per task, whatever rate the task reports at.

## Seeing and cancelling tasks

The status bar's Background Tasks popover lists running, waiting and paused tasks with their progress and pause
reason. The CLI and MCP list, cancel and wait for tasks:

```sh
ove tasks list --json
ove tasks wait --kind thumbnail --timeout 30
ove tasks cancel 12
```

MCP tools: `list_tasks`, `wait_tasks` (with a time-out; it returns the remaining count when time runs out) and
`cancel_task`.
