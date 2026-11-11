//! The bundle's projection follows every revision (timeline-projection task 2.3) and a concurrency soak where GUI
//! batches and projection round trips race against one document (task 7.1).

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{clock, set, setup_ops};
use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_projection::{ApplyInput, Prefer};
use ove_store::Library;
use ove_store::project::{Mode, Project};
use serde_json::json;

fn header_rev(p: &Project) -> i64 {
    p.flush_projection();
    let text = std::fs::read_to_string(p.bundle().join("timeline.tsx")).unwrap();
    ove_projection::parse::header_rev(&text).unwrap().unwrap()
}

#[test]
fn projection_header_follows_every_revision() {
    let d = tempfile::tempdir().unwrap();
    let lib = Library::open(d.path(), clock()).unwrap();
    let mut p = lib.create("Follow").unwrap();
    assert_eq!(header_rev(&p), p.rev());
    assert!(p.bundle().join("timeline.d.ts").exists());
    for (i, src) in [Source::Gui, Source::Cli, Source::Mcp, Source::Agent].into_iter().enumerate() {
        let ops = if i == 0 {
            setup_ops()
        } else {
            vec![Op::UpdateTrack { id: "t1".into(), set: set(json!({"name": format!("Main {i}")})) }]
        };
        p.apply(ops, src, None).unwrap();
        assert_eq!(header_rev(&p), p.rev(), "after {src:?}");
    }
    p.undo().unwrap();
    assert_eq!(header_rev(&p), p.rev(), "after undo");
    // Opened by another session after a crash (no close): the projection is brought up to date on open.
    let bundle = p.bundle().to_path_buf();
    drop(p);
    std::fs::write(bundle.join("timeline.tsx"), "stale").unwrap();
    let p = Project::open(&bundle, Mode::Write, clock()).unwrap();
    assert_eq!(header_rev(&p), p.rev());
}

/// One writer thread applies GUI-style batches (renaming items), another prints the projection, edits it (sets a
/// field the GUI never touches) and applies it against its printed base. An oracle records the last value each side
/// wrote per item; at the end every item must hold both sides' last values — zero lost edits.
#[test]
fn concurrency_soak_loses_no_edits() {
    let secs: u64 = std::env::var("OVE_SOAK_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(10);
    let d = tempfile::tempdir().unwrap();
    let lib = Library::open(d.path(), clock()).unwrap();
    let mut p = lib.create("Soak").unwrap();
    p.apply(setup_ops(), Source::Gui, None).unwrap();
    let mut ops = Vec::new();
    for i in 0..40 {
        ops.push(Op::AddItem { item: json!({"id": format!("s{i}"), "trackId": "t1", "type": "video", "startTime": i, "endTime": i + 1, "name": "init"}) });
    }
    p.apply(ops, Source::Gui, None).unwrap();
    let project = Arc::new(Mutex::new(p));
    let deadline = Instant::now() + Duration::from_secs(secs);
    let gui_oracle = Arc::new(Mutex::new(std::collections::BTreeMap::<String, String>::new()));
    let file_oracle = Arc::new(Mutex::new(std::collections::BTreeMap::<String, i64>::new()));

    let gui = {
        let (project, oracle) = (project.clone(), gui_oracle.clone());
        std::thread::spawn(move || {
            let mut n = 0u64;
            while Instant::now() < deadline {
                n += 1;
                let id = format!("s{}", (n * 7) % 40);
                let name = format!("gui-{n}");
                let mut p = project.lock().unwrap();
                p.apply(vec![Op::UpdateItem { id: id.clone(), set: set(json!({"name": name})) }], Source::Gui, None)
                    .unwrap();
                oracle.lock().unwrap().insert(id, name);
                drop(p);
                std::thread::sleep(Duration::from_micros(300));
            }
            n
        })
    };
    let agent = {
        let (project, oracle) = (project.clone(), file_oracle.clone());
        std::thread::spawn(move || {
            let mut n = 0i64;
            let (mut conflicts, mut applied) = (0usize, 0usize);
            while Instant::now() < deadline {
                n += 1;
                // Print at the current revision, then let the GUI move on before applying.
                let text = project.lock().unwrap().projection();
                std::thread::sleep(Duration::from_millis(2));
                let id = format!("s{}", (n * 13) % 40);
                let needle = format!("id=\"{id}\"");
                let edited: String = text
                    .lines()
                    .map(|l| {
                        if !l.contains(&needle) {
                            return format!("{l}\n");
                        }
                        // Drop a previous zAgent value, then set the new one.
                        let l = match l.find(" zAgent={") {
                            Some(at) => {
                                let end = at + l[at..].find('}').unwrap() + 1;
                                format!("{}{}", &l[..at], &l[end..])
                            }
                            None => l.to_string(),
                        };
                        format!("{}\n", l.replacen(&needle, &format!("{needle} zAgent={{{n}}}"), 1))
                    })
                    .collect();
                let mut p = project.lock().unwrap();
                let current = p.doc().clone();
                let plan = {
                    let pr: &Project = &p;
                    ove_projection::plan(&ApplyInput {
                        text: &edited,
                        current: &current,
                        base_for: &|rev| {
                            pr.doc_at(rev).map_err(|e| ove_projection::BaseError {
                                code: e.code().into(),
                                message: e.to_string(),
                            })
                        },
                        bundle: Some(pr.bundle()),
                        prefer: Prefer::File,
                        now_ms: 1,
                    })
                    .expect("plan")
                };
                conflicts += plan.conflicts.len();
                if !plan.ops.is_empty() {
                    p.apply(plan.ops, Source::Agent, None).unwrap();
                    applied += 1;
                }
                oracle.lock().unwrap().insert(id, n);
            }
            (n, applied, conflicts)
        })
    };
    let gui_batches = gui.join().unwrap();
    let (agent_rounds, agent_applied, conflicts) = agent.join().unwrap();
    let p = project.lock().unwrap();
    let errs = ove_core::invariants::errors(ove_core::invariants::check_document(p.doc()));
    assert!(errs.is_empty(), "{errs:?}");
    for (id, name) in gui_oracle.lock().unwrap().iter() {
        assert_eq!(p.doc().item(id).unwrap().fields["name"], json!(name), "GUI edit lost on {id}");
    }
    for (id, n) in file_oracle.lock().unwrap().iter() {
        assert_eq!(p.doc().item(id).unwrap().fields["zAgent"], json!(n), "file edit lost on {id}");
    }
    assert_eq!(conflicts, 0, "disjoint fields must never conflict");
    // Reopen equals memory.
    let mem = p.doc().clone();
    let bundle = p.bundle().to_path_buf();
    drop(p);
    let reopened = Project::open(&bundle, Mode::ReadOnly, clock()).unwrap();
    assert_eq!(reopened.doc(), &mem);
    eprintln!(
        "SOAK {secs}s: {gui_batches} GUI batches, {agent_rounds} projection round trips ({agent_applied} applied), 0 lost edits, 0 invariant failures"
    );
}
