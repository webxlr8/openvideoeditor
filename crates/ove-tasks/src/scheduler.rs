//! The scheduler: a dispatcher thread admits queued tasks (see [`crate::queue`]) and runs each on its own thread at
//! background priority. Tasks call [`TaskCtx::checkpoint`] per frame or chunk: that is where they stop when
//! cancelled and wait while paused. A paused task gives its class slot and memory back until it resumes, so an
//! export never waits behind paused work.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use crate::cancel::CancelToken;
use crate::model::{
    Cancelled, PauseReason, Priority, TaskError, TaskEvent, TaskFilter, TaskId, TaskInfo, TaskSpec, TaskState,
    WaitOutcome,
};
use crate::queue::{self, Candidate, Limits, Usage};

/// A task body. It returns `Err(TaskError::Cancelled)` when a checkpoint reported cancellation.
pub type Job = Box<dyn FnOnce(&TaskCtx) -> Result<(), TaskError> + Send + 'static>;

type Listener = Box<dyn Fn(&TaskEvent) + Send + Sync + 'static>;

/// How many finished tasks are kept for listing and waiting.
const FINISHED_KEPT: usize = 256;

/// Scheduler options beyond the limits.
#[derive(Debug, Clone)]
pub struct Options {
    pub limits: Limits,
    /// Run task threads (except export) at the operating system's background priority.
    pub background_threads: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { limits: Limits::detect(), background_threads: true }
    }
}

/// The background task scheduler. Clones share one queue; the last clone dropped cancels everything left.
#[derive(Clone)]
pub struct Scheduler {
    inner: Arc<Inner>,
    _guard: Arc<ShutdownGuard>,
}

impl std::fmt::Debug for Scheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler").field("limits", &self.inner.limits).finish_non_exhaustive()
    }
}

struct ShutdownGuard(Arc<Inner>);

impl Drop for ShutdownGuard {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

struct Inner {
    limits: Limits,
    background_threads: bool,
    epoch: Instant,
    state: Mutex<State>,
    cv: Condvar,
    listeners: Arc<Mutex<Vec<Listener>>>,
    events: Mutex<Sender<TaskEvent>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Queued,
    Running,
    /// Stopped at a checkpoint; holds no slot or memory.
    Paused,
    /// The pause ended; waiting for the dispatcher to hand its slot back.
    Resuming,
}

struct Slot {
    claimed: AtomicBool,
    cancel: CancelToken,
    job: Mutex<Option<Job>>,
}

struct Entry {
    spec: TaskSpec,
    priority: Priority,
    queued_at: Duration,
    seq: u64,
    phase: Phase,
    alone: bool,
    bypassed: bool,
    pause_reason: Option<PauseReason>,
    progress: Option<f64>,
    last_progress_event: Option<Instant>,
    slot: Arc<Slot>,
}

impl Entry {
    fn holds(&self) -> bool {
        self.phase == Phase::Running
    }
}

#[derive(Default)]
struct State {
    next_id: TaskId,
    seq: u64,
    tasks: BTreeMap<TaskId, Entry>,
    finished: VecDeque<TaskInfo>,
    sources: BTreeSet<PauseReason>,
    overrides: BTreeSet<PauseReason>,
    stalled: bool,
    shutdown: bool,
}

impl State {
    /// Active pause reasons after overrides. An unfinished export task adds `Export` by itself.
    fn reasons(&self) -> Vec<PauseReason> {
        let export = self.tasks.values().any(|e| e.priority == Priority::Export);
        PauseReason::ALL
            .into_iter()
            .filter(|r| self.sources.contains(r) || (*r == PauseReason::Export && export))
            .filter(|r| !self.overrides.contains(r))
            .collect()
    }

    fn pause_for(&self, priority: Priority) -> Option<PauseReason> {
        self.reasons().into_iter().find(|r| r.applies_to(priority))
    }

    fn usage(&self) -> Usage {
        let mut u = Usage::default();
        for e in self.tasks.values().filter(|e| e.holds()) {
            u.hold(e.spec.class, e.spec.est_mem, e.alone);
        }
        u
    }

