//! The QuickJS sandbox. Every frame runs in a fresh runtime with a memory limit, a stack limit and a CPU deadline
//! enforced by the interrupt handler. The context has only the ECMAScript built-ins and the prelude: no file
//! system, network, timers, module loader or host functions, and `Date`/`Math.random` are deterministic.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rquickjs::{Context, Ctx, Function, Module, Runtime, Value};

const PRELUDE: &str = include_str!("prelude.js");

/// Resource limits for one frame.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Heap limit in bytes.
    pub memory: usize,
    /// Wall-clock budget for evaluating the module and calling the composition.
    pub budget: Duration,
    /// Native stack limit in bytes (deep recursion).
    pub stack: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { memory: 64 << 20, budget: Duration::from_millis(250), stack: 1 << 20 }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RunError {
    #[error("the composition ran longer than its {0} ms budget (endless loop?)")]
    Budget(u64),
    #[error("the composition used more than {0} MB of memory")]
    Memory(usize),
    #[error("{0}")]
    Runtime(String),
    #[error("could not start the sandbox: {0}")]
    Setup(String),
}

impl RunError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Budget(_) => "budget",
            Self::Memory(_) => "memory",
            Self::Runtime(_) => "runtime",
            Self::Setup(_) => "setup",
        }
    }
}

/// What a frame produced: the resolved element tree (JSON from the prelude) and captured console output.
#[derive(Debug, Clone, Default)]
pub struct RunOutput {
    pub nodes: serde_json::Value,
    pub logs: Vec<String>,
}

/// Evaluates the transpiled module `js` and calls its default export with `input` (a JSON object).
pub fn run(js: &str, input: &serde_json::Value, limits: Limits) -> Result<RunOutput, RunError> {
    let rt = Runtime::new().map_err(|e| RunError::Setup(e.to_string()))?;
    rt.set_memory_limit(limits.memory);
    rt.set_max_stack_size(limits.stack);
    let interrupted = Arc::new(AtomicBool::new(false));
    let flag = interrupted.clone();
    let deadline = Instant::now() + limits.budget;
    rt.set_interrupt_handler(Some(Box::new(move || {
        let over = Instant::now() >= deadline;
        if over {
            flag.store(true, Ordering::Relaxed);
        }
        over
    })));
    let ctx = Context::full(&rt).map_err(|e| RunError::Setup(e.to_string()))?;
    let input = input.to_string();
    let result = ctx.with(|ctx| -> Result<String, RunError> {
        let fail = |ctx: &Ctx<'_>, e: rquickjs::Error| classify(ctx, e, &interrupted, limits);
        ctx.eval::<(), _>(PRELUDE).map_err(|e| RunError::Setup(format!("prelude: {}", describe(&ctx, e))))?;
        let tree: Function = ctx.globals().get("__ove_tree").map_err(|e| fail(&ctx, e))?;
        let module = Module::declare(ctx.clone(), "composition", js).map_err(|e| fail(&ctx, e))?;
        let (module, promise) = module.eval().map_err(|e| fail(&ctx, e))?;
        promise.finish::<()>().map_err(|e| fail(&ctx, e))?;
        let main: Value = module.get("default").map_err(|e| fail(&ctx, e))?;
        let Some(main) = main.as_function() else {
            return Err(RunError::Runtime("the default export must be a function".into()));
        };
        let arg = ctx.json_parse(input).map_err(|e| fail(&ctx, e))?;
        let out: Value = main.call((arg,)).map_err(|e| fail(&ctx, e))?;
        let json: String = tree.call((out,)).map_err(|e| fail(&ctx, e))?;
        Ok(json)
    });
    drop(ctx);
    let json = result?;
    let mut v: serde_json::Value = serde_json::from_str(&json).map_err(|e| RunError::Runtime(e.to_string()))?;
    let logs = v
        .get("logs")
        .and_then(|l| l.as_array())
        .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    Ok(RunOutput { nodes: v["nodes"].take(), logs })
}

fn classify(ctx: &Ctx<'_>, e: rquickjs::Error, interrupted: &AtomicBool, limits: Limits) -> RunError {
    if interrupted.load(Ordering::Relaxed) {
        return RunError::Budget(limits.budget.as_millis() as u64);
    }
    if matches!(e, rquickjs::Error::Allocation) {
        return RunError::Memory(limits.memory >> 20);
    }
    let msg = describe(ctx, e);
    if msg.contains("out of memory") {
        return RunError::Memory(limits.memory >> 20);
    }
    if msg.contains("interrupted") {
        return RunError::Budget(limits.budget.as_millis() as u64);
    }
    RunError::Runtime(msg)
}

