//! TSX → JavaScript for the sandbox: TypeScript is stripped and JSX becomes `h(type, props, ...children)` calls.
//! Static checks run on the parsed source: imports, missing default export, and globals the sandbox does not have.

use std::path::Path;

use oxc_allocator::Allocator;
use oxc_ast::ast::{ExportDefaultDeclarationKind, Statement};
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::{GetSpan, SourceType};
use oxc_transformer::{JsxOptions, JsxRuntime, TransformOptions, Transformer};

use crate::diag::{Diagnostic, line_col};

/// Globals a composition may use: the JavaScript built-ins QuickJS provides plus the helpers from the prelude.
pub const ALLOWED_GLOBALS: &[&str] = &[
    // language built-ins
    "Array",
    "ArrayBuffer",
    "BigInt",
    "Boolean",
    "DataView",
    "Date",
    "Error",
    "Float32Array",
    "Float64Array",
    "Infinity",
    "Int16Array",
    "Int32Array",
    "Int8Array",
    "Intl",
    "JSON",
    "Map",
    "Math",
    "NaN",
    "Number",
    "Object",
    "Promise",
    "Proxy",
    "RangeError",
    "Reflect",
    "RegExp",
    "Set",
    "String",
    "Symbol",
    "SyntaxError",
    "TypeError",
    "Uint16Array",
    "Uint32Array",
    "Uint8Array",
    "Uint8ClampedArray",
    "WeakMap",
    "WeakSet",
    "console",
    "decodeURIComponent",
    "encodeURIComponent",
    "globalThis",
    "isFinite",
    "isNaN",
    "parseFloat",
    "parseInt",
    "undefined",
    // prelude helpers (see `SUPPORTED.md`)
    "h",
    "Fragment",
    "React",
    "Fill",
    "interpolate",
    "spring",
    "clamp",
    "mix",
    "easing",
    "stagger",
    "random",
];

/// Globals that exist in browsers or Node but deliberately not in the sandbox, with the reason shown to the user.
const FORBIDDEN_GLOBALS: &[(&str, &str)] = &[
    ("fetch", "no network access: put images in `assets`"),
    ("XMLHttpRequest", "no network access: put images in `assets`"),
    ("WebSocket", "no network access"),
    ("require", "modules cannot be loaded: write everything in this one file"),
    ("process", "there is no host process"),
    ("setTimeout", "no timers: derive everything from `time`"),
    ("setInterval", "no timers: derive everything from `time`"),
    ("requestAnimationFrame", "no timers: derive everything from `time`"),
    ("window", "there is no browser window: use `width`/`height`"),
    ("document", "there is no DOM: return JSX"),
    ("localStorage", "no storage: a frame must depend only on its inputs"),
    ("useState", "no hooks: a composition is a pure function of `time`"),
    ("useEffect", "no hooks: a composition is a pure function of `time`"),
    ("useRef", "no hooks: a composition is a pure function of `time`"),
    ("useMemo", "no hooks: a composition is a pure function of `time`"),
];

/// Transpiled JavaScript (an ES module whose default export is the composition function) and diagnostics.
#[derive(Debug, Clone)]
pub struct Transpiled {
    pub js: String,
    pub diagnostics: Vec<Diagnostic>,
}