    fn info(&self, id: TaskId, e: &Entry) -> TaskInfo {
        let (state, pause_reason) = match e.phase {
            Phase::Running => (TaskState::Running, None),
            Phase::Paused => (TaskState::Paused, e.pause_reason),
            Phase::Queued | Phase::Resuming => match self.pause_for(e.priority) {
                Some(r) => (TaskState::Paused, Some(r)),
                None => (TaskState::Queued, None),
            },
        };
        TaskInfo {
            id,
            kind: e.spec.kind.clone(),
            label: e.spec.label.clone(),
            project: e.spec.project.clone(),
            priority: e.priority,
            class: e.spec.class,
            state,
            pause_reason,
            progress: e.progress,
            running_alone: e.alone,
            bypassed: e.bypassed,
            error: None,
            est_mem: e.spec.est_mem,
        }
    }
}

impl Scheduler {
    pub fn new(options: Options) -> Scheduler {
        let (tx, rx) = channel::<TaskEvent>();
        let listeners: Arc<Mutex<Vec<Listener>>> = Arc::default();
        let inner = Arc::new(Inner {
            limits: options.limits,
            background_threads: options.background_threads,
            epoch: Instant::now(),
            state: Mutex::new(State { next_id: 1, ..State::default() }),
            cv: Condvar::new(),
            listeners: listeners.clone(),
            events: Mutex::new(tx),
        });
        // Events go out on their own thread, in order, so listeners may call back into the scheduler.
        let spawned = std::thread::Builder::new().name("tasks-events".into()).spawn(move || {
            for ev in rx {
                for l in relock(&listeners).iter() {
                    l(&ev);
                }
            }
        });
        if let Err(e) = spawned {
            tracing::error!("could not start the task event thread: {e}");
        }
        let weak = Arc::downgrade(&inner);
        let spawned = std::thread::Builder::new().name("tasks-dispatch".into()).spawn(move || dispatch(weak));
        if let Err(e) = spawned {
            tracing::error!("could not start the task dispatcher: {e}");
        }
        Scheduler { _guard: Arc::new(ShutdownGuard(inner.clone())), inner }
    }

    pub fn limits(&self) -> &Limits {
        &self.inner.limits
    }

    /// Calls `listener` for every event, on the scheduler's event thread.
    pub fn subscribe(&self, listener: impl Fn(&TaskEvent) + Send + Sync + 'static) {
        relock(&self.inner.listeners).push(Box::new(listener));
    }

    /// Queues a task. A task with the same inputs key that is still unfinished is returned instead (its job runs once).
    pub fn submit(
        &self,
        spec: TaskSpec,
        job: impl FnOnce(&TaskCtx) -> Result<(), TaskError> + Send + 'static,
    ) -> TaskId {
        let inner = &self.inner;
        let (id, watchdog, slot) = {
            let mut st = inner.lock();
            if let Some(key) = &spec.key
                && let Some((&id, _)) = st.tasks.iter().find(|(_, e)| e.spec.key.as_ref() == Some(key))
            {
                return id;
            }
            let before = st.reasons();
            let id = st.next_id;
            st.next_id += 1;
            st.seq += 1;
            let slot =
                Arc::new(Slot { claimed: AtomicBool::new(false), cancel: CancelToken::new(), job: Mutex::new(None) });
            *relock(&slot.job) = Some(Box::new(job));
            let entry = Entry {
                priority: spec.priority,
                queued_at: inner.now(),
                seq: st.seq,
                phase: Phase::Queued,
                alone: false,
                bypassed: false,
                pause_reason: None,
                progress: None,
                last_progress_event: None,
                slot: slot.clone(),
                spec,
            };
            let watchdog = entry.spec.watchdog;
            st.tasks.insert(id, entry);
            inner.updated(&st, id);
            inner.pause_changed(&st, &before);
            (id, watchdog, slot)
        };
        inner.cv.notify_all();
        if let Some(after) = watchdog {
            let weak = Arc::downgrade(inner);
            let spawned = std::thread::Builder::new().name("tasks-watchdog".into()).spawn(move || {
                std::thread::sleep(after);
                if let Some(inner) = weak.upgrade() {
                    watchdog_start(&inner, id, slot);
                }
            });
            if let Err(e) = spawned {
                tracing::error!("could not start a task watchdog: {e}");
            }
        }
        id
    }

