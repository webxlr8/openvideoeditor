//! Admission: which waiting task starts next, given what is running. Pure functions over a caller-supplied clock so
//! ordering, limits, the memory budget and ageing are tested on simulated time.

use std::time::Duration;

use crate::model::{Class, Priority, TaskId};

/// Concurrency limits per class, the background memory budget and timing constants.
#[derive(Debug, Clone, PartialEq)]
pub struct Limits {
    pub decode: usize,
    pub inference: usize,
    pub cpu: usize,
    pub io: usize,
    /// Bytes all running tasks together may estimate.
    pub mem_budget: u64,
    /// A waiting task's effective priority rises one level per this much waiting.
    pub age_step: Duration,
    /// Shortest gap between two progress events of one task.
    pub progress_interval: Duration,
    /// How often the dispatcher re-evaluates ageing when nothing else happens.
    pub tick: Duration,
}

/// The memory budget ceiling: 1.5 GiB.
pub const MEM_BUDGET_CAP: u64 = 1536 * 1024 * 1024;

impl Limits {
    /// Limits for this machine: decode 2, inference 1, cpu cores − 2 (at least 1), io 4, memory
    /// min(1.5 GiB, 20 % of physical memory).
    pub fn detect() -> Limits {
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
        Limits::for_machine(cores, crate::os::physical_memory())
    }

    /// Limits for a machine with `cores` logical cores and `physical` bytes of memory (`None`: unknown).
    pub fn for_machine(cores: usize, physical: Option<u64>) -> Limits {
        let mem_budget = physical.map(|p| (p / 5).min(MEM_BUDGET_CAP)).unwrap_or(MEM_BUDGET_CAP / 2);
        Limits {
            decode: 2,
            inference: 1,
            cpu: cores.saturating_sub(2).max(1),
            io: 4,
            mem_budget,
            age_step: Duration::from_secs(5 * 60),
            progress_interval: Duration::from_millis(250),
            tick: Duration::from_millis(500),
        }
    }

    pub fn class_limit(&self, class: Class) -> usize {
        match class {
            Class::Decode => self.decode,
            Class::Inference => self.inference,
            Class::Cpu => self.cpu,
            Class::Io => self.io,
        }
    }
}

/// A task that may start: a new one, or a paused one ready to resume.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub id: TaskId,
    pub priority: Priority,
    pub class: Class,
    pub est_mem: u64,
    /// When it was submitted, on the scheduler's clock.
    pub queued_at: Duration,
    /// Submission order: ties go to the older task.
    pub seq: u64,
}

/// What running tasks hold.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    pub per_class: [usize; 4],
    pub mem: u64,
    /// Number of running tasks.
    pub count: usize,
    /// A task larger than the whole budget is running: nothing else may start.
    pub alone: bool,
}

impl Usage {
    pub fn hold(&mut self, class: Class, est_mem: u64, alone: bool) {
        self.per_class[class.index()] += 1;
        self.mem = self.mem.saturating_add(est_mem);
        self.count += 1;
        self.alone |= alone;
    }
}

/// The priority level a task competes at after waiting `waited`: one level better per `age_step`. Ageing never lifts
/// a task above interactive, so export always goes first.
pub fn effective_level(priority: Priority, waited: Duration, age_step: Duration) -> u8 {
    let base = priority.level();
    if base <= Priority::Interactive.level() || age_step.is_zero() {
        return base;
    }
    let steps = (waited.as_nanos() / age_step.as_nanos()).min(u8::MAX as u128) as u8;
    base.saturating_sub(steps).max(Priority::Interactive.level())
}

/// The decision for the next task to start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Admit {
    /// Index into the candidate slice.
    pub index: usize,
    /// It is larger than the budget and runs with nothing beside it.
    pub alone: bool,
}