fn describe(ctx: &Ctx<'_>, e: rquickjs::Error) -> String {
    if !matches!(e, rquickjs::Error::Exception) {
        return e.to_string();
    }
    let v = ctx.catch();
    if let Some(ex) = v.as_exception() {
        let msg = ex.message().unwrap_or_default();
        let name: Option<String> = ex.get("name").ok();
        match name {
            Some(n) if !n.is_empty() => format!("{n}: {msg}"),
            _ => msg,
        }
    } else if let Some(s) = v.as_string() {
        s.to_string().unwrap_or_default()
    } else {
        format!("uncaught {}", v.type_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transpile::transpile;

    fn exec(src: &str) -> Result<RunOutput, RunError> {
        let t = transpile(src);
        // Escape tests bypass the static checks to prove the runtime holds on its own.
        let js = if t.ok() { t.js } else { raw_js(src) };
        run(
            &js,
            &serde_json::json!({"time": 1.0, "width": 1920, "height": 1080, "assets": {}, "props": {}}),
            Limits { budget: Duration::from_millis(100), memory: 16 << 20, ..Limits::default() },
        )
    }

    fn raw_js(src: &str) -> String {
        src.to_string()
    }

    #[test]
    fn calls_the_default_export() {
        let out =
            exec("export default function C({ time }) { return <div style={{ opacity: time / 2 }}>t={time}</div>; }")
                .unwrap();
        assert_eq!(out.nodes[0]["tag"], "div");
        assert_eq!(out.nodes[0]["props"]["style"]["opacity"], 0.5);
        assert_eq!(out.nodes[0]["children"][0], "t=");
        assert_eq!(out.nodes[0]["children"][1], "1");
    }

    #[test]
    fn components_fragments_and_helpers() {
        let out = exec(
            "const Row = ({ n, children }) => <span>{n}:{children}</span>;
             export default function C({ time }) {
               const v = interpolate(time, [0, 2], [0, 100]);
               const s = spring({ time, stiffness: 100 });
               return <><Row n={1}>a</Row>{[1, 2].map((i) => <i key={i}>{i}</i>)}<b>{v}</b><u>{s > 0.5 ? 'up' : 'low'}</u></>;
             }",
        )
        .unwrap();
        let n = out.nodes.as_array().unwrap();
        assert_eq!(n.len(), 5);
        assert_eq!(n[0]["children"], serde_json::json!(["1", ":", "a"]));
        assert_eq!(n[3]["children"][0], "50");
        assert_eq!(n[4]["children"][0], "up");
    }

    #[test]
    fn date_and_random_are_deterministic() {
        let src = "export default function C() { return <p>{Date.now()}|{new Date().getTime()}|{Math.random()}|{random('a')}</p>; }";
        let a = exec(src).unwrap();
        let b = exec(src).unwrap();
        assert_eq!(a.nodes, b.nodes);
        assert_eq!(a.nodes[0]["children"][0], "0");
        assert_eq!(a.nodes[0]["children"][2], "0");
    }

    #[test]
    fn escapes_are_contained() {
        // No network, host or module APIs exist at runtime even when the static checks are skipped.
        for (src, needle) in [
            ("export default function C() { return fetch('http://example.com'); }", "fetch"),
            ("export default function C() { return require('fs'); }", "require"),
            ("export default function C() { return process.env; }", "process"),
            ("export default function C() { setTimeout(() => 0, 1); return null; }", "setTimeout"),
            ("export default function C() { return std.loadFile('/etc/passwd'); }", "std"),
            ("export default function C() { return os.exec(['ls']); }", "os"),
        ] {
            match exec(src) {
                Err(RunError::Runtime(m)) => assert!(m.contains(needle) && m.contains("not defined"), "{src}: {m}"),
                other => panic!("{src}: {other:?}"),
            }
        }
        match exec("import fs from 'fs'; export default function C() { return null; }") {
            Err(RunError::Runtime(m)) => assert!(m.contains("fs"), "{m}"),
            other => panic!("{other:?}"),
        }
        match exec("export default async function C() { await import('fs'); return null; }") {
            Err(RunError::Runtime(m)) => assert!(m.contains("render"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn endless_loops_hit_the_budget() {
        let t = Instant::now();
        assert_eq!(exec("export default function C() { for (;;) {} }").unwrap_err(), RunError::Budget(100));
        assert_eq!(exec("while (true) {} export default () => null;").unwrap_err(), RunError::Budget(100));
        assert!(t.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn huge_allocations_hit_the_memory_limit() {
        for src in [
            "export default function C() { const a = []; for (;;) a.push(new Array(1e5).fill(1)); }",
            "export default function C() { const a = []; for (let i = 0; ; i++) a.push(String(i).repeat(1e6)); }",
            "export default function C() { return new ArrayBuffer(1e9); }",
        ] {
            assert_eq!(exec(src).unwrap_err(), RunError::Memory(16), "{src}");
        }
        // Deep recursion is a stack overflow, reported as a runtime error rather than a crash.
        let e = exec("const f = (n) => f(n + 1) + 1; export default function C() { return f(0); }").unwrap_err();
        assert!(matches!(e, RunError::Runtime(_)), "{e:?}");
    }
}