    /// Cancels a task. A queued task is dropped without running; a running or paused one stops at its next
    /// checkpoint. Returns false when the task is unknown or already finished.
    pub fn cancel(&self, id: TaskId) -> bool {
        let inner = &self.inner;
        let found = {
            let mut st = inner.lock();
            let Some(e) = st.tasks.get(&id) else { return false };
            e.slot.cancel.cancel();
            if e.phase == Phase::Queued
                && e.slot.claimed.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_ok()
            {
                inner.finish_locked(&mut st, id, Err(TaskError::Cancelled));
            }
            true
        };
        inner.cv.notify_all();
        found
    }

    /// Cancels every unfinished task matching `filter`; returns how many.
    pub fn cancel_matching(&self, filter: &TaskFilter) -> usize {
        let ids: Vec<TaskId> = self.list(filter).into_iter().map(|t| t.id).collect();
        ids.into_iter().filter(|&id| self.cancel(id)).count()
    }

    /// Unfinished tasks matching `filter`, most urgent first.
    pub fn list(&self, filter: &TaskFilter) -> Vec<TaskInfo> {
        let st = self.inner.lock();
        let mut v: Vec<TaskInfo> = st
            .tasks
            .iter()
            .filter(|(_, e)| filter.matches(e.spec.project.as_deref(), &e.spec.kind))
            .map(|(&id, e)| st.info(id, e))
            .collect();
        v.sort_by_key(|t| (t.state != TaskState::Running, t.priority, t.id));
        v
    }

    /// Recently finished tasks matching `filter`, newest first.
    pub fn finished(&self, filter: &TaskFilter) -> Vec<TaskInfo> {
        let st = self.inner.lock();
        st.finished.iter().rev().filter(|t| filter.matches(t.project.as_deref(), &t.kind)).cloned().collect()
    }

    /// One task, unfinished or recently finished.
    pub fn get(&self, id: TaskId) -> Option<TaskInfo> {
        let st = self.inner.lock();
        st.tasks.get(&id).map(|e| st.info(id, e)).or_else(|| st.finished.iter().rev().find(|t| t.id == id).cloned())
    }

    /// The unfinished task with this inputs key.
    pub fn find_key(&self, key: &str) -> Option<TaskId> {
        let st = self.inner.lock();
        st.tasks.iter().find(|(_, e)| e.spec.key.as_deref() == Some(key)).map(|(&id, _)| id)
    }

    /// Raises a task's priority (never lowers it), for example a skim sheet the person is hovering.
    pub fn promote(&self, id: TaskId, priority: Priority) -> bool {
        let inner = &self.inner;
        let changed = {
            let mut st = inner.lock();
            let before = st.reasons();
            let Some(e) = st.tasks.get_mut(&id) else { return false };
            if priority >= e.priority {
                return false;
            }
            e.priority = priority;
            inner.updated(&st, id);
            inner.pause_changed(&st, &before);
            true
        };
        inner.cv.notify_all();
        changed
    }

    /// Waits until every task matching `filter` has finished, or the time-out passes.
    pub fn wait(&self, filter: &TaskFilter, timeout: Duration) -> WaitOutcome {
        let inner = &self.inner;
        let deadline = Instant::now() + timeout;
        let mut st = inner.lock();
        loop {
            let remaining =
                st.tasks.values().filter(|e| filter.matches(e.spec.project.as_deref(), &e.spec.kind)).count();
            if remaining == 0 {
                return WaitOutcome { done: true, remaining: 0 };
            }
            let now = Instant::now();
            if now >= deadline {
                return WaitOutcome { done: false, remaining };
            }
            st = inner.wait_timeout(st, deadline - now);
        }
    }