impl Transpiled {
    pub fn ok(&self) -> bool {
        !self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

pub fn transpile(src: &str) -> Transpiled {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, src, SourceType::tsx()).parse();
    let mut diags: Vec<Diagnostic> = Vec::new();
    let pos = |off: u32| Some(line_col(src, off as usize));
    for e in ret.diagnostics.errors() {
        let off = e.labels.first().map(|l| l.offset() as u32);
        diags.push(Diagnostic::error("syntax", e.message.to_string()).at(off.and_then(pos)));
    }
    if !diags.is_empty() {
        return Transpiled { js: String::new(), diagnostics: diags };
    }
    let mut program = ret.program;

    let mut has_default = false;
    for stmt in &program.body {
        match stmt {
            Statement::ImportDeclaration(d) => diags.push(
                Diagnostic::error(
                    "import",
                    format!("imports are not supported (`{}`): write everything in this one file", d.source.value.as_str().unwrap_or_default()),
                )
                .at(pos(d.span.start)),
            ),
            Statement::ExportAllDeclaration(d) => {
                diags.push(Diagnostic::error("import", "re-exports are not supported").at(pos(d.span.start)))
            }
            Statement::ExportFromDeclaration(d) => {
                diags.push(Diagnostic::error("import", "re-exports are not supported").at(pos(d.span.start)))
            }
            Statement::ExportDefaultDeclaration(d) => {
                has_default = true;
                if let ExportDefaultDeclarationKind::FunctionDeclaration(f) = &d.declaration
                    && (f.r#async || f.generator)
                {
                    diags.push(
                        Diagnostic::error("async", "the composition must be a plain synchronous function")
                            .at(pos(d.span.start)),
                    );
                }
            }
            _ => {}
        }
    }
    if !has_default {
        diags.push(Diagnostic::error(
            "export",
            "missing `export default function Composition({ time, width, height, assets, props })`",
        ));
    }

    let sem = SemanticBuilder::new().with_build_nodes(true).with_check_syntax_error(true).build(&program);
    for e in sem.diagnostics.errors() {
        let off = e.labels.first().map(|l| l.offset() as u32);
        diags.push(Diagnostic::error("syntax", e.message.to_string()).at(off.and_then(pos)));
    }
    let semantic = sem.semantic;
    {
        let scoping = semantic.scoping();
        let mut unresolved: Vec<(String, u32)> = Vec::new();
        for (name, ids) in scoping.root_unresolved_references().iter() {
            // Type-only references (`props: Input<Props>`) are erased by the transform.
            let Some(first) = ids
                .iter()
                .map(|id| scoping.get_reference(*id))
                .filter(|r| r.is_value())
                .map(|r| semantic.nodes().get_node(r.node_id()).kind().span().start)
                .min()
            else {
                continue;
            };
            unresolved.push((name.as_str().to_string(), first));
        }
        unresolved.sort_by_key(|(_, o)| *o);
        for (name, off) in unresolved {
            if ALLOWED_GLOBALS.contains(&name.as_str()) {
                continue;
            }
            let d = match FORBIDDEN_GLOBALS.iter().find(|(n, _)| *n == name) {
                Some((_, why)) => Diagnostic::error("global", format!("`{name}` is not available: {why}")),
                None => Diagnostic::error("global", format!("`{name}` is not defined")),
            };
            diags.push(d.at(pos(off)));
        }
    }
    // `import(...)` expressions show up as syntax nodes, not references.
    for node in semantic.nodes().iter() {
        if let oxc_ast::AstKind::ImportExpression(e) = node.kind() {
            diags.push(Diagnostic::error("import", "dynamic `import()` is not supported").at(pos(e.span.start)));
        }
    }
    if diags.iter().any(Diagnostic::is_error) {
        return Transpiled { js: String::new(), diagnostics: diags };
    }

    let scoping = semantic.into_scoping();
    let options = TransformOptions {
        jsx: JsxOptions {
            runtime: JsxRuntime::Classic,
            pragma: Some("h".into()),
            pragma_frag: Some("Fragment".into()),
            ..JsxOptions::default()
        },
        ..TransformOptions::default()
    };
    let out = Transformer::new(&allocator, Path::new("composition.tsx"), &options).build_with_scoping(scoping, &mut program);
    for e in out.diagnostics.errors() {
        let off = e.labels.first().map(|l| l.offset() as u32);
        diags.push(Diagnostic::error("syntax", e.message.to_string()).at(off.and_then(pos)));
    }
    let js = Codegen::new().build(&program).code;
    Transpiled { js, diagnostics: diags }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_types_and_lowers_jsx() {
        let t = transpile(
            "type P = { time: number };\nexport default function C({ time }: P) { return <div style={{ opacity: time }}>hi {time as number}</div>; }",
        );
        assert!(t.ok(), "{:?}", t.diagnostics);
        assert!(t.js.contains("h(\"div\""), "{}", t.js);
        assert!(!t.js.contains(": P"), "{}", t.js);
        let f = transpile("export default () => <><b>a</b></>;");
        assert!(f.ok(), "{:?}", f.diagnostics);
        assert!(f.js.contains("Fragment"), "{}", f.js);
    }

    #[test]
    fn reports_imports_globals_and_missing_export() {
        let t = transpile("import x from 'fs';\nexport default function C() { return fetch('http://a'); }");
        let codes: Vec<_> = t.diagnostics.iter().map(|d| d.code.as_str()).collect();
        assert!(codes.contains(&"import"), "{codes:?}");
        assert!(codes.contains(&"global"), "{codes:?}");
        let g = t.diagnostics.iter().find(|d| d.code == "global").unwrap();
        assert_eq!((g.line, g.column), (Some(2), Some(38)));
        assert!(transpile("function C() {}").diagnostics.iter().any(|d| d.code == "export"));
        assert!(
            transpile("export default function C() { return import('x'); }").diagnostics.iter().any(|d| d.code == "import")
        );
        assert!(transpile("export default function C() { setTimeout(() => 1); }").diagnostics[0].message.contains("timers"));
        let s = transpile("export default function C( { return 1 }");
        assert_eq!(s.diagnostics[0].code, "syntax");
    }
}
