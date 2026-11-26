//! Scheduler behaviour on real threads: completion, cancellation, pause and resume at checkpoints, export
//! preemption, the watchdog bypass, progress throttling and background priority.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ove_tasks::{
    Class, Limits, Options, PartFile, PauseReason, Priority, Scheduler, TaskCtx, TaskError, TaskEvent, TaskFilter,
    TaskSpec, TaskState,
};

const MB: u64 = 1024 * 1024;

fn scheduler() -> Scheduler {
    scheduler_with(|_| {})
}

fn scheduler_with(f: impl FnOnce(&mut Limits)) -> Scheduler {
    let mut limits = Limits::for_machine(8, Some(16 * 1024 * MB));
    limits.tick = Duration::from_millis(20);
    f(&mut limits);
    Scheduler::new(Options { limits, background_threads: true })
}

fn spec(kind: &str, priority: Priority, class: Class) -> TaskSpec {
    TaskSpec::new(kind, format!("{kind} task"), priority, class).project("p1")
}

/// A task that runs `ms` milliseconds in 5 ms steps, checking in at every step.
fn steps(ms: u64) -> impl FnOnce(&TaskCtx) -> Result<(), TaskError> + Send + 'static {
    move |ctx| {
        for i in 0..ms / 5 {
            ctx.checkpoint()?;
            ctx.progress(i as f64 * 5.0 / ms as f64);
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
}

/// Polls `cond` until true or `limit` passes; returns the time it took.
fn within(limit: Duration, mut cond: impl FnMut() -> bool) -> Option<Duration> {
    let start = Instant::now();
    while start.elapsed() < limit {
        if cond() {
            return Some(start.elapsed());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    None
}

fn state(s: &Scheduler, id: u64) -> Option<TaskState> {
    s.get(id).map(|t| t.state)
}

#[test]
fn tasks_run_and_report_done() {
    let s = scheduler();
    let id = s.submit(spec("thumbnail", Priority::Derivative, Class::Decode), steps(20));
    let info = s.wait_task(id, Duration::from_secs(5)).unwrap();
    assert_eq!(info.state, TaskState::Done);
    assert_eq!(info.progress, Some(1.0));
    let failed =
        s.submit(spec("probe", Priority::Interactive, Class::Io), |_| Err(TaskError::Failed("no such file".into())));
    let info = s.wait_task(failed, Duration::from_secs(5)).unwrap();
    assert_eq!((info.state, info.error.as_deref()), (TaskState::Failed, Some("no such file")));
}

#[test]
fn same_inputs_key_runs_once() {
    let s = scheduler();
    let runs = Arc::new(AtomicUsize::new(0));
    let ids: Vec<u64> = (0..3)
        .map(|_| {
            let r = runs.clone();
            s.submit(spec("skim", Priority::Derivative, Class::Decode).key("skim:abc"), move |ctx| {
                r.fetch_add(1, Ordering::SeqCst);
                steps(30)(ctx)
            })
        })
        .collect();
    assert!(ids.iter().all(|&i| i == ids[0]));
    assert!(s.wait(&TaskFilter::default(), Duration::from_secs(5)).done);
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[test]
fn a_panicking_task_fails_and_others_continue() {
    let s = scheduler();
    let bad = s.submit(spec("captions", Priority::Analysis, Class::Cpu), |_| panic!("injected"));
    let good = s.submit(spec("thumbnail", Priority::Derivative, Class::Cpu), steps(10));
    assert_eq!(s.wait_task(bad, Duration::from_secs(5)).unwrap().state, TaskState::Failed);
    assert_eq!(s.wait_task(good, Duration::from_secs(5)).unwrap().state, TaskState::Done);
}

#[test]
fn cancel_stops_within_200ms_and_leaves_no_output() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("proxies/abc_1080.mp4");
    let s = scheduler();
    let target = out.clone();
    let id = s.submit(spec("proxy", Priority::Derivative, Class::Decode), move |ctx| {
        let part = PartFile::new(&target).map_err(|e| TaskError::Failed(e.to_string()))?;
        loop {
            ctx.checkpoint()?;
            std::fs::write(part.path(), b"frames so far").map_err(|e| TaskError::Failed(e.to_string()))?;
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    let other = s.submit(spec("thumbnail", Priority::Derivative, Class::Io), steps(300));
    within(Duration::from_secs(2), || state(&s, id) == Some(TaskState::Running)).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let t = Instant::now();
    assert!(s.cancel(id));
    let info = s.wait_task(id, Duration::from_secs(2)).unwrap();
    let took = t.elapsed();
    assert_eq!(info.state, TaskState::Cancelled);
    assert!(took <= Duration::from_millis(200), "cancel took {took:?}");
    assert!(!out.exists(), "no final output");
    assert!(!ove_tasks::part_path(&out).exists(), "partial file removed");
    assert_eq!(state(&s, other), Some(TaskState::Running), "other tasks continue");
    println!("cancel latency: {took:?}");
}

#[test]
fn a_cancelled_queued_task_never_runs() {
    let s = scheduler_with(|l| l.decode = 1);
    let blocker = s.submit(spec("proxy", Priority::Derivative, Class::Decode), steps(200));
    let ran = Arc::new(AtomicUsize::new(0));
    let r = ran.clone();
    let queued = s.submit(spec("proxy", Priority::Derivative, Class::Decode), move |_| {
        r.fetch_add(1, Ordering::SeqCst);
        Ok(())
    });
    assert_eq!(state(&s, queued), Some(TaskState::Queued));
    assert!(s.cancel(queued));
    assert_eq!(state(&s, queued), Some(TaskState::Cancelled));
    s.wait_task(blocker, Duration::from_secs(5)).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(ran.load(Ordering::SeqCst), 0);
    assert!(!s.cancel(queued), "already finished");
}

#[test]
fn playback_pauses_derivative_work_within_200ms_and_resumes() {
    let s = scheduler();
    let skim = s.submit(spec("skim", Priority::Derivative, Class::Decode), steps(400));
    let probe = s.submit(spec("probe", Priority::Interactive, Class::Io), steps(400));
    within(Duration::from_secs(2), || state(&s, skim) == Some(TaskState::Running)).unwrap();
    s.set_pause(PauseReason::Playback, true);
    let took = within(Duration::from_secs(1), || state(&s, skim) == Some(TaskState::Paused)).unwrap();
    assert!(took <= Duration::from_millis(200), "pause took {took:?}");
    let info = s.get(skim).unwrap();
    assert_eq!(info.pause_reason, Some(PauseReason::Playback));
    assert_eq!(state(&s, probe), Some(TaskState::Running), "interactive work does not pause for playback");
    // A paused task makes no progress.
    let p0 = s.get(skim).unwrap().progress;
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(s.get(skim).unwrap().progress, p0);
    s.set_pause(PauseReason::Playback, false);
    within(Duration::from_secs(1), || state(&s, skim) == Some(TaskState::Running)).unwrap();
    assert_eq!(s.wait_task(skim, Duration::from_secs(5)).unwrap().state, TaskState::Done);
    println!("pause latency: {took:?}");
}

#[test]
fn queued_work_shows_the_pause_reason_and_waits() {
    let s = scheduler();
    s.set_pause(PauseReason::Battery, true);
    let id = s.submit(spec("captions", Priority::Analysis, Class::Cpu), steps(10));
    std::thread::sleep(Duration::from_millis(80));
    let info = s.get(id).unwrap();
    assert_eq!((info.state, info.pause_reason), (TaskState::Paused, Some(PauseReason::Battery)));
    assert_eq!(PauseReason::Battery.label(), "on battery");
    // "Run anyway".
    s.set_override(PauseReason::Battery, true);
    assert!(s.pause_reasons().is_empty());
    assert_eq!(s.wait_task(id, Duration::from_secs(5)).unwrap().state, TaskState::Done);
}

#[test]
fn every_pause_source_pauses_and_resumes() {
    for reason in
        [PauseReason::Playback, PauseReason::Battery, PauseReason::LowPower, PauseReason::Thermal, PauseReason::User]
    {
        let s = scheduler();
        let id = s.submit(spec("index", Priority::Analysis, Class::Cpu), steps(300));
        within(Duration::from_secs(2), || state(&s, id) == Some(TaskState::Running)).unwrap();
        s.set_pause(reason, true);
        within(Duration::from_millis(500), || s.get(id).unwrap().pause_reason == Some(reason))
            .unwrap_or_else(|| panic!("{reason:?} did not pause"));
        s.set_pause(reason, false);
        within(Duration::from_millis(500), || state(&s, id) == Some(TaskState::Running))
            .unwrap_or_else(|| panic!("{reason:?} did not resume"));
        s.cancel(id);
    }
}

#[test]
fn the_persons_pause_holds_interactive_work_but_not_export() {
    let s = scheduler();
    s.set_pause(PauseReason::User, true);
    let probe = s.submit(spec("probe", Priority::Interactive, Class::Io), steps(10));
    let export = s.submit(spec("export", Priority::Export, Class::Decode), steps(10));
    assert_eq!(s.wait_task(export, Duration::from_secs(5)).unwrap().state, TaskState::Done);
    assert_eq!(state(&s, probe), Some(TaskState::Paused));
    s.set_pause(PauseReason::User, false);
    assert_eq!(s.wait_task(probe, Duration::from_secs(5)).unwrap().state, TaskState::Done);
}

#[test]
fn export_starts_within_1s_and_indexing_yields_its_slot() {
    // One decode session, held by an analysis task: the export needs it.
    let s = scheduler_with(|l| l.decode = 1);
    let index = s.submit(spec("index", Priority::Analysis, Class::Decode), steps(5000));
    within(Duration::from_secs(2), || state(&s, index) == Some(TaskState::Running)).unwrap();
    let t = Instant::now();
    let export = s.submit(spec("export", Priority::Export, Class::Decode), steps(100));
    let started = within(Duration::from_secs(2), || state(&s, export) == Some(TaskState::Running)).unwrap();
    assert!(started <= Duration::from_secs(1), "export started after {started:?}");
    let info = s.get(index).unwrap();
    assert_eq!((info.state, info.pause_reason), (TaskState::Paused, Some(PauseReason::Export)));
    assert_eq!(s.wait_task(export, Duration::from_secs(5)).unwrap().state, TaskState::Done);
    within(Duration::from_secs(1), || state(&s, index) == Some(TaskState::Running)).expect("indexing resumes");
    println!("export start latency under load: {:?}", t.elapsed().min(started));
    s.cancel(index);
}

#[test]
fn a_stalled_queue_still_starts_export_through_the_watchdog() {
    let s = scheduler();
    s.set_stalled(true);
    let t = Instant::now();
    let id = s.submit(spec("export", Priority::Export, Class::Decode).watchdog(Duration::from_millis(300)), steps(20));
    let info = s.wait_task(id, Duration::from_secs(5)).unwrap();
    assert_eq!(info.state, TaskState::Done);
    assert!(info.bypassed);
    let took = t.elapsed();
    assert!(took >= Duration::from_millis(300) && took < Duration::from_secs(2), "{took:?}");
    // A task the queue starts in time is not bypassed.
    s.set_stalled(false);
    let id = s.submit(spec("export", Priority::Export, Class::Decode).watchdog(Duration::from_millis(300)), steps(20));
    assert!(!s.wait_task(id, Duration::from_secs(5)).unwrap().bypassed);
}

#[test]
fn a_task_larger_than_the_budget_runs_alone() {
    let s = scheduler_with(|l| l.mem_budget = 100 * MB);
    let big = s.submit(spec("proxy", Priority::Derivative, Class::Decode).memory(500 * MB), steps(150));
    within(Duration::from_secs(2), || state(&s, big) == Some(TaskState::Running)).unwrap();
    assert!(s.get(big).unwrap().running_alone);
    let small = s.submit(spec("thumbnail", Priority::Derivative, Class::Io).memory(MB), steps(10));
    std::thread::sleep(Duration::from_millis(60));
    assert_eq!(state(&s, small), Some(TaskState::Queued), "nothing starts beside it");
    assert_eq!(s.wait_task(small, Duration::from_secs(5)).unwrap().state, TaskState::Done);
}

#[test]
fn budget_holds_back_tasks_that_do_not_fit() {
    let s = scheduler_with(|l| l.mem_budget = 100 * MB);
    let a = s.submit(spec("skim", Priority::Derivative, Class::Cpu).memory(60 * MB), steps(150));
    let b = s.submit(spec("skim", Priority::Derivative, Class::Cpu).memory(60 * MB), steps(10));
    within(Duration::from_secs(2), || state(&s, a) == Some(TaskState::Running)).unwrap();
    std::thread::sleep(Duration::from_millis(40));
    assert_eq!(state(&s, b), Some(TaskState::Queued));
    s.wait_task(a, Duration::from_secs(5)).unwrap();
    assert_eq!(s.wait_task(b, Duration::from_secs(5)).unwrap().state, TaskState::Done);
}

#[test]
fn progress_events_are_throttled_to_4_per_second() {
    let s = scheduler();
    let updates = Arc::new(Mutex::new(Vec::new()));
    let u = updates.clone();
    s.subscribe(move |ev| {
        if let TaskEvent::Updated(t) = ev
            && t.state == TaskState::Running
            && t.progress.is_some()
        {
            u.lock().unwrap().push(t.id);
        }
    });
    let id = s.submit(spec("captions", Priority::Analysis, Class::Cpu), |ctx| {
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(1000) {
            ctx.checkpoint()?;
            ctx.progress(start.elapsed().as_secs_f64());
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    });
    s.wait_task(id, Duration::from_secs(5)).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let n = updates.lock().unwrap().iter().filter(|&&i| i == id).count();
    assert!((2..=6).contains(&n), "{n} progress events in one second");
}

#[test]
fn wait_times_out_with_the_remaining_count() {
    let s = scheduler();
    s.set_pause(PauseReason::Playback, true);
    for _ in 0..3 {
        s.submit(spec("index", Priority::Analysis, Class::Cpu), steps(10));
    }
    s.submit(TaskSpec::new("index", "other project", Priority::Analysis, Class::Cpu).project("p2"), steps(10));
    let filter = TaskFilter { project: Some("p1".into()), kind: Some("index".into()) };
    let t = Instant::now();
    let w = s.wait(&filter, Duration::from_millis(100));
    assert!(!w.done && w.remaining == 3, "{w:?}");
    assert!(t.elapsed() >= Duration::from_millis(100));
    s.set_pause(PauseReason::Playback, false);
    let w = s.wait(&filter, Duration::from_secs(5));
    assert!(w.done && w.remaining == 0);
    assert_eq!(s.cancel_matching(&TaskFilter::default()), 0);
}

#[test]
fn hovering_promotes_a_queued_sheet_past_paused_derivatives() {
    let s = scheduler();
    s.set_pause(PauseReason::Playback, true);
    let id = s.submit(spec("skim", Priority::Derivative, Class::Decode).key("skim:x"), steps(10));
    std::thread::sleep(Duration::from_millis(40));
    assert_eq!(state(&s, id), Some(TaskState::Paused));
    assert_eq!(s.find_key("skim:x"), Some(id));
    assert!(s.promote(id, Priority::Interactive));
    assert!(!s.promote(id, Priority::Analysis), "never lowered");
    assert_eq!(s.wait_task(id, Duration::from_secs(5)).unwrap().state, TaskState::Done);
}

#[test]
fn task_threads_run_at_background_priority_except_export() {
    let s = scheduler();
    let seen = Arc::new(Mutex::new(Vec::new()));
    for p in [Priority::Export, Priority::Derivative, Priority::Analysis] {
        let seen = seen.clone();
        let id = s.submit(spec("probe", p, Class::Io), move |_| {
            seen.lock().unwrap().push((p, ove_tasks::os::is_background()));
            Ok(())
        });
        s.wait_task(id, Duration::from_secs(5)).unwrap();
    }
    let seen = seen.lock().unwrap().clone();
    if cfg!(any(windows, target_os = "macos")) {
        assert_eq!(seen, vec![(Priority::Export, false), (Priority::Derivative, true), (Priority::Analysis, true)]);
    }
}

#[test]
fn finished_tasks_are_listed_and_serialise_for_surfaces() {
    let s = scheduler();
    let id = s.submit(spec("thumbnail", Priority::Derivative, Class::Io), steps(10));
    s.wait_task(id, Duration::from_secs(5)).unwrap();
    assert!(s.list(&TaskFilter::default()).is_empty());
    let done = s.finished(&TaskFilter::default());
    assert_eq!(done[0].id, id);
    let json = serde_json::to_value(&done[0]).unwrap();
    assert_eq!(json["state"], "done");
    assert_eq!(json["priority"], "derivative");
    assert_eq!(json["class"], "io");
    assert_eq!(json["project"], "p1");
}

#[test]
fn dropping_the_scheduler_cancels_running_work() {
    let s = scheduler();
    let stopped = Arc::new(AtomicUsize::new(0));
    let st = stopped.clone();
    s.submit(spec("index", Priority::Analysis, Class::Cpu), move |ctx| {
        let r = steps(10_000)(ctx);
        if r == Err(TaskError::Cancelled) {
            st.fetch_add(1, Ordering::SeqCst);
        }
        r
    });
    std::thread::sleep(Duration::from_millis(50));
    drop(s);
    within(Duration::from_secs(1), || stopped.load(Ordering::SeqCst) == 1).expect("the task stopped");
}

#[test]
fn a_callers_token_cancels_a_queued_task_without_running_it() {
    let s = scheduler_with(|l| l.decode = 1);
    let blocker = s.submit(spec("index", Priority::Analysis, Class::Decode), steps(2000));
    let token = ove_tasks::CancelToken::new();
    let ran = Arc::new(AtomicUsize::new(0));
    let r = ran.clone();
    let id = s.submit_with(spec("export", Priority::Derivative, Class::Decode), token.clone(), move |_| {
        r.fetch_add(1, Ordering::SeqCst);
        Ok(())
    });
    token.cancel();
    assert_eq!(s.wait_task(id, Duration::from_secs(2)).unwrap().state, TaskState::Cancelled);
    assert_eq!(ran.load(Ordering::SeqCst), 0);
    // Cancelling the task sets the caller's token too.
    let token2 = ove_tasks::CancelToken::new();
    let id2 = s.submit_with(spec("export", Priority::Derivative, Class::Decode), token2.clone(), |_| Ok(()));
    s.cancel(id2);
    assert!(token2.is_cancelled());
    s.cancel(blocker);
}