    /// Waits for one task to finish and returns its final state; `None` at the time-out or for an unknown id.
    pub fn wait_task(&self, id: TaskId, timeout: Duration) -> Option<TaskInfo> {
        let inner = &self.inner;
        let deadline = Instant::now() + timeout;
        let mut st = inner.lock();
        loop {
            if !st.tasks.contains_key(&id) {
                return st.finished.iter().rev().find(|t| t.id == id).cloned();
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            st = inner.wait_timeout(st, deadline - now);
        }
    }

    /// Turns a pause source on or off (playback, battery, the person's pause, …).
    pub fn set_pause(&self, reason: PauseReason, active: bool) {
        self.change_pause(|st| if active { st.sources.insert(reason) } else { st.sources.remove(&reason) });
    }

    /// "Run anyway": ignore one pause reason until the override is lifted.
    pub fn set_override(&self, reason: PauseReason, run_anyway: bool) {
        self.change_pause(|st| if run_anyway { st.overrides.insert(reason) } else { st.overrides.remove(&reason) });
    }

    /// Active pause reasons, after overrides.
    pub fn pause_reasons(&self) -> Vec<PauseReason> {
        self.inner.lock().reasons()
    }

    /// Reasons the person chose to run anyway.
    pub fn overrides(&self) -> Vec<PauseReason> {
        self.inner.lock().overrides.iter().copied().collect()
    }

    /// Fault injection: a stalled scheduler admits nothing (watchdogs still fire).
    pub fn set_stalled(&self, stalled: bool) {
        self.inner.lock().stalled = stalled;
        self.inner.cv.notify_all();
    }

    /// Polls the power source, low-power mode and thermal state every `interval` and pauses accordingly. The thread
    /// ends with the scheduler.
    pub fn watch_power(&self, interval: Duration) {
        let weak = Arc::downgrade(&self.inner);
        let spawned = std::thread::Builder::new().name("tasks-power".into()).spawn(move || {
            loop {
                let state = crate::os::power_state();
                let Some(inner) = weak.upgrade() else { return };
                if inner.lock().shutdown {
                    return;
                }
                let s = Scheduler::from_inner(inner);
                s.set_pause(PauseReason::Battery, state.on_battery == Some(true));
                s.set_pause(PauseReason::LowPower, state.low_power == Some(true));
                s.set_pause(PauseReason::Thermal, state.thermal_pressure == Some(true));
                drop(s);
                std::thread::sleep(interval);
            }
        });
        if let Err(e) = spawned {
            tracing::error!("could not start the power watcher: {e}");
        }
    }

    /// A handle that does not own the queue's lifetime.
    fn from_inner(inner: Arc<Inner>) -> WeakHandle {
        WeakHandle(inner)
    }

    fn change_pause(&self, change: impl FnOnce(&mut State) -> bool) {
        change_pause(&self.inner, change);
    }
}

/// Internal handle for helper threads: changes state without keeping the scheduler alive.
struct WeakHandle(Arc<Inner>);

impl WeakHandle {
    fn set_pause(&self, reason: PauseReason, active: bool) {
        change_pause(&self.0, |st| if active { st.sources.insert(reason) } else { st.sources.remove(&reason) });
    }
}

fn change_pause(inner: &Arc<Inner>, change: impl FnOnce(&mut State) -> bool) {
    {
        let mut st = inner.lock();
        let before = st.reasons();
        if !change(&mut st) {
            return;
        }
        inner.pause_changed(&st, &before);
        let ids: Vec<TaskId> = st.tasks.keys().copied().collect();
        for id in ids {
            inner.updated(&st, id);
        }
    }
    inner.cv.notify_all();
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        relock(&self.state)
    }

