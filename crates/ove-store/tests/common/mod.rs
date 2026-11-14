#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use ove_core::ops::Op;
use ove_core::revision::Clock;
use serde_json::{Map, Value, json};

/// A clock that advances 1 ms per reading, so revisions are deterministic.
#[derive(Debug)]
pub struct StepClock(pub AtomicI64);
impl Clock for StepClock {
    fn now_ms(&self) -> i64 {
        self.0.fetch_add(1, Ordering::SeqCst)
    }
}
pub fn clock() -> Arc<dyn Clock> {
    Arc::new(StepClock(AtomicI64::new(1_700_000_000_000)))
}

pub fn set(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

pub fn setup_ops() -> Vec<Op> {
    vec![
        Op::AddTrack { track: json!({"id": "t1", "name": "Main", "order": 0, "type": "video"}) },
        Op::AddTrack { track: json!({"id": "t2", "name": "Music", "order": 1, "type": "audio"}) },
    ]
}

/// Deterministic xorshift PRNG.
pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// A deterministic sequence of valid batches for a document that starts with `setup_ops()` applied.
pub fn batches(seed: u64, n: usize) -> Vec<Vec<Op>> {
    let mut rng = Rng(seed.wrapping_mul(2_654_435_761).max(1));
    let mut live: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for b in 0..n {
        let mut ops = Vec::new();
        let k = 1 + rng.below(4);
        for j in 0..k {
            let choice = rng.below(10);
            if live.is_empty() || choice < 4 {
                let id = format!("i{b}_{j}");
                let start = rng.below(60_000) as f64 / 1000.0;
                let dur = 0.1 + rng.below(9_000) as f64 / 1000.0;
                let track = if rng.below(2) == 0 { "t1" } else { "t2" };
                let kind = if track == "t1" { "video" } else { "audio" };
                ops.push(Op::AddItem {
                    item: json!({"id": id, "trackId": track, "type": kind, "startTime": start, "endTime": start + dur, "name": format!("clip {b}"), "volume": rng.below(200)}),
                });
                live.push(id);
            } else if choice < 8 {
                let id = live[rng.below(live.len() as u64) as usize].clone();
                ops.push(Op::UpdateItem {
                    id,
                    set: set(
                        json!({"opacity": rng.below(100) as f64 / 100.0, "name": format!("n{}", rng.below(1000))}),
                    ),
                });
            } else {
                let at = rng.below(live.len() as u64) as usize;
                let id = live.swap_remove(at);
                ops.push(Op::RemoveItems { ids: vec![id] });
            }
        }
        out.push(ops);
    }
    out
}
