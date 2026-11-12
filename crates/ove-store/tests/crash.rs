//! Crash harness (task 4.3): a child process applies batches and is killed at random points — inside write, flush,
//! rename and directory sync via failpoints, or by an external kill at a random moment. Every reopen must yield a
//! valid document equal to the state before or after the last acknowledged batch.
//!
//! Kills: `OVE_CRASH_RUNS` (default 1000); runs continue until that many children were actually killed.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

mod common;

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use common::{Rng, batches, clock, setup_ops};
use ove_core::model::Document;
use ove_core::ops::apply_batch;
use ove_core::revision::Source;
use ove_store::Library;
use ove_store::project::{Mode, Project};

const BATCHES: usize = 25;

/// Child: apply the seeded batches, acknowledging each one only after `apply` returned.
#[test]
#[ignore = "child process helper"]
fn crash_child() {
    let (Ok(bundle), Ok(seed)) = (std::env::var("OVE_CRASH_BUNDLE"), std::env::var("OVE_CRASH_SEED")) else { return };
    let seed: u64 = seed.parse().unwrap();
    let mut p = Project::open(Path::new(&bundle), Mode::Write, clock()).unwrap();
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    for (i, b) in batches(seed, BATCHES).into_iter().enumerate() {
        p.apply(b, Source::Agent, None).unwrap();
        writeln!(out, "ACK {i}").unwrap();
        out.flush().unwrap();
    }
    writeln!(out, "DONE").unwrap();
}

fn expected_states(setup: &Document, seed: u64) -> Vec<Document> {
    let mut states = vec![setup.clone()];
    let mut d = setup.clone();
    for b in batches(seed, BATCHES) {
        d = apply_batch(&d, &b).unwrap().doc;
        states.push(d.clone());
    }
    states
}

#[test]
fn kill_at_random_points_never_corrupts() {
    let runs: u64 = std::env::var("OVE_CRASH_RUNS").ok().and_then(|s| s.parse().ok()).unwrap_or(1000);
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path(), clock()).unwrap();
    let exe = std::env::current_exe().unwrap();
    let points = [
        "record.write",
        "record.flush",
        "record.rename",
        "record.dirsync",
        "snapshot.write",
        "snapshot.flush",
        "snapshot.rename",
        "index.write",
        "index.rename",
    ];
    let mut rng = Rng(0x5eed_cafe);
    let (mut killed_by_failpoint, mut killed_externally, mut completed) = (0, 0, 0);
    let mut acked_hist = [0usize; BATCHES + 2];

    let mut run = 0u64;
    while killed_by_failpoint + killed_externally < runs {
        run += 1;
        let mut p = lib.create(&format!("crash {run}")).unwrap();
        p.apply(setup_ops(), Source::Gui, None).unwrap();
        let setup = p.doc().clone();
        let bundle = p.bundle().to_path_buf();
        p.close().unwrap();

        let seed = run + 1;
        let mut cmd = Command::new(&exe);
        cmd.args(["--exact", "crash_child", "--ignored", "--nocapture", "--test-threads", "1"])
            .env("OVE_CRASH_BUNDLE", &bundle)
            .env("OVE_CRASH_SEED", seed.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let external = rng.below(3) == 0;
        if !external {
            let point = points[rng.below(points.len() as u64) as usize];
            let hit = if point.starts_with("record") { 1 + rng.below(2 * BATCHES as u64) } else { 1 };
            cmd.env("OVE_FAILPOINTS", format!("{point}=abort@{hit}"));
        }
        let mut child = cmd.spawn().unwrap();
        let stdout = child.stdout.take().unwrap();
        let reader = std::thread::spawn(move || {
            let mut last: Option<usize> = None;
            let mut done = false;
            let mut all = Vec::new();
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                all.push(line.clone());
                // libtest prints "test crash_child ... " without a newline before the first ACK.
                if let Some(at) = line.find("ACK ") {
                    last = Some(line[at + 4..].trim().parse().unwrap());
                } else if line.trim_end().ends_with("DONE") {
                    done = true;
                }
            }
            (last, done, all)
        });
        if external {
            std::thread::sleep(Duration::from_micros(2_000 + rng.below(60_000)));
            let _ = child.kill();
        }
        let _ = child.wait();
        let (last, done, all_lines) = reader.join().unwrap();
        if done {
            completed += 1;
        } else if external {
            killed_externally += 1;
        } else {
            killed_by_failpoint += 1;
        }

        // `acked` batches are durable for sure; the next one may or may not be.
        let acked = last.map(|n| n + 1).unwrap_or(0);
        acked_hist[acked.min(BATCHES + 1)] += 1;
        let states = expected_states(&setup, seed);
        let reopened =
            Project::open(&bundle, Mode::Write, clock()).unwrap_or_else(|e| panic!("run {run}: reopen failed: {e}"));
        let got = reopened.doc();
        let ok_before = got.content_eq(&states[acked]);
        let ok_after = states.get(acked + 1).is_some_and(|s| got.content_eq(s));
        let matches: Vec<usize> =
            states.iter().enumerate().filter(|(_, s)| got.content_eq(s)).map(|(i, _)| i).collect();
        assert!(
            ok_before || ok_after,
            "external={external} failpoints={:?} matches states {matches:?}; lines {all_lines:?}",
            cmd.get_envs()
                .find(|(k, _)| *k == "OVE_FAILPOINTS")
                .map(|(_, v)| v.map(|v| v.to_string_lossy().into_owned())),
        );
        assert!(
            ok_before || ok_after,
            "run {run} (seed {seed}): reopened state matches neither batch {acked} nor {} (items: {})",
            acked + 1,
            got.project.items.len()
        );
        drop(reopened);
        std::fs::remove_dir_all(&bundle).unwrap();
    }
    eprintln!(
        "crash harness: {run} runs — {killed_by_failpoint} killed at a failpoint, {killed_externally} killed externally, {completed} completed; acked-batch histogram {acked_hist:?}"
    );
    // Sanity: both kill styles happen (the mix varies with machine load).
    assert!(killed_by_failpoint > runs / 10 && killed_externally > runs / 10);
}