    fn wait_timeout<'a>(&self, g: MutexGuard<'a, State>, d: Duration) -> MutexGuard<'a, State> {
        match self.cv.wait_timeout(g, d) {
            Ok((g, _)) => g,
            Err(poisoned) => poisoned.into_inner().0,
        }
    }

    fn now(&self) -> Duration {
        self.epoch.elapsed()
    }

    fn send(&self, ev: TaskEvent) {
        let _ = relock(&self.events).send(ev);
    }

    fn updated(&self, st: &State, id: TaskId) {
        if let Some(e) = st.tasks.get(&id) {
            self.send(TaskEvent::Updated(st.info(id, e)));
        }
    }

    fn pause_changed(&self, st: &State, before: &[PauseReason]) {
        let after = st.reasons();
        if after != before {
            self.send(TaskEvent::PauseChanged(after));
        }
    }

    /// Removes a task and records how it ended.
    fn finish_locked(&self, st: &mut State, id: TaskId, result: Result<(), TaskError>) {
        let before = st.reasons();
        let Some(e) = st.tasks.get(&id) else { return };
        let mut info = st.info(id, e);
        st.tasks.remove(&id);
        info.pause_reason = None;
        match result {
            Ok(()) => {
                info.state = TaskState::Done;
                info.progress = Some(1.0);
            }
            Err(TaskError::Cancelled) => info.state = TaskState::Cancelled,
            Err(TaskError::Failed(m)) => {
                info.state = TaskState::Failed;
                info.error = Some(m);
            }
        }
        if st.finished.len() >= FINISHED_KEPT {
            st.finished.pop_front();
        }
        st.finished.push_back(info.clone());
        self.send(TaskEvent::Finished(info));
        self.pause_changed(st, &before);
    }

    fn finish(&self, id: TaskId, result: Result<(), TaskError>) {
        {
            let mut st = self.lock();
            self.finish_locked(&mut st, id, result);
        }
        self.cv.notify_all();
    }

    fn shutdown(&self) {
        {
            let mut st = self.lock();
            st.shutdown = true;
            for e in st.tasks.values() {
                e.slot.cancel.cancel();
            }
        }
        self.cv.notify_all();
    }
}

/// The dispatcher: admits tasks whenever something changes, and at least every `tick` (for ageing).
fn dispatch(weak: Weak<Inner>) {
    loop {
        let Some(inner) = weak.upgrade() else { return };
        let mut start: Vec<(TaskId, Arc<Slot>, Priority, String)> = vec![];
        let mut st = inner.lock();
        if st.shutdown {
            return;
        }
        let mut resumed = false;
        while !st.stalled {
            let now = inner.now();
            let candidates: Vec<Candidate> = st
                .tasks
                .iter()
                .filter(|(_, e)| matches!(e.phase, Phase::Queued | Phase::Resuming))
                .map(|(&id, e)| Candidate {
                    id,
                    priority: e.priority,
                    class: e.spec.class,
                    est_mem: e.spec.est_mem,
                    queued_at: e.queued_at,
                    seq: e.seq,
                })
                .collect();
            let usage = st.usage();
            let Some(admit) = queue::next(&candidates, &usage, now, &inner.limits, |p| st.pause_for(p).is_some())
            else {
                break;
            };
            let id = candidates[admit.index].id;
            let Some(e) = st.tasks.get_mut(&id) else { break };
            match e.phase {
                Phase::Queued => {
                    if e.slot.claimed.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
                        // Its watchdog started it already; it counts as running.
                        e.phase = Phase::Running;
                        continue;
                    }
                    start.push((id, e.slot.clone(), e.priority, e.spec.kind.clone()));
                }
                _ => resumed = true,
            }
            e.phase = Phase::Running;
            e.alone = admit.alone;
            inner.updated(&st, id);
        }
        if start.is_empty() && !resumed {
            let tick = inner.limits.tick;
            drop(inner.wait_timeout(st, tick));
            continue;
        }
        drop(st);
        if resumed {
            inner.cv.notify_all();
        }
        for (id, slot, priority, kind) in start {
            spawn_task(&inner, id, slot, priority, &kind);
        }
    }
}

fn spawn_task(inner: &Arc<Inner>, id: TaskId, slot: Arc<Slot>, priority: Priority, kind: &str) {
    let worker = inner.clone();
    let spawned = std::thread::Builder::new().name(format!("task-{kind}")).spawn(move || {
        run_task(&worker, id, slot, priority);
    });
    if let Err(e) = spawned {
        tracing::error!(task = id, "could not start a task thread: {e}");
        inner.finish(id, Err(TaskError::Failed(format!("Could not start the task: {e}"))));
    }
}

