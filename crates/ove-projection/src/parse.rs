//! The parser (design D1): oxc in TSX mode, walked into a small element tree of literal values. Nothing is ever
//! executed. All oxc usage lives in this module so a version bump touches one file.

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    ArrayExpressionElement, Expression, JSXAttributeItem, JSXAttributeName, JSXAttributeValue, JSXChild, JSXElement,
    JSXElementName, JSXExpression, ObjectPropertyKind, PropertyKey, Statement, UnaryOperator,
};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType, Span};
use serde::Serialize;
use serde_json::{Map, Number, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// A problem in a projection, with 1-based line and column (columns count characters).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub line: usize,
    pub column: usize,
    pub code: String,
    pub message: String,
}

impl Diagnostic {
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = if self.is_error() { "error" } else { "warning" };
        write!(f, "{s} {}:{} {}: {}", self.line, self.column, self.code, self.message)
    }
}

/// Byte offset → (line, column) map.
#[derive(Debug, Clone)]
pub struct LineIndex {
    starts: Vec<usize>,
    text: String,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        LineIndex { starts, text: text.to_string() }
    }

    pub fn pos(&self, offset: usize) -> (usize, usize) {
        let line = self.starts.partition_point(|s| *s <= offset).max(1);
        let start = self.starts[line - 1];
        let col = self.text.get(start..offset.min(self.text.len())).map(|s| s.chars().count()).unwrap_or(0) + 1;
        (line, col)
    }
}

/// A position in the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pos {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attr {
    pub name: String,
    pub value: Value,
    pub pos: Pos,
    /// Position of the value (for diagnostics about the value).
    pub value_pos: Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub tag: String,
    pub attrs: Vec<Attr>,
    /// Attributes present but rejected (already reported), so readers do not report them as missing.
    pub invalid: Vec<String>,
    pub children: Vec<Element>,
    pub pos: Pos,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&Attr> {
        self.attrs.iter().find(|a| a.name == name)
    }
}

/// Result of parsing: the root element (if the structure was readable; after a recovered syntax error it lacks the
/// broken elements), the header `rev`, and diagnostics.
#[derive(Debug, Clone)]
pub struct Parsed {
    pub root: Option<Element>,
    /// `None`: no `rev:` line; `Some(Err)`: malformed.
    pub rev: Option<Result<i64, String>>,
    pub diagnostics: Vec<Diagnostic>,
}

struct Walker<'t> {
    idx: LineIndex,
    text: &'t str,
    diags: Vec<Diagnostic>,
}

