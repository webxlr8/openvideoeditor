//! Stale text-edit benchmark (unify-agent-tool-layer task 2.2): a 5,000-item project, an agent prints it at rev R,
//! the person makes 10 edits, then the agent applies one text edit against R. Prints JSON with the apply times,
//! with the printed revision cached (the usual case: the agent printed it in this session) and without (a new
//! session that kept the old rev).
//!
//! ```text
//! cargo run -p ove-bench --release --bin stale-apply [-- --items N --behind K --runs N]
//! ```

use std::sync::Arc;
use std::time::Instant;

use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_store::project::{Mode, Project};
use ove_tools::{BundleCtx, Surface};
use serde_json::{Map, Value, json};

fn arg(name: &str, default: usize) -> usize {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (items, behind, runs) = (arg("--items", 5000), arg("--behind", 10), arg("--runs", 5));
    let clock: Arc<dyn ove_core::Clock> = Arc::new(ove_core::SystemClock);
    let mut cold = Vec::new();
    let mut warm = Vec::new();
    for run in 0..runs * 2 {
        let d = tempfile::tempdir()?;
        let lib = d.path().join("lib");
        let library = ove_store::Library::open(&lib, clock.clone())?;
        let mut p = library.create("Bench")?;
        let mut ops: Vec<Op> = Vec::new();
        for (n, t) in ["t_a", "t_b", "t_c", "t_d", "t_e"].iter().enumerate() {
            ops.push(serde_json::from_value(
                json!({"op": "addTrack", "track": {"id": t, "name": t, "order": n, "type": "video"}}),
            )?);
        }
        for i in 0..items {
            let track = ["t_a", "t_b", "t_c", "t_d", "t_e"][i % 5];
            ops.push(serde_json::from_value(
                json!({"op": "addItem", "item": {"id": format!("i{i}"), "trackId": track,
                "type": "text", "startTime": (i / 5) as f64, "endTime": (i / 5) as f64 + 0.9,
                "textContent": format!("Title {i}"), "name": format!("Item {i}")}}),
            )?);
        }
        p.apply(ops, Source::Gui, None)?;
        let bundle = p.bundle().to_path_buf();
        p.close()?;

        let mut ctx = BundleCtx::open(Some(&lib), Surface::Mcp, Source::Mcp)?;
        let printed = ove_tools::call(&mut ctx, "print_timeline", &obj(json!({"project": "Bench"})))?;
        let text = printed.json["text"].as_str().unwrap_or_default().to_string();
        let rev = printed.json["rev"].as_i64().unwrap_or_default();
        // The person's edits in between, each its own revision.
        let mut p = Project::open(&bundle, Mode::Write, clock.clone())?;
        for k in 0..behind {
            let set = obj(json!({"name": format!("Person {k}")}));
            p.apply(vec![Op::UpdateItem { id: format!("i{}", items - 1 - k), set }], Source::Gui, None)?;
        }
        p.close()?;
        let cached = run % 2 == 0;
        if !cached {
            // A new session: nothing printed in this process yet.
            ctx = BundleCtx::open(Some(&lib), Surface::Mcp, Source::Mcp)?;
            forget(&bundle, rev);
        }
        let line = ove_tools::edits::elements(&text).remove("i7").unwrap_or_default();
        let edit = json!({"project": "Bench", "rev": rev,
            "edits": [{"oldText": line, "newText": line.replace("Title 7", "Agent title")}]});
        let held = std::env::var_os("OVE_BENCH_HELD").is_some();
        let (r, ms) = if held {
            // The project held open, as the app holds it (no open and close per call).
            let mut p = Project::open(&bundle, Mode::Write, clock.clone())?;
            let mut host = ove_tools::ProjectHost { project: &mut p };
            let mut actx =
                ove_tools::AppCtx { host: &mut host, source: Source::Mcp, surface: Surface::Mcp, actor: None };
            let mut e = obj(edit);
            e.remove("project");
            let t = Instant::now();
            let r = ove_tools::call(&mut actx, "apply_timeline", &e)?;
            (r, t.elapsed().as_secs_f64() * 1000.0)
        } else {
            let t = Instant::now();
            let r = ove_tools::call(&mut ctx, "apply_timeline", &obj(edit))?;
            (r, t.elapsed().as_secs_f64() * 1000.0)
        };
        if r.json["changed"] != true {
            return Err(format!("the edit did not apply: {}", r.json).into());
        }
        if cached { warm.push(ms) } else { cold.push(ms) }
    }
    let median = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    println!(
        "{}",
        json!({"items": items, "behind": behind, "runs": runs,
            "applyMsCachedMedian": median(&mut warm), "applyMsUncachedMedian": median(&mut cold),
            "applyMsCached": warm, "applyMsUncached": cold})
    );
    Ok(())
}

fn obj(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

/// Drops the cached print of `rev` by filling the cache with other revisions of the project.
fn forget(bundle: &std::path::Path, rev: i64) {
    for k in 1..=ove_tools::edits::KEEP_PRINTS as i64 {
        ove_tools::edits::remember(bundle, rev + 1_000_000 + k, "");
    }
}