/// Picks the next candidate to start, or `None` when nothing may start now.
///
/// Candidates are taken in effective-priority order, then submission order. Paused priorities and full classes are
/// skipped. A task that does not fit in the remaining memory waits while smaller ones that fit go ahead, except once
/// it has waited a full age step: then it holds the line until memory frees. A task larger than the whole budget
/// starts only when nothing runs, and while it waits for that, nothing else starts.
pub fn next(
    candidates: &[Candidate],
    usage: &Usage,
    now: Duration,
    limits: &Limits,
    paused: impl Fn(Priority) -> bool,
) -> Option<Admit> {
    if usage.alone {
        return None;
    }
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by_key(|&i| {
        let c = &candidates[i];
        (effective_level(c.priority, now.saturating_sub(c.queued_at), limits.age_step), c.seq)
    });
    for i in order {
        let c = &candidates[i];
        if paused(c.priority) {
            continue;
        }
        if usage.per_class[c.class.index()] >= limits.class_limit(c.class) {
            continue;
        }
        if c.est_mem > limits.mem_budget {
            return if usage.count == 0 { Some(Admit { index: i, alone: true }) } else { None };
        }
        if usage.mem.saturating_add(c.est_mem) > limits.mem_budget {
            if now.saturating_sub(c.queued_at) >= limits.age_step {
                return None;
            }
            continue;
        }
        return Some(Admit { index: i, alone: false });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1024 * 1024;

    fn limits() -> Limits {
        Limits { mem_budget: 1000 * MB, ..Limits::for_machine(8, Some(16 * 1024 * MB)) }
    }

    fn cand(id: TaskId, priority: Priority, class: Class, est_mem: u64, queued_at: u64) -> Candidate {
        Candidate { id, priority, class, est_mem, queued_at: Duration::from_secs(queued_at), seq: id }
    }

    /// Starts candidates until none may start; returns the started ids in order.
    fn drain(cands: &mut Vec<Candidate>, usage: &mut Usage, now: u64, l: &Limits) -> Vec<TaskId> {
        let mut started = vec![];
        while let Some(a) = next(cands, usage, Duration::from_secs(now), l, |_| false) {
            let c = cands.remove(a.index);
            usage.hold(c.class, c.est_mem, a.alone);
            started.push(c.id);
        }
        started
    }

    #[test]
    fn machine_limits() {
        let l = Limits::for_machine(8, Some(16 * 1024 * MB));
        assert_eq!((l.decode, l.inference, l.cpu, l.io), (2, 1, 6, 4));
        assert_eq!(l.mem_budget, MEM_BUDGET_CAP, "20 % of 16 GiB is above the 1.5 GiB cap");
        let small = Limits::for_machine(2, Some(4 * 1024 * MB));
        assert_eq!(small.cpu, 1, "at least one cpu slot");
        assert_eq!(small.mem_budget, 4 * 1024 * MB / 5);
    }

    #[test]
    fn priority_then_submission_order() {
        let l = limits();
        let mut c = vec![
            cand(1, Priority::Analysis, Class::Cpu, 0, 0),
            cand(2, Priority::Derivative, Class::Cpu, 0, 1),
            cand(3, Priority::Export, Class::Cpu, 0, 2),
            cand(4, Priority::Interactive, Class::Cpu, 0, 3),
            cand(5, Priority::Derivative, Class::Cpu, 0, 4),
        ];
        let mut u = Usage::default();
        assert_eq!(drain(&mut c, &mut u, 5, &l), vec![3, 4, 2, 5, 1]);
    }

    #[test]
    fn class_limits_hold_and_other_classes_pass() {
        let l = limits();
        let mut c: Vec<Candidate> = (1..=4).map(|i| cand(i, Priority::Derivative, Class::Decode, 0, i)).collect();
        c.push(cand(9, Priority::Analysis, Class::Io, 0, 9));
        let mut u = Usage::default();
        let started = drain(&mut c, &mut u, 10, &l);
        assert_eq!(started, vec![1, 2, 9], "two decode sessions, then the io task passes the full decode class");
        assert_eq!(u.per_class[Class::Decode.index()], 2);
    }

    #[test]
    fn budget_admits_only_what_fits() {
        let l = limits();
        let mut c = vec![
            cand(1, Priority::Derivative, Class::Cpu, 600 * MB, 0),
            cand(2, Priority::Derivative, Class::Cpu, 600 * MB, 1),
            cand(3, Priority::Derivative, Class::Cpu, 300 * MB, 2),
        ];
        let mut u = Usage::default();
        assert_eq!(drain(&mut c, &mut u, 3, &l), vec![1, 3], "the 600 MB task waits; the 300 MB one fits");
        assert_eq!(u.mem, 900 * MB);
        // Memory released: the waiting task starts.
        let mut u2 = Usage::default();
        u2.hold(Class::Cpu, 300 * MB, false);
        assert_eq!(drain(&mut c, &mut u2, 4, &l), vec![2]);
    }

    #[test]
    fn long_waiting_big_task_holds_the_line() {
        let l = limits();
        let mut u = Usage::default();
        u.hold(Class::Cpu, 600 * MB, false);
        let mut c = vec![
            cand(1, Priority::Derivative, Class::Cpu, 600 * MB, 0),
            cand(2, Priority::Derivative, Class::Cpu, 10, 1),
        ];
        let late = l.age_step.as_secs() + 1;
        assert!(
            drain(&mut c, &mut u, late, &l).is_empty(),
            "small tasks no longer pass a task that waited an age step"
        );
    }

    #[test]
    fn task_larger_than_budget_runs_alone() {
        let l = limits();
        let mut c = vec![
            cand(1, Priority::Derivative, Class::Cpu, 2000 * MB, 0),
            cand(2, Priority::Derivative, Class::Io, 10 * MB, 1),
        ];
        let mut busy = Usage::default();
        busy.hold(Class::Io, 10 * MB, false);
        assert!(drain(&mut c, &mut busy, 2, &l).is_empty(), "waits for the machine to empty; nothing passes it");
        let mut idle = Usage::default();
        assert_eq!(drain(&mut c, &mut idle, 3, &l), vec![1], "starts alone, and nothing starts beside it");
        assert!(idle.alone);
    }

    #[test]
    fn paused_priorities_are_skipped() {
        let l = limits();
        let c = vec![cand(1, Priority::Analysis, Class::Cpu, 0, 0), cand(2, Priority::Interactive, Class::Cpu, 0, 1)];
        let a = next(&c, &Usage::default(), Duration::ZERO, &l, |p| p == Priority::Interactive).unwrap();
        assert_eq!(c[a.index].id, 1);
        assert!(next(&c, &Usage::default(), Duration::ZERO, &l, |_| true).is_none());
    }

    #[test]
    fn ageing_lifts_one_level_per_step_and_stops_at_interactive() {
        let step = Duration::from_secs(300);
        assert_eq!(effective_level(Priority::Analysis, Duration::from_secs(299), step), 3);
        assert_eq!(effective_level(Priority::Analysis, Duration::from_secs(300), step), 2);
        assert_eq!(effective_level(Priority::Analysis, Duration::from_secs(600), step), 1);
        assert_eq!(effective_level(Priority::Analysis, Duration::from_secs(9000), step), 1);
        assert_eq!(effective_level(Priority::Export, Duration::from_secs(9000), step), 0);
    }

    /// Interactive requests arrive every second for 10 minutes and each holds the only cpu slot for 2 seconds; one
    /// analysis task waits from the start. Simulated time, one-second steps.
    #[test]
    fn no_starvation_under_continuous_interactive_load() {
        let l = Limits { cpu: 1, ..limits() };
        let mut waiting = vec![cand(1, Priority::Analysis, Class::Cpu, 0, 0)];
        let mut running: Vec<(TaskId, u64)> = vec![]; // (id, ends at)
        let mut analysis_started_at = None;
        for (next_id, now) in (2u64..).zip(0..=600u64) {
            running.retain(|&(_, end)| end > now);
            waiting.push(Candidate { seq: next_id, ..cand(next_id, Priority::Interactive, Class::Cpu, 0, now) });
            let mut usage = Usage::default();
            for _ in &running {
                usage.hold(Class::Cpu, 0, false);
            }
            while let Some(a) = next(&waiting, &usage, Duration::from_secs(now), &l, |_| false) {
                let c = waiting.remove(a.index);
                usage.hold(c.class, c.est_mem, a.alone);
                if c.id == 1 {
                    analysis_started_at = Some(now);
                }
                running.push((c.id, now + 2));
            }
        }
        let at = analysis_started_at.expect("the analysis task started within 10 minutes");
        assert!(at <= 600 && at >= 2 * l.age_step.as_secs(), "started at {at} s");
    }
}