impl Walker<'_> {
    fn pos(&self, span: Span) -> Pos {
        let (line, column) = self.idx.pos(span.start as usize);
        Pos { line, column }
    }

    fn err(&mut self, span: Span, code: &str, message: impl Into<String>) {
        let p = self.pos(span);
        self.diags.push(Diagnostic {
            severity: Severity::Error,
            line: p.line,
            column: p.column,
            code: code.into(),
            message: message.into(),
        });
    }

    fn literal_error(&mut self, span: Span) {
        let snippet: String =
            self.text.get(span.start as usize..span.end as usize).unwrap_or("").chars().take(40).collect();
        self.err(
            span,
            "literal.only",
            format!(
                "only literal values are allowed (strings, numbers, booleans, null, arrays, objects); found `{snippet}`"
            ),
        );
    }

    fn expr(&mut self, e: &Expression) -> Option<Value> {
        match e {
            Expression::ParenthesizedExpression(p) => self.expr(&p.expression),
            Expression::NumericLiteral(n) => num(n.value).or_else(|| {
                self.err(n.span, "value.finite", "numbers must be finite");
                None
            }),
            Expression::StringLiteral(s) => Some(Value::String(s.value.display().to_string())),
            Expression::BooleanLiteral(b) => Some(Value::Bool(b.value)),
            Expression::NullLiteral(_) => Some(Value::Null),
            Expression::TemplateLiteral(t) if t.expressions.is_empty() => {
                let s: String =
                    t.quasis.iter().filter_map(|q| q.value.cooked.as_ref().map(|c| c.display().to_string())).collect();
                Some(Value::String(s))
            }
            Expression::UnaryExpression(u) if u.operator == UnaryOperator::UnaryNegation => match &u.argument {
                Expression::NumericLiteral(n) => num(-n.value),
                _ => {
                    self.literal_error(u.span);
                    None
                }
            },
            Expression::UnaryExpression(u) if u.operator == UnaryOperator::UnaryPlus => match &u.argument {
                Expression::NumericLiteral(n) => num(n.value),
                _ => {
                    self.literal_error(u.span);
                    None
                }
            },
            Expression::ArrayExpression(a) => {
                let mut out = Vec::new();
                let mut ok = true;
                for el in &a.elements {
                    match el {
                        ArrayExpressionElement::SpreadElement(s) => {
                            self.literal_error(s.span);
                            ok = false;
                        }
                        ArrayExpressionElement::Elision(s) => {
                            self.literal_error(s.span);
                            ok = false;
                        }
                        other => match other.as_expression().and_then(|e| self.expr(e)) {
                            Some(v) => out.push(v),
                            None => ok = false,
                        },
                    }
                }
                ok.then_some(Value::Array(out))
            }
            Expression::ObjectExpression(o) => {
                let mut m = Map::new();
                let mut ok = true;
                for p in &o.properties {
                    match p {
                        ObjectPropertyKind::SpreadProperty(s) => {
                            self.literal_error(s.span);
                            ok = false;
                        }
                        ObjectPropertyKind::ObjectProperty(p) => {
                            if p.computed || p.method || p.shorthand {
                                self.literal_error(p.span);
                                ok = false;
                                continue;
                            }
                            let key = match &p.key {
                                PropertyKey::StaticIdentifier(id) => Some(id.name.to_string()),
                                PropertyKey::StringLiteral(s) => Some(s.value.display().to_string()),
                                PropertyKey::NumericLiteral(n) => {
                                    Some(crate::format::number(&Number::from_f64(n.value).unwrap_or(Number::from(0))))
                                }
                                _ => None,
                            };
                            let Some(key) = key else {
                                self.literal_error(p.span);
                                ok = false;
                                continue;
                            };
                            match self.expr(&p.value) {
                                Some(v) => {
                                    if m.insert(key.clone(), v).is_some() {
                                        self.err(p.span, "attr.duplicate", format!("key `{key}` appears twice"));
                                        ok = false;
                                    }
                                }
                                None => ok = false,
                            }
                        }
                    }
                }
                ok.then_some(Value::Object(m))
            }
            other => {
                self.literal_error(other.span());
                None
            }
        }
    }

    fn element(&mut self, el: &JSXElement) -> Option<Element> {
        let open = &el.opening_element;
        let tag = match &open.name {
            JSXElementName::Identifier(id) => id.name.to_string(),
            JSXElementName::IdentifierReference(id) => id.name.to_string(),
            other => {
                self.err(other.span(), "structure.element", "element names must be plain words such as <clip>");
                return None;
            }
        };
        let pos = self.pos(open.span);
        let mut attrs: Vec<Attr> = Vec::new();
        let mut invalid: Vec<String> = Vec::new();
        for a in &open.attributes {
            match a {
                JSXAttributeItem::SpreadAttribute(s) => self.literal_error(s.span),
                JSXAttributeItem::Attribute(at) => {
                    let name = match &at.name {
                        JSXAttributeName::Identifier(id) => id.name.to_string(),
                        JSXAttributeName::NamespacedName(n) => {
                            self.err(n.span, "structure.attribute", "namespaced attributes are not supported");
                            continue;
                        }
                    };
                    let apos = self.pos(at.span);
                    let (value, vspan) = match &at.value {
                        None => (Some(Value::Bool(true)), at.span),
                        Some(JSXAttributeValue::StringLiteral(s)) => {
                            // JSX strings have no escapes: take the source text between the quotes literally.
                            let raw = self.text.get(s.span.start as usize + 1..s.span.end as usize - 1).unwrap_or("");
                            (Some(Value::String(raw.to_string())), s.span)
                        }
                        Some(JSXAttributeValue::ExpressionContainer(c)) => match &c.expression {
                            JSXExpression::EmptyExpression(e) => {
                                self.err(e.span, "literal.only", format!("attribute `{name}` has an empty value"));
                                (None, c.span)
                            }
                            other => (other.as_expression().and_then(|e| self.expr(e)), c.span),
                        },
                        Some(other) => {
                            self.literal_error(other.span());
                            (None, at.span)
                        }
                    };
                    if attrs.iter().any(|x| x.name == name) {
                        self.err(at.span, "attr.duplicate", format!("attribute `{name}` appears twice"));
                        continue;
                    }
                    match value {
                        Some(value) => attrs.push(Attr { name, value, pos: apos, value_pos: self.pos(vspan) }),
                        None => invalid.push(name),
                    }
                }
            }
        }
        let mut children = Vec::new();
        for c in &el.children {
            match c {
                JSXChild::Text(t) => {
                    if !t.value.trim().is_empty() {
                        self.err(
                            t.span,
                            "structure.text",
                            "text is not allowed between elements; use attributes such as textContent",
                        );
                    }
                }
                JSXChild::Element(e) => {
                    if let Some(e) = self.element(e) {
                        children.push(e);
                    }
                }
                JSXChild::ExpressionContainer(c) => {
                    if !matches!(c.expression, JSXExpression::EmptyExpression(_)) {
                        self.literal_error(c.span);
                    }
                }
                JSXChild::Fragment(f) => self.err(f.span, "structure.element", "fragments are not allowed"),
                JSXChild::Spread(s) => self.literal_error(s.span),
            }
        }
        Some(Element { tag, attrs, invalid, children, pos })
    }
}