fn run_task(inner: &Arc<Inner>, id: TaskId, slot: Arc<Slot>, priority: Priority) {
    if inner.background_threads && priority != Priority::Export && !crate::os::enter_background() {
        tracing::debug!(task = id, "background priority not available");
    }
    let job = relock(&slot.job).take();
    let ctx = TaskCtx { inner: inner.clone(), id, cancel: slot.cancel.clone() };
    let result = match job {
        Some(job) => catch_unwind(AssertUnwindSafe(|| job(&ctx))).unwrap_or_else(|payload| {
            let message = panic_message(payload.as_ref());
            tracing::error!(task = id, "background task panicked: {message}");
            Err(TaskError::Failed(format!("An internal error stopped the task: {message}")))
        }),
        None => Err(TaskError::Failed("The task had no work to run.".into())),
    };
    inner.finish(id, result);
}

/// Starts a task its scheduler has not started in time, outside the queue's limits.
fn watchdog_start(inner: &Arc<Inner>, id: TaskId, slot: Arc<Slot>) {
    if slot.claimed.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return;
    }
    let priority = {
        let mut st = inner.lock();
        let Some(e) = st.tasks.get_mut(&id) else { return };
        e.phase = Phase::Running;
        e.bypassed = true;
        let p = e.priority;
        tracing::warn!(
            task = id,
            kind = e.spec.kind,
            "the scheduler did not start the task in time; starting it directly"
        );
        inner.updated(&st, id);
        p
    };
    inner.cv.notify_all();
    run_task(inner, id, slot, priority);
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

fn relock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// What a running task sees: its id, its cancel token, checkpoints and progress reporting.
pub struct TaskCtx {
    inner: Arc<Inner>,
    id: TaskId,
    cancel: CancelToken,
}

impl std::fmt::Debug for TaskCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskCtx").field("id", &self.id).finish_non_exhaustive()
    }
}

impl TaskCtx {
    pub fn id(&self) -> TaskId {
        self.id
    }

    pub fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Call per frame or chunk. Returns `Err(Cancelled)` once the task is cancelled. While a pause applies to the
    /// task, it blocks here with its slot and memory released, and returns when the pause ends and the scheduler
    /// hands the slot back.
    pub fn checkpoint(&self) -> Result<(), Cancelled> {
        self.cancel.check()?;
        let inner = &self.inner;
        let mut st = inner.lock();
        loop {
            if self.cancel.is_cancelled() {
                return Err(Cancelled);
            }
            let Some(e) = st.tasks.get(&self.id) else { return Ok(()) };
            if e.bypassed {
                return Ok(());
            }
            let reason = st.pause_for(e.priority);
            let phase = e.phase;
            match (reason, phase) {
                (None, Phase::Running | Phase::Queued) => return Ok(()),
                (Some(r), _) => {
                    if phase != Phase::Paused || e.pause_reason != Some(r) {
                        if let Some(e) = st.tasks.get_mut(&self.id) {
                            e.phase = Phase::Paused;
                            e.pause_reason = Some(r);
                        }
                        inner.updated(&st, self.id);
                        inner.cv.notify_all();
                    }
                }
                (None, Phase::Paused) => {
                    if let Some(e) = st.tasks.get_mut(&self.id) {
                        e.phase = Phase::Resuming;
                        e.pause_reason = None;
                    }
                    inner.updated(&st, self.id);
                    inner.cv.notify_all();
                }
                (None, Phase::Resuming) => {}
            }
            st = inner.wait_timeout(st, inner.limits.tick);
        }
    }

    /// Reports progress (0..=1). Events go out at most once per progress interval per task, and always at 1.
    pub fn progress(&self, fraction: f64) {
        let fraction = fraction.clamp(0.0, 1.0);
        let inner = &self.inner;
        let mut st = inner.lock();
        let interval = inner.limits.progress_interval;
        let Some(e) = st.tasks.get_mut(&self.id) else { return };
        e.progress = Some(fraction);
        let due = fraction >= 1.0 || e.last_progress_event.is_none_or(|t| t.elapsed() >= interval);
        if due {
            e.last_progress_event = Some(Instant::now());
            inner.updated(&st, self.id);
        }
    }
}