fn num(f: f64) -> Option<Value> {
    if !f.is_finite() {
        return None;
    }
    if f == f.trunc() && f.abs() < 9.0e15 {
        Some(Value::Number(Number::from(f as i64)))
    } else {
        Number::from_f64(f).map(Value::Number)
    }
}

/// Finds the `rev:` line in the leading header comment.
pub fn header_rev(text: &str) -> Option<Result<i64, String>> {
    let t = text.trim_start();
    if !t.starts_with("/*") {
        return None;
    }
    let end = t.find("*/")?;
    for line in t[..end].lines() {
        let l = line.trim().trim_start_matches('*').trim();
        if let Some(v) = l.strip_prefix("rev:") {
            let v = v.trim();
            return Some(v.parse::<i64>().map_err(|_| format!("`rev: {v}` is not a revision number")));
        }
    }
    None
}

/// Parses projection text into an element tree with diagnostics. Never executes anything.
pub fn parse(text: &str) -> Parsed {
    let mut source = text.to_string();
    let mut syntax: Vec<Diagnostic> = Vec::new();
    let idx = LineIndex::new(text);
    // Error recovery: a syntax error inside an item element blanks that element (offsets preserved) and the file
    // is parsed again, so later problems are still reported. The result is for diagnostics only.
    for _ in 0..50 {
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, &source, SourceType::tsx()).parse();
        let errors: Vec<_> = ret.diagnostics.errors().collect();
        if errors.is_empty() {
            let mut w = Walker { idx: idx.clone(), text, diags: Vec::new() };
            let root = w_root(&mut w, &ret.program.body);
            let mut diags = syntax;
            diags.extend(w.diags);
            diags.sort_by_key(|d| (d.line, d.column));
            diags.dedup();
            // After recovery the tree lacks the blanked elements; it is still read for further diagnostics, and
            // the syntax errors keep any apply refused.
            return Parsed { root, rev: header_rev(text), diagnostics: diags };
        }
        let first = errors[0].labels.first().map(|l| l.offset() as usize).unwrap_or(0);
        for e in &errors {
            let offset = e.labels.first().map(|l| l.offset() as usize).unwrap_or(0);
            let (line, column) = idx.pos(offset);
            let d = Diagnostic {
                severity: Severity::Error,
                line,
                column,
                code: "syntax".into(),
                message: e.message.to_string(),
            };
            if !syntax.contains(&d) {
                syntax.push(d);
            }
        }
        match item_element_around(&source, first) {
            Some((a, b)) => blank(&mut source, a, b),
            None => break,
        }
    }
    syntax.sort_by_key(|d| (d.line, d.column));
    Parsed { root: None, rev: header_rev(text), diagnostics: syntax }
}

fn w_root(w: &mut Walker, body: &[Statement]) -> Option<Element> {
    let mut root = None;
    let mut found = false;
    for st in body {
        match st {
            Statement::ExportDefaultDeclaration(d) if !found => {
                found = true;
                let mut e = d.declaration.as_expression();
                while let Some(Expression::ParenthesizedExpression(p)) = e {
                    e = Some(&p.expression);
                }
                match e {
                    Some(Expression::JSXElement(el)) => root = w.element(el),
                    _ => w.err(d.span, "structure.root", "`export default` must be a <timeline> element"),
                }
            }
            other => {
                w.err(other.span(), "structure.root", "only `export default (<timeline> … </timeline>);` is allowed")
            }
        }
    }
    if !found {
        w.err(Span::new(0, 0), "structure.root", "missing `export default (<timeline> … </timeline>);`");
    }
    root
}

/// Byte range of the self-closing item element (`<tag … />`) around `offset`, if any.
fn item_element_around(text: &str, offset: usize) -> Option<(usize, usize)> {
    let offset = offset.min(text.len());
    let before = &text[..offset];
    let open = before.rfind('<')?;
    let tag: String = text[open + 1..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    if !matches!(tag.as_str(), "clip" | "image" | "audio" | "text" | "caption" | "code" | "adjustment" | "shader") {
        return None;
    }
    let close = text[offset..].find("/>").map(|i| offset + i + 2)?;
    // The element must not contain another element start between the error and its end.
    if text[open + 1..close].contains("\n    <track") {
        return None;
    }
    Some((open, close))
}

fn blank(s: &mut String, a: usize, b: usize) {
    let replaced: String = s[a..b].chars().map(|c| if c == '\n' { '\n' } else { ' ' }).collect();
    // Same byte length is required to keep offsets: replace multi-byte chars by as many spaces as bytes.
    let replaced = if replaced.len() == b - a {
        replaced
    } else {
        s[a..b].bytes().map(|c| if c == b'\n' { '\n' } else { ' ' }).collect()
    };
    s.replace_range(a..b, &replaced);
}
